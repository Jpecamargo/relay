import { invokeCommand } from "@relay/protocol-types";
import { emitTo } from "@tauri-apps/api/event";

/** Ids das seções da janela principal. */
export type Section = "overview" | "screens" | "shortcuts" | "audio" | "clipboard" | "connection" | "general";

/** Abre a janela principal (traz o app ao Dock) já na seção pedida. */
export async function openMain(section?: Section): Promise<void> {
  await invokeCommand("open_window", { label: "main" });
  if (section) await emitTo("main", "relay:navigate", section);
}
