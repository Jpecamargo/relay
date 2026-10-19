// dono: U2 (redesenho: ui-redesign). Painel do menu bar: só o essencial, no estilo de menu nativo.
import { invokeCommand, type PermissionsStatus } from "@relay/protocol-types";
import { Banner, Button, MenuRow, OPEN_UPDATE_EVENT, StatusDot, updateOf, useRelayEvent, useUpdater } from "@relay/ui";
import { emit, emitTo } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useRef, useState } from "react";
import { errorText } from "../shared/errors";
import { decimal, STATUS_LABELS, statusKey, statusTone } from "../shared/status";
import { useActions } from "../shared/useActions";
import { useHotkeys } from "../shared/useHotkeys";
import { useLiveStatus } from "../shared/useLiveStatus";
import { openMain } from "../shared/windows";
import styles from "./panel.module.css";
import { PcMenu } from "./PcMenu";
import { CaptureMenu, GotoMenu } from "./ScreenMenus";
import { SoundMenu } from "./SoundMenu";
import { useFitWindow } from "./useFitWindow";
import { VolumeBlock } from "./VolumeBlock";

const WIDTH = 360;

/** Erros de áudio do PC e o toggle que eles desfazem. */
const AUDIO_ERRORS: Record<string, "pc_to_mac" | "mac_to_pc"> = {
  vbcable_missing: "mac_to_pc",
  virtual_output_missing: "pc_to_mac",
  process_not_found: "pc_to_mac",
};

/** Abre a janela principal já com o modal da atualização e some com o painel. */
function openUpdateFromPanel() {
  void openMain()
    .then(() => emitTo("main", OPEN_UPDATE_EVENT))
    .then(() => getCurrentWindow().hide())
    .catch(() => undefined);
}

/** Abre a janela principal e some com o painel. */
function openFromPanel(section?: Parameters<typeof openMain>[0]) {
  void openMain(section)
    .then(() => getCurrentWindow().hide())
    .catch(() => undefined);
}

