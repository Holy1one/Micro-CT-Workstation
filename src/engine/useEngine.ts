/**
 * React bridge to the selected EngineAdapter.
 * It owns only the latest immutable snapshot and request lifecycle; it never
 * predicts command success or advances production scan state locally.
 */

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
  const refreshActiveRef = useRef(false);
  const disposedRef = useRef(false);
  const [adapterKind, setAdapterKind] = useState<AdapterKind>("developer_preview");
  const [snapshot, setSnapshot] = useState<EngineSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [transportError, setTransportError] = useState<string | null>(null);

  const commitSnapshot = useCallback((revision: number, next: EngineSnapshot): void => {
    if (!disposedRef.current && revision === requestRevisionRef.current) {
      setSnapshot(next);
      setTransportError(null);
    }
  }, []);

  useEffect(() => {
    const adapter = createEngineAdapter();
    adapterRef.current = adapter;
    disposedRef.current = false;
    setAdapterKind(adapter.kind);

    const refresh = async (): Promise<void> => {
      if (disposedRef.current || commandActiveRef.current || refreshActiveRef.current) return;
      refreshActiveRef.current = true;
      const revision = ++requestRevisionRef.current;
      try {
        const next = await adapter.getSnapshot();
        commitSnapshot(revision, next);
      } catch (reason) {
        if (!disposedRef.current && revision === requestRevisionRef.current) {
          setError(errorMessage(reason));
          setTransportError(errorMessage(reason));
        }
      } finally {
        refreshActiveRef.current = false;
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
    if (!adapter || commandActiveRef.current || refreshActiveRef.current) return;
    refreshActiveRef.current = true;
    const revision = ++requestRevisionRef.current;
    try {
      commitSnapshot(revision, await adapter.getSnapshot());
    } catch (reason) {
      if (revision === requestRevisionRef.current) {
        setError(errorMessage(reason));
        setTransportError(errorMessage(reason));
      }
    } finally {
      refreshActiveRef.current = false;
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
          setError(null);
        } catch (reason) {
          if (revision === requestRevisionRef.current) {
            const message = errorMessage(reason);
            setError(message);
            if (!message.startsWith("ENGINE:")) setTransportError(message);
          }
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

  return { adapterKind, snapshot, busy, error, transportError, refresh, dispatch };
}
