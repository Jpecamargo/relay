import type { ConnectionStatus, StatusSnapshot } from "@relay/protocol-types";
import type { StatusTone } from "@relay/ui";

export const EMPTY_STATUS: StatusSnapshot = {
  status: "Disconnected", peer_name: null, focus: "Mac", locked: false, paused: false,
  secure_input: false, pc_unreachable: false, clipboard_sync: false, audio: { pc_to_mac: false, mac_to_pc: false },
  peer_version: null, version_sync: "InSync",
};

export const STATUS_LABELS: Record<string, string> = {
  Disconnected: "Desconectado", Discovering: "Procurando o PC", Connecting: "Conectando",
  Pairing: "Pareando", Connected: "Conectado", Reconnecting: "Reconectando",
  IncompatibleVersion: "Versão incompatível",
};

export function statusKey(s: ConnectionStatus): string {
  return typeof s === "string" ? s : Object.keys(s)[0] ?? "Disconnected";
}

export function statusTone(key: string): StatusTone {
  if (key === "Connected") return "ok";
  if (key === "Reconnecting" || key === "Connecting" || key === "Pairing") return "warn";
  if (key === "IncompatibleVersion") return "danger";
  return "idle";
}

/** Número com vírgula decimal ("0,8"). */
export const decimal = (n: number, digits = 1) => n.toFixed(digits).replace(".", ",");
