// dono: U2 (redesenho: ui-redesign). Painel do menu bar: só o essencial, no estilo de menu nativo.
import { CAPS, hasCap, invokeCommand, type PermissionsStatus } from "@relay/protocol-types";
import { Banner, Button, MenuRow, OPEN_UPDATE_EVENT, StatusDot, updateOf, useRelayEvent, useUpdater, type MatchPeer } from "@relay/ui";
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
import { CaptureMenu, GotoMenu, LayoutMenu } from "./ScreenMenus";
import { SoundMenu } from "./SoundMenu";
import { useFitWindow } from "./useFitWindow";
import { VolumeBlock } from "./VolumeBlock";

const WIDTH = 360;

/** Abre a janela principal já com o modal da atualização e some com o painel. */
function openUpdateFromPanel(matchPeer?: MatchPeer) {
  void openMain()
    .then(() => emitTo("main", OPEN_UPDATE_EVENT, matchPeer))
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
    // A bandeja do Windows mostra "Atualização disponível" (o painel fica escondido); no Mac é no-op.
    void invokeCommand("set_update_available", { version: update ?? null }).catch(() => undefined);
  }, [update]);
  // O PC já está numa versão mais nova (ele é quem manda): procura na hora e mostra o modal.
  const peer = status.peer_name ?? "outro computador";
  const behindPeer = status.version_sync === "LocalBehind" ? status.peer_version : null;
  const verify = updater.verify;
  useEffect(() => {
    if (!behindPeer) return;
    void verify().then((r) => r.resultado === "disponivel" && openUpdateFromPanel({ name: peer, version: behindPeer }));
  }, [behindPeer, verify, peer]);
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
    if (r.error === "vbcable_missing") {
      // O microfone não subiu no PC: desliga o toggle e relê o status.
      void invokeCommand("set_audio_streams", { mic_to_peer: false }).then(refresh, () => undefined);
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
  const pcUpdate = !online
    ? null
    : status.version_sync === "PeerBehind"
      ? `Atualizando ${peer} para a versão ${updater.version || "nova"}…`
      : status.version_sync === "PeerBehindManual"
        ? `Atualize o Relay no ${peer} para ${updater.version || "a versão mais nova"}.`
        : null;

  const retry = (
    <Button variant="ghost" onClick={() => void act("Reconectar", () => invokeCommand("reconnect"))}>Tentar de novo</Button>
  );
  const mic = status.audio.mic_to_peer;
  const hasVolume = hasCap(status.peer_caps, CAPS.VOLUME);
  const hasMic = hasCap(status.local_caps, CAPS.MIC_SOURCE) && hasCap(status.peer_caps, CAPS.MIC_SINK);

  return (
    <main ref={root} className={styles.panel}>
      <header className={styles.header}>
        <div className={styles.titleRow}>
          <h1 className={styles.title}>Relay</h1>
          <span className={styles.peer}>{status.peer_name ?? ""}</span>
        </div>
        <div className={styles.subRow}>
          <StatusDot tone={statusTone(key)} label={STATUS_LABELS[key] ?? key} />
          <span>{firstUse ? "Nenhum computador pareado" : `${STATUS_LABELS[key] ?? key}${latency && online ? ` · ${decimal(latency.rtt_ms)} ms` : ""}`}</span>
          {firstUse ? null : <span className={styles.focus}>{status.focus === "Local" ? "Foco: Este computador" : `Foco: ${peer}`}</span>}
        </div>
      </header>

      {secure || missing.length > 0 || incompatible || status.peer_unreachable || key === "Reconnecting" || pcUpdate ? (
        <div className={styles.banners}>
          {secure ? <Banner variant="line" tone="warn">Secure Input ativo: o teclado não é encaminhado a {peer}.</Banner> : null}
          {missing.map(([kind, label]) => (
            <Banner
              key={kind} variant="line" tone="danger"
              action={<Button variant="ghost" onClick={() => void act("Ajustes do sistema", () => invokeCommand("open_permission_settings", { kind }))}>Abrir ajustes</Button>}
            >
              Permissão faltando: {label}
            </Banner>
          ))}
          {incompatible ? <Banner variant="line" tone="danger" action={retry}>Versão incompatível (este computador {incompatible.local}, {peer} {incompatible.remote}). Atualize os dois lados.</Banner> : null}
          {status.peer_unreachable ? <Banner variant="line" tone="warn" action={retry}>{peer} está na rede mas não responde. O roteador pode estar isolando o Wi-Fi do cabo: tente a outra banda do Wi-Fi (2.4/5 GHz) ou ligue um dos computadores por cabo.</Banner> : null}
          {key === "Reconnecting" ? <Banner variant="line" tone="warn" action={retry}>Reconectando a {peer}…</Banner> : null}
          {pcUpdate ? <Banner variant="line">{pcUpdate}</Banner> : null}
        </div>
      ) : null}

      {firstUse ? (
        <>
          <hr className={styles.sep} />
          <div className={styles.group}>
            <MenuRow label="Conectar outro computador…" icon="link" onClick={() => openFromPanel()} />
          </div>
          <hr className={styles.sep} />
        </>
      ) : (
        <>
        <hr className={styles.sep} />
        {hasVolume ? <><VolumeBlock act={act} online={online} peer={peer} /><hr className={styles.sep} /></> : null}

        <div className={styles.group}>
          <MenuRow
            label="Travar cursor" icon="lock" checked={status.locked} shortcut={hotkey("ToggleLock")}
            onClick={() => void act("Travar cursor", () => invokeCommand("set_cursor_lock", { locked: !status.locked })).then(refresh)}
          />
          <MenuRow
            label="Pausar compartilhamento" icon="pause" checked={status.paused} shortcut={hotkey("TogglePause")}
            onClick={() => void act("Pausar", () => invokeCommand("set_sharing_paused", { paused: !status.paused })).then(refresh)}
          />
          {hasMic ? (
            <MenuRow
              label={`Microfone deste computador em ${peer}`} icon="mic" checked={mic}
              onClick={() => void act("Microfone", () => invokeCommand("set_audio_streams", { mic_to_peer: !mic })).then(refresh)}
            />
          ) : null}
        </div>
        <hr className={styles.sep} />

        <div className={styles.group}>
          {update ? <MenuRow label={`Atualização ${update} disponível`} icon="download" onClick={() => openUpdateFromPanel()} /> : null}
          <MenuRow label="Abrir Relay…" icon="window" onClick={() => openFromPanel()} />
          {hasVolume ? <SoundMenu act={act} online={online} peer={peer} /> : null}
          <LayoutMenu act={act} />
          <GotoMenu act={act} online={online} hotkey={(n) => hotkey({ JumpTo: n })} />
          {online && hasCap(status.peer_caps, CAPS.SCREEN_CAPTURE) ? <CaptureMenu show={show} firstTime={firstTime} peer={peer} /> : null}
          <PcMenu act={act} online={online} peer={peer} canCommand={hasCap(status.peer_caps, CAPS.COMMANDS)} canWake={canWake} hasWake={hasCap(status.peer_caps, CAPS.WOL_TARGET)} wakeShortcut={hotkey("WakePc")} />
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
