// Janela principal "Relay": sidebar de seções (estilo Ajustes do Sistema) + conteúdo.
import { invokeCommand, type TransferProgress } from "@relay/protocol-types";
import { Icon, StatusDot, updateOf, UpdaterProvider, useRelayEvent, useUpdate, type IconName } from "@relay/ui";
import { listen } from "@tauri-apps/api/event";
import { Fragment, useEffect, useState } from "react";
import { Editor } from "../editor";
import { SaveBar, SettingsPane, SettingsProvider } from "../settings";
import { STATUS_LABELS, statusKey, statusTone } from "../shared/status";
import { useActions } from "../shared/useActions";
import { useLiveStatus } from "../shared/useLiveStatus";
import type { Section } from "../shared/windows";
import { LiveAudio } from "./LiveAudio";
import styles from "./main.module.css";
import { Overview } from "./Overview";

const SECTIONS: ReadonlyArray<{ id: Section; label: string; icon: IconName }> = [
  { id: "overview", label: "Visão geral", icon: "home" },
  { id: "screens", label: "Telas", icon: "monitor" },
  { id: "shortcuts", label: "Teclado e atalhos", icon: "keyboard" },
  { id: "audio", label: "Áudio", icon: "volume" },
  { id: "clipboard", label: "Clipboard e arquivos", icon: "clipboard" },
  { id: "connection", label: "Conexão e pareamento", icon: "link" },
  { id: "general", label: "Geral", icon: "gear" },
];

const isSection = (v: unknown): v is Section => SECTIONS.some((s) => s.id === v);

/**
 * A janela principal não agenda checagens (o dono do timer é o painel, que existe desde o boot):
 * checa uma vez ao montar e de novo quando o painel avisa com `relay:update-available`.
 */
export function Main() {
  return (
    <UpdaterProvider schedule={false}>
      <MainShell />
    </UpdaterProvider>
  );
}

function MainShell() {
  const { state, verify, openModal } = useUpdate();
  const newVersion = updateOf(state)?.version;
  useRelayEvent("relay:update-available", () => void verify());
  const [section, setSection] = useState<Section>("overview");
  const { status, latency, history, refresh } = useLiveStatus();
  const metrics = useRelayEvent("relay:metrics");
  const audioStats = useRelayEvent("relay:audio-stats");
  const [transfers, setTransfers] = useState<TransferProgress[]>([]);
  const { act, toast } = useActions();

  useRelayEvent("relay:transfer-progress", (t) =>
    setTransfers((list) =>
      list.some((x) => x.transfer_id === t.transfer_id)
        ? list.map((x) => (x.transfer_id === t.transfer_id ? t : x))
        : [t, ...list].slice(0, 8),
    ),
  );

  // O painel e a bandeja pedem uma seção com `relay:navigate`.
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    listen<string>("relay:navigate", (e) => isSection(e.payload) && setSection(e.payload))
      .then((fn) => (gone ? fn() : (off = fn)))
      .catch(() => undefined);
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  const key = statusKey(status.status);
  const online = key === "Connected";
  const current = SECTIONS.find((s) => s.id === section) ?? SECTIONS[0]!;

  return (
    <SettingsProvider>
      <div className={styles.shell}>
        <nav className={styles.sidebar} aria-label="Seções">
          <div className={styles.drag} data-tauri-drag-region />
          {SECTIONS.map((s) => (
            <Fragment key={s.id}>
              {s.id === "general" && newVersion ? (
                <button type="button" className={`${styles.item} ${styles.update}`} onClick={openModal}>
                  <Icon name="download" size={16} />
                  <span>Atualização {newVersion} disponível</span>
                </button>
              ) : null}
              <button
                type="button" className={styles.item}
                aria-current={s.id === section ? "page" : undefined} onClick={() => setSection(s.id)}
              >
                <Icon name={s.icon} size={16} />
                <span>{s.label}</span>
              </button>
            </Fragment>
          ))}
          <div className={styles.sideStatus}>
            <StatusDot tone={statusTone(key)} label={STATUS_LABELS[key] ?? key} />
            <span>{STATUS_LABELS[key] ?? key}{status.peer_name ? ` · ${status.peer_name}` : ""}</span>
          </div>
        </nav>

        <div className={styles.content}>
          <header className={styles.titlebar} data-tauri-drag-region>
            <h1 className={styles.pageTitle}>{current.label}</h1>
          </header>
          <main className={styles.scroll} key={section}>
            {section === "overview" ? (
              <Overview act={act} status={status} latency={latency} history={history} metrics={metrics} transfers={transfers} />
            ) : null}
            {section === "screens" ? <div className={styles.screens}><Editor /></div> : null}
            {section === "shortcuts" ? <SettingsPane views={["keyboard", "hotkeys"]} /> : null}
            {section === "audio" ? (
              <>
                <LiveAudio act={act} online={online} audio={status.audio} stats={audioStats} refresh={refresh} />
                <SettingsPane views={["audio"]} />
              </>
            ) : null}
            {section === "clipboard" ? (
              <>
                <SettingsPane views={["clipboard"]} />
                <ClipboardNow act={act} />
              </>
            ) : null}
            {section === "connection" ? <SettingsPane views={["connection"]} /> : null}
            {section === "general" ? <SettingsPane views={["general", "transition"]} /> : null}
            <SaveBar />
          </main>
        </div>
        {toast}
      </div>
    </SettingsProvider>
  );
}

function ClipboardNow({ act }: { act: ReturnType<typeof useActions>["act"] }) {
  return (
    <div className={styles.inlineAction}>
      <button
        type="button" className={styles.linkAction}
        onClick={() => void act("Clipboard enviado", () => invokeCommand("clipboard_send_now"))}
      >
        <Icon name="send" /> Enviar clipboard agora
      </button>
      <button
        type="button" className={styles.linkAction}
        onClick={() => void act("Receber arquivos", () => invokeCommand("receive_remote_files"))}
      >
        <Icon name="download" /> Receber arquivos do PC
      </button>
    </div>
  );
}
