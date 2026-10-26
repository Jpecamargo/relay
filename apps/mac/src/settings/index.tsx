// dono: U3 (redesenho: ui-redesign). Preferências como seções da janela principal: o estado do
// Config é um só (provider) e o "Salvar" envia o Config completo, qualquer que seja a seção.
import type { Config } from "@relay/protocol-types";
import { invokeCommand } from "@relay/protocol-types";
import { Banner, Button } from "@relay/ui";
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { errorMessage } from "../shared/errors";
import { AudioTab, ClipboardTab, TransitionTab } from "./BehaviorTabs";
import { ComputerTab } from "./ComputerTab";
import { ConnectionTab } from "./ConnectionTab";
import { GeneralTab } from "./GeneralTab";
import { HotkeysTab } from "./HotkeysTab";
import { KeyboardTab } from "./KeyboardTab";
import { LayoutTab } from "./LayoutTab";
import { validateConfig } from "./model";
import styles from "./settings.module.css";

export type SettingsView = "connection" | "computer" | "layout" | "keyboard" | "hotkeys" | "transition" | "clipboard" | "audio" | "general";

const VIEWS = {
  connection: ConnectionTab,
  computer: ComputerTab,
  layout: LayoutTab,
  keyboard: KeyboardTab,
  hotkeys: HotkeysTab,
  transition: TransitionTab,
  clipboard: ClipboardTab,
  audio: AudioTab,
  general: GeneralTab,
} as const;

interface SettingsState {
  config: Config | null;
  errors: Record<string, string>;
  update: (fn: (c: Config) => Config) => void;
  dirty: boolean;
  saving: boolean;
  saved: boolean;
  saveError: string;
  showErrors: boolean;
  save: () => Promise<void>;
}

const Ctx = createContext<SettingsState | null>(null);

export function useSettings(): SettingsState {
  const s = useContext(Ctx);
  if (!s) throw new Error("SettingsProvider ausente");
  return s;
}

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [config, setConfig] = useState<Config | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState("");
  const [saved, setSaved] = useState(false);
  const [showErrors, setShowErrors] = useState(false);

  useEffect(() => {
    invokeCommand("get_config").then(setConfig, (e) => setSaveError(errorMessage(e)));
  }, []);

  const errors = useMemo(() => (config ? validateConfig(config) : {}), [config]);
  const update = (fn: (c: Config) => Config) => {
    setConfig((c) => (c ? fn(c) : c));
    setDirty(true);
    setSaved(false);
  };

  const save = async () => {
    if (!config) return;
    setShowErrors(true);
    if (Object.keys(errors).length > 0) return;
    setSaving(true);
    setSaveError("");
    try {
      await invokeCommand("save_config", { config });
      setDirty(false);
      setSaved(true);
    } catch (e) {
      setSaveError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Ctx.Provider value={{ config, errors, update, dirty, saving, saved, saveError, showErrors, save }}>
      {children}
    </Ctx.Provider>
  );
}

/** Mostra as visões pedidas, empilhadas, sobre o Config compartilhado. */
export function SettingsPane({ views }: { views: ReadonlyArray<SettingsView> }) {
  const { config, errors, update, saveError, showErrors } = useSettings();
  if (!config) {
    return saveError ? <Banner tone="danger">{saveError}</Banner> : <p className={styles.muted}>Carregando…</p>;
  }
  return (
    <>
      {views.map((v) => {
        const View = VIEWS[v];
        return <View key={v} config={config} update={update} errors={showErrors || v === "hotkeys" ? errors : {}} />;
      })}
    </>
  );
}

/** Barra de salvar: só aparece quando há o que salvar, avisar ou corrigir. */
export function SaveBar() {
  const { errors, dirty, saving, saved, saveError, showErrors, save } = useSettings();
  const count = Object.keys(errors).length;
  if (!dirty && !saveError && !saved) return null;
  return (
    <footer className={styles.saveBar}>
      <span role="status" className={styles.saveMsg}>
        {saveError ? <span className={styles.error}>{saveError}</span> : showErrors && count > 0 ? <span className={styles.error}>Corrija {count} campo(s) antes de salvar.</span> : saved && !dirty ? "Configurações salvas." : "Alterações não salvas."}
      </span>
      <Button variant="primary" disabled={!dirty || saving} onClick={save}>
        Salvar
      </Button>
    </footer>
  );
}
