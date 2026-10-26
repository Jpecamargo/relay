import { invokeCommand, type StatusSnapshot } from "@relay/protocol-types";
import { useRelayEvent } from "@relay/ui";
import { useEffect, useState } from "react";

/** Status atual para quem só precisa de um recorte dele (aba de Ajustes): comando ao montar, depois evento. */
export function useStatus(): StatusSnapshot | null {
  const [initial, setInitial] = useState<StatusSnapshot | null>(null);
  const live = useRelayEvent("relay:status");
  useEffect(() => {
    invokeCommand("get_status").then(setInitial, () => undefined);
  }, []);
  return live ?? initial;
}
