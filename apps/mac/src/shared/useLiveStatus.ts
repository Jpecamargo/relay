import { invokeCommand, type LatencyStats, type StatusSnapshot } from "@relay/protocol-types";
import { useRelayEvent } from "@relay/ui";
import { useCallback, useEffect, useState } from "react";
import { EMPTY_STATUS } from "./status";

const HISTORY = 60;

/** Estado da conexão e latência ao vivo (status inicial por comando, depois por evento). */
export function useLiveStatus(): {
  status: StatusSnapshot;
  latency: LatencyStats | undefined;
  history: number[];
  refresh: () => void;
} {
  const [initial, setInitial] = useState<StatusSnapshot>(EMPTY_STATUS);
  const [history, setHistory] = useState<number[]>([]);
  const statusEvent = useRelayEvent("relay:status");
  const latency = useRelayEvent("relay:latency", (l) => setHistory((h) => [...h.slice(-(HISTORY - 1)), l.rtt_ms]));
  const refresh = useCallback(() => {
    invokeCommand("get_status").then(setInitial).catch(() => undefined);
  }, []);
  useEffect(refresh, [refresh]);
  return { status: statusEvent ?? initial, latency, history, refresh };
}
