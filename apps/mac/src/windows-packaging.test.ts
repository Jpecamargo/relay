// Onda 2 (I8, I13): contrato do app Windows visto de fora — config do Tauri, front único e a lista
// de comandos registrados. Lê só arquivos de configuração e a lista de `generate_handler!`.
import { expect, test } from "vitest";
import confRaw from "../../windows-agent/src-tauri/tauri.conf.json?raw";
import libRs from "../../windows-agent/src-tauri/src/lib.rs?raw";
import bridgeTs from "../../../packages/protocol-types/src/bridge.ts?raw";

const conf = JSON.parse(confRaw);

test("I8: o identifier da 0.2 continua e o front servido é o build de apps/mac", () => {
  expect(conf.identifier).toBe("com.joaocamargo.relay.agent");
  expect(conf.build.frontendDist).toBe("../../mac/dist");
  expect(conf.build.beforeBuildCommand).toMatch(/@relay\/mac\b.*build|build.*@relay\/mac/);
});

test("I8: janelas 'panel' e 'main' tituladas 'Relay'; o painel é sem borda e fora da barra de tarefas", () => {
  const byLabel = Object.fromEntries(conf.app.windows.map((w: { label: string }) => [w.label, w]));
  expect(Object.keys(byLabel).sort()).toEqual(["main", "panel"]);
  expect(byLabel.panel.title).toBe("Relay");
  expect(byLabel.main.title).toBe("Relay");
  expect(byLabel.panel.decorations).toBe(false);
  expect(byLabel.panel.skipTaskbar).toBe(true);
});

test("I8: serviço SYSTEM e VB-CABLE continuam empacotados no instalador NSIS", () => {
  expect(conf.bundle.targets).toContain("nsis");
  expect(conf.bundle.externalBin).toContain("binaries/relay-service");
  expect(Object.keys(conf.bundle.resources)).toContain("vbcable/");
});

test("I8: o front próprio do agente e o AgentCommandMap foram removidos", () => {
  expect(Object.keys(import.meta.glob("../../windows-agent/src/**/*"))).toEqual([]);
  expect(bridgeTs).not.toContain("AgentCommandMap");
});

const I13 = [
  "get_status", "get_config", "save_config", "list_discovered", "set_manual_address", "send_command",
  "set_cursor_lock", "set_sharing_paused", "jump_to", "clipboard_send_now", "set_clipboard_sync",
  "receive_remote_files", "wake_peer", "reconnect", "list_audio_devices", "set_audio_streams",
  "set_audio_devices", "check_permissions", "open_permission_settings", "set_launch_at_login",
  "open_window", "quit_app", "get_service_status", "set_update_available",
];

test("I13: o handler do Windows registra todos os comandos do contrato único", () => {
  const block = libRs.slice(libRs.indexOf("generate_handler!["));
  const handler = block.slice(0, block.indexOf("]"));
  const registered = [...handler.matchAll(/(?:\w+::)+(\w+)/g)].map((m) => m[1]);
  for (const name of I13) expect(registered, name).toContain(name);
  expect(new Set(registered).size, "sem duplicatas").toBe(registered.length);
});

test("I13: cada comando do contrato aparece no bridge compartilhado com o mesmo nome", () => {
  for (const name of I13) expect(bridgeTs, name).toContain(name);
});
