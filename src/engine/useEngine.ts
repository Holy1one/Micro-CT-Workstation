import { useCallback, useEffect, useRef, useState } from "react";
import { createEngineAdapter } from "./adapter";
import type { AdapterKind, EngineAdapter, EngineCommand, EngineSnapshot } from "./types";

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

export function useEngine() {
  const adapterRef = useRef<EngineAdapter | null>(null);
  const requestRevisionRef = useRef(0);
  const commandQueueRef = useRef<Promise<void>>(Promise.resolve());
  const commandActiveRef = useRef(false);
  const hardwareConnectAttemptedRef = useRef(false);
  const disposedRef = useRef(false);
  const [adapterKind, setAdapterKind] = useState<AdapterKind>("developer_preview");
  const [snapshot, setSnapshot] = useState<EngineSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const commitSnapshot = useCallback((revision: number, next: EngineSnapshot): void => {
    if (!disposedRef.current && revision === requestRevisionRef.current) {
      setSnapshot(next);
      setError(null);
    }
  }, []);

  useEffect(() => {
    const adapter = createEngineAdapter();
    adapterRef.current = adapter;
    disposedRef.current = false;
    setAdapterKind(adapter.kind);

    const refresh = async (): Promise<void> => {
      if (disposedRef.current || commandActiveRef.current) return;
      const revision = ++requestRevisionRef.current;
      try {
        let next = await adapter.getSnapshot();
        let connectError: string | null = null;
        if (
          adapter.kind === "tauri" &&
          next.mode === "production_locked" &&
          next.connectionState === "disconnected" &&
          !hardwareConnectAttemptedRef.current
        ) {
          hardwareConnectAttemptedRef.current = true;
          setBusy(true);
          try {
            next = await adapter.dispatch({ type: "connect", adapter: "real_hardware" });
          } catch (reason) {
            connectError = errorMessage(reason);
          } finally {
            setBusy(false);
          }
        }
        commitSnapshot(revision, next);
        if (connectError && !disposedRef.current && revision === requestRevisionRef.current) {
          setError(connectError);
        }
      } catch (reason) {
        if (!disposedRef.current && revision === requestRevisionRef.current) {
          setError(errorMessage(reason));
        }
      }
    };

    void refresh();
    const interval = window.setInterval(() => void refresh(), 850);
    return () => {
      disposedRef.current = true;
      requestRevisionRef.current++;
      window.clearInterval(interval);
      adapter.close?.();
      adapterRef.current = null;
    };
  }, [commitSnapshot]);

  const refresh = useCallback(async (): Promise<void> => {
    const adapter = adapterRef.current;
    if (!adapter || commandActiveRef.current) return;
    const revision = ++requestRevisionRef.current;
    try {
      commitSnapshot(revision, await adapter.getSnapshot());
    } catch (reason) {
      if (revision === requestRevisionRef.current) setError(errorMessage(reason));
    }
  }, [commitSnapshot]);

  const dispatch = useCallback(
    async (command: EngineCommand): Promise<void> => {
      const run = async (): Promise<void> => {
        const adapter = adapterRef.current;
        if (!adapter) return;
        commandActiveRef.current = true;
        setBusy(true);
        const revision = ++requestRevisionRef.current;
        try {
          commitSnapshot(revision, await adapter.dispatch(command));
        } catch (reason) {
          if (revision === requestRevisionRef.current) setError(errorMessage(reason));
        } finally {
          commandActiveRef.current = false;
          setBusy(false);
        }
      };
      const queued = commandQueueRef.current.then(run, run);
      commandQueueRef.current = queued.catch(() => undefined);
      await queued;
    },
    [commitSnapshot],
  );

  return { adapterKind, snapshot, busy, error, refresh, dispatch };
}
