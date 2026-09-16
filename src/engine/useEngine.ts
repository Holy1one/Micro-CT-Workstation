import { useCallback, useEffect, useRef, useState } from "react";
import { createEngineAdapter } from "./adapter";
import type { EngineCommand, EngineSnapshot } from "./types";

export function useEngine() {
  const adapterRef = useRef<ReturnType<typeof createEngineAdapter> | null>(null);
  const [snapshot, setSnapshot] = useState<EngineSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const adapter = createEngineAdapter();
    adapterRef.current = adapter;
    let disposed = false;

    const refresh = async () => {
      try {
        const next = await adapter.getSnapshot();
        if (!disposed) setSnapshot(next);
      } catch (reason) {
        if (!disposed) setError(reason instanceof Error ? reason.message : String(reason));
      }
    };

    void refresh();
    const interval = window.setInterval(() => void refresh(), 850);
    return () => {
      disposed = true;
      window.clearInterval(interval);
      adapterRef.current = null;
    };
  }, []);

  const refresh = useCallback(async () => {
    const adapter = adapterRef.current;
    if (!adapter) return;
    setError(null);
    try {
      setSnapshot(await adapter.getSnapshot());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }, []);

  const dispatch = useCallback(async (command: EngineCommand) => {
    const adapter = adapterRef.current;
    if (!adapter) return;
    setBusy(true);
    setError(null);
    try {
      setSnapshot(await adapter.dispatch(command));
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      setError(message);
    } finally {
      setBusy(false);
    }
  }, []);

  return {
    adapterKind: adapterRef.current?.kind ?? "developer_preview",
    snapshot,
    busy,
    error,
    refresh,
    dispatch,
  };
}
