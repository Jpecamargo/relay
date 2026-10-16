import { listenEvent, type EventMap, type EventName } from "@relay/protocol-types";
import { useEffect, useRef, useState } from "react";

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
