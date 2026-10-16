// Cliente tipado dos comandos e eventos Tauri (D15: tipado à mão, tipos de payload gerados).
//
// Convenções:
// - Os argumentos de cada comando usam os nomes snake_case do glossário. Os handlers Rust
//   precisam de `#[tauri::command(rename_all = "snake_case")]`, senão o Tauri espera camelCase.
// - Comando sem argumentos: `invokeCommand("get_status")`.
// - Só o item F2 edita este arquivo: comando ou evento novo pede ao PO.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AudioDevices,
  AudioStats,
  CommandResult,
  Config,
  DiscoveredPeer,
  DisplaysPayload,
  LatencyStats,
  LayoutProfile,
  NewPairingCode,
  PairingEvent,
  PermissionKind,
  PermissionsStatus,
  RemoteCommand,
  SaveLayoutResult,
  SecureInputPayload,
  ServiceStatus,
  StatusSnapshot,
  SystemMetrics,
  TransferProgress,
  UpdateAvailable,
} from "./generated";

/** Labels das janelas Tauri: `panel`, `settings` e `editor` no Mac; `main` no agente. */
export type WindowLabel = "panel" | "settings" | "editor" | "main";

/** Comandos do app Mac: argumentos e retorno. */
export interface MacCommandMap {
  get_status: { args: void; result: StatusSnapshot };
  list_discovered: { args: void; result: DiscoveredPeer[] };
  pair_with_code: { args: { code: string; address?: string }; result: void };
  unpair: { args: void; result: void };
  set_manual_address: { args: { address?: string }; result: void };
  get_displays: { args: void; result: DisplaysPayload };
  get_layout: { args: void; result: LayoutProfile };
  save_layout: { args: { profile: LayoutProfile }; result: SaveLayoutResult };
  get_config: { args: void; result: Config };
  save_config: { args: { config: Config }; result: void };
  send_command: { args: { command: RemoteCommand }; result: CommandResult };
  set_cursor_lock: { args: { locked: boolean }; result: void };
  set_sharing_paused: { args: { paused: boolean }; result: void };
  clipboard_send_now: { args: void; result: void };
  set_clipboard_sync: { args: { enabled: boolean }; result: void };
  receive_remote_files: { args: void; result: void };
  wake_pc: { args: void; result: void };
  list_audio_devices: { args: void; result: AudioDevices };
  set_audio_streams: { args: { pc_to_mac: boolean; mac_to_pc: boolean }; result: void };
  set_audio_devices: { args: { output_id?: string; input_id?: string }; result: void };
  set_stream_volume: { args: { volume: number }; result: void };
  check_permissions: { args: void; result: PermissionsStatus };
  open_permission_settings: { args: { kind: PermissionKind }; result: void };
  set_launch_at_login: { args: { enabled: boolean }; result: void };
  /** Versão disponível, ou `null` se já está atualizado. */
  check_updates: { args: void; result: string | null };
  install_update: { args: void; result: void };
  open_window: { args: { label: WindowLabel }; result: void };
}

/** Comandos do agente Windows: argumentos e retorno. */
export interface AgentCommandMap {
  agent_get_status: { args: void; result: StatusSnapshot };
  agent_new_pairing_code: { args: void; result: NewPairingCode };
  agent_revoke_pair: { args: void; result: void };
  agent_get_service_status: { args: void; result: ServiceStatus };
  agent_get_config: { args: void; result: Config };
  agent_save_config: { args: { config: Config }; result: void };
  agent_set_autostart: { args: { enabled: boolean }; result: void };
}

export type CommandMap = MacCommandMap & AgentCommandMap;
export type CommandName = keyof CommandMap;
export type CommandArgs<K extends CommandName> = CommandMap[K]["args"];
export type CommandResultOf<K extends CommandName> = CommandMap[K]["result"];

/** Nomes dos comandos (o `satisfies` garante que o objeto cobre exatamente o `CommandMap`). */
export const COMMANDS = {
  get_status: "get_status",
  list_discovered: "list_discovered",
  pair_with_code: "pair_with_code",
  unpair: "unpair",
  set_manual_address: "set_manual_address",
  get_displays: "get_displays",
  get_layout: "get_layout",
  save_layout: "save_layout",
  get_config: "get_config",
  save_config: "save_config",
  send_command: "send_command",
  set_cursor_lock: "set_cursor_lock",
  set_sharing_paused: "set_sharing_paused",
  clipboard_send_now: "clipboard_send_now",
  set_clipboard_sync: "set_clipboard_sync",
  receive_remote_files: "receive_remote_files",
  wake_pc: "wake_pc",
  list_audio_devices: "list_audio_devices",
  set_audio_streams: "set_audio_streams",
  set_audio_devices: "set_audio_devices",
  set_stream_volume: "set_stream_volume",
  check_permissions: "check_permissions",
  open_permission_settings: "open_permission_settings",
  set_launch_at_login: "set_launch_at_login",
  check_updates: "check_updates",
  install_update: "install_update",
  open_window: "open_window",
  agent_get_status: "agent_get_status",
  agent_new_pairing_code: "agent_new_pairing_code",
  agent_revoke_pair: "agent_revoke_pair",
  agent_get_service_status: "agent_get_service_status",
  agent_get_config: "agent_get_config",
  agent_save_config: "agent_save_config",
  agent_set_autostart: "agent_set_autostart",
} as const satisfies { [K in CommandName]: K };

/** Payload de cada evento core→UI. */
export interface EventMap {
  "relay:status": StatusSnapshot;
  /** No máximo 2 Hz. */
  "relay:latency": LatencyStats;
  "relay:displays": DisplaysPayload;
  "relay:command-result": CommandResult;
  /** No máximo 10 Hz. */
  "relay:transfer-progress": TransferProgress;
  "relay:secure-input": SecureInputPayload;
  "relay:permissions": PermissionsStatus;
  "relay:metrics": SystemMetrics;
  "relay:audio-stats": AudioStats;
  "relay:pairing": PairingEvent;
  "relay:update-available": UpdateAvailable;
}

export type EventName = keyof EventMap;

export const EVENTS = {
  status: "relay:status",
  latency: "relay:latency",
  displays: "relay:displays",
  commandResult: "relay:command-result",
  transferProgress: "relay:transfer-progress",
  secureInput: "relay:secure-input",
  permissions: "relay:permissions",
  metrics: "relay:metrics",
  audioStats: "relay:audio-stats",
  pairing: "relay:pairing",
  updateAvailable: "relay:update-available",
} as const satisfies Record<string, EventName>;

type ArgsTuple<K extends CommandName> = CommandArgs<K> extends void ? [] : [args: CommandArgs<K>];

/** Chama um comando Tauri com argumentos e retorno tipados. */
export function invokeCommand<K extends CommandName>(
  name: K,
  ...args: ArgsTuple<K>
): Promise<CommandResultOf<K>> {
  return invoke<CommandResultOf<K>>(name, args[0] as Record<string, unknown> | undefined);
}

/** Escuta um evento core→UI. Devolve a função que cancela a escuta. */
export function listenEvent<K extends EventName>(
  name: K,
  callback: (payload: EventMap[K]) => void,
): Promise<UnlistenFn> {
  return listen<EventMap[K]>(name, (event) => callback(event.payload));
}
