// Lógica de pareamento compartilhada entre a tela de primeiro uso e Ajustes › Conexão.
import type { DiscoveredPeer } from "@relay/protocol-types";
import { invokeCommand } from "@relay/protocol-types";
import { useRelayEvent } from "@relay/ui";
import { useCallback, useEffect, useState } from "react";
import { errorMessage } from "../errors";

const WRONG_CODE = "Código incorreto ou expirado. Confira o código no PC e digite de novo.";
const POLL_MS = 3000;

export interface UsePairingOptions {
  /** Endereço manual gravado hoje. */
  manualAddress: string | null;
  /** Avisa que o endereço manual foi aplicado (o backend já persistiu; o Config em tela precisa acompanhar). */
  onManualApplied?: (address: string | null) => void;
  /** Reconsulta os PCs encontrados a cada poucos segundos (tela de primeiro uso). */
  poll?: boolean;
}

export function usePairing({ manualAddress, onManualApplied, poll }: UsePairingOptions) {
  const [peers, setPeers] = useState<DiscoveredPeer[]>([]);
  const [peerName, setPeerName] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [manual, setManual] = useState(manualAddress ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [paired, setPaired] = useState(false);

  const refresh = useCallback(() => {
    invokeCommand("list_discovered").then(setPeers, (e) => setError(errorMessage(e)));
    invokeCommand("get_status").then((s) => setPeerName(s.peer_name), () => undefined);
  }, []);
  useEffect(refresh, [refresh]);
  useEffect(() => {
    if (!poll) return;
    const id = setInterval(refresh, POLL_MS);
    return () => clearInterval(id);
  }, [poll, refresh]);

  // Um único PC na rede já vem selecionado.
  const target = peers.find((p) => p.device_id === selectedId) ?? (peers.length === 1 ? peers[0]! : null);

  const fail = (message: string) => {
    setError(message);
    setCode("");
  };

  useRelayEvent("relay:status", (s) => setPeerName(s.peer_name));
  useRelayEvent("relay:pairing", (p) => {
    if (p.state === "paired") {
      setPaired(true);
      setCode("");
      refresh();
    } else if (p.state === "failed") fail(WRONG_CODE);
  });

  const changeCode = (value: string) => {
    setError("");
    setCode(value);
  };

  const pair = async (digits: string) => {
    setBusy(true);
    setError("");
    try {
      const address = target ? `${target.addrs[0]}:${target.port}` : undefined;
      await invokeCommand("pair_with_code", address ? { code: digits, address } : { code: digits });
      setPaired(true);
      setCode("");
      refresh();
    } catch {
      fail(WRONG_CODE);
    } finally {
      setBusy(false);
    }
  };

  const applyManual = async () => {
    const address = manual.trim() || undefined;
    setError("");
    try {
      await invokeCommand("set_manual_address", address ? { address } : {});
      onManualApplied?.(address ?? null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const unpair = async () => {
    try {
      await invokeCommand("unpair");
      setPaired(false);
      setPeerName(null);
      refresh();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return {
    peers, peerName, target, select: (p: DiscoveredPeer) => setSelectedId(p.device_id),
    code, changeCode, busy, error, paired, pair, refresh,
    manual, setManual, applyManual, unpair,
  };
}
