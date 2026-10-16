import {
  invokeCommand,
  listenEvent,
  type CommandArgs,
  type CommandName,
  type CommandResultOf,
  type EventMap,
  type EventName,
} from "@relay/protocol-types";
import { useCallback, useEffect, useRef, useState } from "react";

/** Último payload do evento (`undefined` até o primeiro). `onEvent` recebe cada payload. */
export function useRelayEvent<K extends EventName>(
  name: K,
  onEvent?: (payload: EventMap[K]) => void,
): EventMap[K] | undefined {
  const [payload, setPayload] = useState<EventMap[K]>();
  const handler = useRef(onEvent);
  handler.current = onEvent;

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listenEvent(name, (p) => {
      setPayload(p);
      handler.current?.(p);
    })
      .then((u) => (cancelled ? u() : (unlisten = u)))
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [name]);

  return payload;
}

export interface RelayCommand<K extends CommandName> {
  run: (...args: CommandArgs<K> extends void ? [] : [args: CommandArgs<K>]) => Promise<CommandResultOf<K>>;
  pending: boolean;
  error: string | undefined;
  data: CommandResultOf<K> | undefined;
}

/** Envolve `invokeCommand`: `run` dispara o comando e `pending`/`error`/`data` refletem a última chamada. */
export function useRelayCommand<K extends CommandName>(name: K): RelayCommand<K> {
  const [state, setState] = useState<Pick<RelayCommand<K>, "pending" | "error" | "data">>({
    pending: false,
    error: undefined,
    data: undefined,
  });
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const run = useCallback(
    async (...args: unknown[]) => {
      setState((s) => ({ ...s, pending: true, error: undefined }));
      try {
        const data = await (invokeCommand as (n: K, ...a: unknown[]) => Promise<CommandResultOf<K>>)(name, ...args);
        if (mounted.current) setState({ pending: false, error: undefined, data });
        return data;
      } catch (e) {
        if (mounted.current) setState((s) => ({ ...s, pending: false, error: errorMessage(e) }));
        throw e;
      }
    },
    [name],
  ) as RelayCommand<K>["run"];

  return { run, ...state };
}

function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