export function Panel() {
  const { status, latency, refresh, loaded } = useLiveStatus();
  const secureEvent = useRelayEvent("relay:secure-input");
  const permEvent = useRelayEvent("relay:permissions");

  const [perms, setPerms] = useState<PermissionsStatus>();
  // O painel existe desde o boot: é o único dono do timer de atualizações (15 s após abrir, depois
  // a cada 6 h). Ao achar versão nova avisa a janela principal, que só repete a checagem sob demanda.
  const updater = useUpdater();
  const update = updateOf(updater.state)?.version;
  useEffect(() => {
    if (update) void emit("relay:update-available", { version: update }).catch(() => undefined);
  }, [update]);
  const { act, show, firstTime, toast } = useActions();
  const hotkey = useHotkeys();
  const root = useRef<HTMLElement>(null);
  useFitWindow(root, WIDTH);

  useEffect(() => {
    invokeCommand("check_permissions").then(setPerms).catch(() => undefined);
  }, []);

  const statusRef = useRef(status);
  statusRef.current = status;

  useRelayEvent("relay:command-result", (r) => {
    if (!firstTime(r)) return;
    const audio = r.error ? AUDIO_ERRORS[r.error] : undefined;
    if (audio) {
      // O stream não subiu: desliga o toggle correspondente e relê o status.
      const a = statusRef.current.audio;
      void invokeCommand("set_audio_streams", {
        pc_to_mac: audio === "pc_to_mac" ? false : a.pc_to_mac,
        mac_to_pc: audio === "mac_to_pc" ? false : a.mac_to_pc,
      }).then(refresh, () => undefined);
    }
    if (!r.ok) show(errorText(r.error ?? "falhou"), "danger");
  });

  // Atalhos do menu: ⌘, abre os Ajustes, ⌘Q encerra, Esc fecha o painel.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key === ",") {
        e.preventDefault();
        openFromPanel(statusRef.current.peer_name ? "general" : undefined);
      } else if (e.metaKey && e.key.toLowerCase() === "q") {
        e.preventDefault();
        void invokeCommand("quit_app");
      } else if (e.key === "Escape") {
        void getCurrentWindow().hide().catch(() => undefined);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const key = statusKey(status.status);
  const online = key === "Connected";
  const firstUse = loaded && !status.peer_name;
  const canWake = key === "Disconnected" || key === "Discovering";
  const secure = secureEvent?.active ?? status.secure_input;
  const permissions = permEvent ?? perms;
  const missing = (
    [["Accessibility", "Acessibilidade", permissions?.accessibility], ["InputMonitoring", "Monitoramento de entrada", permissions?.input_monitoring]] as const
  ).filter(([, , s]) => s !== undefined && s !== "Granted");
  const incompatible = typeof status.status === "object" ? status.status.IncompatibleVersion : undefined;

  const retry = (
    <Button variant="ghost" onClick={() => void act("Reconectar", () => invokeCommand("reconnect"))}>Tentar de novo</Button>
  );
  const audio = status.audio;
  const setAudio = (label: string, next: { pc_to_mac: boolean; mac_to_pc: boolean }) =>
    void act(label, () => invokeCommand("set_audio_streams", next)).then(refresh);

  return (
    <main ref={root} className={styles.panel}>
      <header className={styles.header}>
        <div className={styles.titleRow}>
          <h1 className={styles.title}>Relay</h1>
          <span className={styles.peer}>{status.peer_name ?? ""}</span>
        </div>
        <div className={styles.subRow}>
          <StatusDot tone={statusTone(key)} label={STATUS_LABELS[key] ?? key} />
          <span>{firstUse ? "Nenhum PC pareado" : `${STATUS_LABELS[key] ?? key}${latency && online ? ` · ${decimal(latency.rtt_ms)} ms` : ""}`}</span>
          {firstUse ? null : <span className={styles.focus}>{status.focus === "Mac" ? "Foco: Mac" : "Foco: Windows"}</span>}
        </div>
      </header>

      {secure || missing.length > 0 || incompatible || status.pc_unreachable || key === "Reconnecting" ? (
        <div className={styles.banners}>
          {secure ? <Banner variant="line" tone="warn">Secure Input ativo: o teclado não é encaminhado ao PC.</Banner> : null}
          {missing.map(([kind, label]) => (
            <Banner
              key={kind} variant="line" tone="danger"
              action={<Button variant="ghost" onClick={() => void act("Ajustes do sistema", () => invokeCommand("open_permission_settings", { kind }))}>Abrir ajustes</Button>}
            >
              Permissão faltando: {label}
            </Banner>
          ))}
          {incompatible ? <Banner variant="line" tone="danger" action={retry}>Versão incompatível (este Mac {incompatible.local}, PC {incompatible.remote}). Atualize os dois lados.</Banner> : null}
          {status.pc_unreachable ? <Banner variant="line" tone="warn" action={retry}>O PC está na rede mas não responde. O roteador pode estar isolando o Wi-Fi do cabo: tente a outra banda do Wi-Fi (2.4/5 GHz) ou ligue o Mac por cabo.</Banner> : null}
          {key === "Reconnecting" ? <Banner variant="line" tone="warn" action={retry}>Reconectando ao PC…</Banner> : null}
        </div>
      ) : null}

      {firstUse ? (
        <>
          <hr className={styles.sep} />
          <div className={styles.group}>
            <MenuRow label="Conectar seu PC…" icon="link" onClick={() => openFromPanel()} />
          </div>
          <hr className={styles.sep} />
        </>
      ) : (
        <>
        <hr className={styles.sep} />
        <VolumeBlock act={act} online={online} />
        <hr className={styles.sep} />

        <div className={styles.group}>
          <MenuRow
            label="Travar cursor" icon="lock" checked={status.locked} shortcut={hotkey("ToggleLock")}
            onClick={() => void act("Travar cursor", () => invokeCommand("set_cursor_lock", { locked: !status.locked })).then(refresh)}
          />
          <MenuRow
            label="Pausar compartilhamento" icon="pause" checked={status.paused} shortcut={hotkey("TogglePause")}
            onClick={() => void act("Pausar", () => invokeCommand("set_sharing_paused", { paused: !status.paused })).then(refresh)}
          />
          <MenuRow
            label="Som do PC no Mac" icon="volume" checked={audio.pc_to_mac}
            onClick={() => setAudio("Som do PC no Mac", { ...audio, pc_to_mac: !audio.pc_to_mac })}
          />
          <MenuRow
            label="Microfone do Mac no PC" icon="mic" checked={audio.mac_to_pc}
            onClick={() => setAudio("Microfone do Mac no PC", { ...audio, mac_to_pc: !audio.mac_to_pc })}
          />
        </div>
        <hr className={styles.sep} />

        <div className={styles.group}>
          {update ? <MenuRow label={`Atualização ${update} disponível`} icon="download" onClick={openUpdateFromPanel} /> : null}
          <MenuRow label="Abrir Relay…" icon="window" onClick={() => openFromPanel()} />
          <SoundMenu act={act} online={online} streamToMac={audio.pc_to_mac} />
          <GotoMenu act={act} online={online} hotkey={(n) => hotkey({ JumpTo: n })} />
          {online ? <CaptureMenu show={show} firstTime={firstTime} /> : null}
          <PcMenu act={act} online={online} canWake={canWake} wakeShortcut={hotkey("WakePc")} />
          <MenuRow
            label="Enviar clipboard agora" icon="clipboard" shortcut={hotkey("ClipboardSendNow")}
            onClick={() => void act("Clipboard enviado", () => invokeCommand("clipboard_send_now"), "Clipboard enviado")}
          />
          {key === "Disconnected" ? (
            <MenuRow label="Reconectar" icon="refresh" onClick={() => void act("Reconectar", () => invokeCommand("reconnect"))} />
          ) : null}
        </div>
        <hr className={styles.sep} />

        </>
      )}

      <div className={styles.group}>
        {firstUse ? null : <MenuRow label="Ajustes…" icon="gear" shortcut="⌘," onClick={() => openFromPanel("general")} />}
        <MenuRow label="Encerrar" icon="quit" shortcut="⌘Q" onClick={() => void invokeCommand("quit_app")} />
      </div>
      {toast}
    </main>
  );
}
