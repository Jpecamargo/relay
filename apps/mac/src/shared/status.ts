import type { ConnectionStatus, StatusSnapshot } from "@relay/protocol-types";
import type { StatusTone } from "@relay/ui";

export const EMPTY_STATUS: StatusSnapshot = {
  status: "Disconnected", peer_name: null, focus: "Local", locked: false, paused: false,
  secure_input: false, peer_unreachable: false, clipboard_sync: false, audio: { mic_to_peer: false, mic_from_peer: false },
  peer_version: null, version_sync: "InSync",
  local_os: "Mac", peer_os: null, local_caps: { os: "Mac", features: 544 }, peer_caps: null, layout_host: "Local", layout_editable: true,
};

export const STATUS_LABELS: Record<string, string> = {
  Disconnected: "Desconectado", Discovering: "Procurando o outro computador", Connecting: "Conectando",
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
