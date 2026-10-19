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
  /** `false` até o primeiro status chegar: antes disso `peer_name` ainda não diz se há PC pareado. */
  loaded: boolean;
} {
  const [initial, setInitial] = useState<StatusSnapshot>(EMPTY_STATUS);
  const [loaded, setLoaded] = useState(false);
  const [history, setHistory] = useState<number[]>([]);
  const statusEvent = useRelayEvent("relay:status");
  const latency = useRelayEvent("relay:latency", (l) => setHistory((h) => [...h.slice(-(HISTORY - 1)), l.rtt_ms]));
  const refresh = useCallback(() => {
    invokeCommand("get_status").then((s) => { setInitial(s); setLoaded(true); }).catch(() => undefined);
  }, []);
  useEffect(refresh, [refresh]);
  return { status: statusEvent ?? initial, latency, history, refresh, loaded: loaded || statusEvent !== undefined };
}
