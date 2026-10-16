// dono: U2 (redesenho: ui-redesign). Painel do menu bar: só o essencial, no estilo de menu nativo.
import { invokeCommand, type PermissionsStatus } from "@relay/protocol-types";
import { Banner, Button, MenuRow, StatusDot, useRelayEvent } from "@relay/ui";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useRef, useState } from "react";
import { errorText } from "../shared/errors";
import { decimal, STATUS_LABELS, statusKey, statusTone } from "../shared/status";
import { useActions } from "../shared/useActions";
import { useLiveStatus } from "../shared/useLiveStatus";
import { openMain } from "../shared/windows";
import styles from "./panel.module.css";
import { PcMenu } from "./PcMenu";
import { useFitWindow } from "./useFitWindow";
import { VolumeBlock } from "./VolumeBlock";

const WIDTH = 360;

/** Erros de áudio do PC e o toggle que eles desfazem. */
const AUDIO_ERRORS: Record<string, "pc_to_mac" | "mac_to_pc"> = {
  vbcable_missing: "mac_to_pc",
  virtual_output_missing: "pc_to_mac",
};

/** Abre a janela principal e some com o painel. */
function openFromPanel(section?: Parameters<typeof openMain>[0]) {
  void openMain(section)
    .then(() => getCurrentWindow().hide())
    .catch(() => undefined);
}

export function Panel() {
  const { status, latency, refresh } = useLiveStatus();
  const secureEvent = useRelayEvent("relay:secure-input");
  const permEvent = useRelayEvent("relay:permissions");
  const updateEvent = useRelayEvent("relay:update-available");

  const [perms, setPerms] = useState<PermissionsStatus>();
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const { act, show, firstTime, toast } = useActions();
  const root = useRef<HTMLElement>(null);
  useFitWindow(root, WIDTH);

  useEffect(() => {
    invokeCommand("check_permissions").then(setPerms).catch(() => undefined);
    invokeCommand("check_updates").then(setUpdateVersion).catch(() => undefined);
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
    else if (r.id !== 0) show("Comando executado", "ok");
  });

  // Atalhos do menu: ⌘, abre os Ajustes, ⌘Q encerra, Esc fecha o painel.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key === ",") {
        e.preventDefault();
        openFromPanel("general");
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
  const canWake = key === "Disconnected" || key === "Discovering";
  const secure = secureEvent?.active ?? status.secure_input;
  const permissions = permEvent ?? perms;
  const missing = (
    [["Accessibility", "Acessibilidade", permissions?.accessibility], ["InputMonitoring", "Monitoramento de entrada", permissions?.input_monitoring]] as const
  ).filter(([, , s]) => s !== undefined && s !== "Granted");
  const update = updateEvent?.version ?? updateVersion;
  const incompatible = typeof status.status === "object" ? status.status.IncompatibleVersion : undefined;

  const retry = (
    <Button variant="ghost" onClick={() => void act("Reconectar", () => invokeCommand("reconnect"))}>Tentar de novo</Button>
  );
  const audio = status.audio;
  const setAudio = (label: string, next: { pc_to_mac: boolean; mac_to_pc: boolean }) =>
    void act(label, () => invokeCommand("set_audio_streams", next), true).then(refresh);

  return (
    <main ref={root} className={styles.panel}>
      <header className={styles.header}>
        <div className={styles.titleRow}>
          <h1 className={styles.title}>Relay</h1>
          <span className={styles.peer}>{status.peer_name ?? ""}</span>
        </div>
        <div className={styles.subRow}>
          <StatusDot tone={statusTone(key)} label={STATUS_LABELS[key] ?? key} />
          <span>{STATUS_LABELS[key] ?? key}{latency && online ? ` · ${decimal(latency.rtt_ms)} ms` : ""}</span>
          <span className={styles.focus}>{status.focus === "Mac" ? "Foco: Mac" : "Foco: Windows"}</span>
        </div>
      </header>

      {secure || missing.length > 0 || incompatible || key === "Reconnecting" || update ? (
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
          {key === "Reconnecting" ? <Banner variant="line" tone="warn" action={retry}>Reconectando ao PC…</Banner> : null}
          {update ? (
            <Banner variant="line" tone="info" action={<Button variant="primary" onClick={() => void act("Atualização", () => invokeCommand("install_update"))}>Instalar</Button>}>
              Atualização disponível: {update}
            </Banner>
          ) : null}
        </div>
      ) : null}

      <hr className={styles.sep} />
      <VolumeBlock act={act} online={online} />
      <hr className={styles.sep} />

      <div className={styles.group}>
        <MenuRow
          label="Travar cursor" icon="lock" checked={status.locked}
          onClick={() => void act("Travar cursor", () => invokeCommand("set_cursor_lock", { locked: !status.locked }), true).then(refresh)}
        />
        <MenuRow
          label="Pausar compartilhamento" icon="pause" checked={status.paused}
          onClick={() => void act("Pausar", () => invokeCommand("set_sharing_paused", { paused: !status.paused }), true).then(refresh)}
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
        <MenuRow label="Abrir Relay…" icon="window" onClick={() => openFromPanel()} />
        <PcMenu act={act} online={online} canWake={canWake} />
        <MenuRow
          label="Enviar clipboard agora" icon="clipboard"
          onClick={() => void act("Clipboard enviado", () => invokeCommand("clipboard_send_now"))}
        />
      </div>
      <hr className={styles.sep} />

      <div className={styles.group}>
        <MenuRow label="Ajustes…" icon="gear" shortcut="⌘," onClick={() => openFromPanel("general")} />
        <MenuRow label="Encerrar" icon="quit" shortcut="⌘Q" onClick={() => void invokeCommand("quit_app")} />
      </div>
      {toast}
    </main>
  );
}
