// dono: U3. Janela de preferências do Mac: abas + rodapé "Salvar" que envia o Config completo.
import type { Config } from "@relay/protocol-types";
import { invokeCommand } from "@relay/protocol-types";
import { Banner, Button, Tabs } from "@relay/ui";
import { useEffect, useMemo, useState } from "react";
import { AudioTab, ClipboardTab, TransitionTab } from "./BehaviorTabs";
import { ConnectionTab } from "./ConnectionTab";
import { GeneralTab } from "./GeneralTab";
import { HotkeysTab } from "./HotkeysTab";
import { KeyboardTab } from "./KeyboardTab";
import { validateConfig } from "./model";
import styles from "./settings.module.css";

const TABS = [
  { id: "connection", label: "Conexão" },
  { id: "keyboard", label: "Teclado" },
  { id: "hotkeys", label: "Atalhos" },
  { id: "transition", label: "Transição" },
  { id: "clipboard", label: "Clipboard" },
  { id: "audio", label: "Áudio" },
  { id: "general", label: "Geral" },
] as const;
type TabId = (typeof TABS)[number]["id"];

const VIEWS = {
  connection: ConnectionTab,
  keyboard: KeyboardTab,
  hotkeys: HotkeysTab,
  transition: TransitionTab,
  clipboard: ClipboardTab,
  audio: AudioTab,
  general: GeneralTab,
} as const;

export function Settings() {
  const [tab, setTab] = useState<TabId>("connection");
  const [config, setConfig] = useState<Config | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState("");
  const [saved, setSaved] = useState(false);
  const [showErrors, setShowErrors] = useState(false);

  useEffect(() => {
    invokeCommand("get_config").then(setConfig, (e) => setSaveError(String(e)));
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
      setSaveError(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  if (!config) {
    return <main className={styles.window}>{saveError ? <Banner tone="danger">{saveError}</Banner> : <p className={styles.muted}>Carregando…</p>}</main>;
  }

  const View = VIEWS[tab];
  const count = Object.keys(errors).length;
  return (
    <main className={styles.window}>
      <Tabs tabs={TABS} active={tab} onChange={setTab}>
        <View config={config} update={update} errors={showErrors || tab === "hotkeys" ? errors : {}} />
      </Tabs>
      <footer className={styles.footer}>
        <span role="status" className={styles.footerMsg}>
          {saveError ? <span className={styles.error}>{saveError}</span> : showErrors && count > 0 ? <span className={styles.error}>Corrija {count} campo(s) antes de salvar.</span> : saved ? "Configurações salvas." : ""}
        </span>
        <Button variant="primary" disabled={!dirty || saving} onClick={save}>
          Salvar
        </Button>
      </footer>
    </main>
  );
}
