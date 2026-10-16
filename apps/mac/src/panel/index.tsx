// dono: U2
import {
  invokeCommand,
  type AudioDevices,
  type ConnectionStatus,
  type LatencyStats,
  type PermissionsStatus,
  type StatusSnapshot,
  type TransferProgress,
} from "@relay/protocol-types";
import { Banner, Button, LatencySparkline, StatusDot, Toast, useRelayEvent, type StatusTone } from "@relay/ui";
import { useCallback, useEffect, useRef, useState } from "react";
import { errorText, isCommandResult } from "./errors";
import styles from "./panel.module.css";
import {
  AudioSection, CursorClipboardSections, MediaSection, PowerSection,
  SystemSection, TransfersSection, UtilitiesSection, type Act,
} from "./sections";

const HISTORY = 30;

const EMPTY: StatusSnapshot = {
  status: "Disconnected", peer_name: null, focus: "Mac", locked: false, paused: false,
  secure_input: false, clipboard_sync: false, audio: { pc_to_mac: false, mac_to_pc: false },
};

const LABELS: Record<string, string> = {
  Disconnected: "Desconectado", Discovering: "Procurando o PC", Connecting: "Conectando",
  Pairing: "Pareando", Connected: "Conectado", Reconnecting: "Reconectando",
  IncompatibleVersion: "Versão incompatível",
};

function statusKey(s: ConnectionStatus): string {
  return typeof s === "string" ? s : Object.keys(s)[0] ?? "Disconnected";
}
function tone(key: string): StatusTone {
  if (key === "Connected") return "ok";
  if (key === "Reconnecting" || key === "Connecting" || key === "Pairing") return "warn";
  if (key === "IncompatibleVersion") return "danger";
  return "idle";
}

interface ToastState { message: string; tone: "ok" | "danger"; n: number }

export function Panel() {
  const [initial, setInitial] = useState<StatusSnapshot>(EMPTY);
  const statusEvent = useRelayEvent("relay:status");
  const status = statusEvent ?? initial;

  const [history, setHistory] = useState<number[]>([]);
  const latency: LatencyStats | undefined = useRelayEvent("relay:latency", (l) =>
    setHistory((h) => [...h.slice(-(HISTORY - 1)), l.rtt_ms]),
  );
  const metrics = useRelayEvent("relay:metrics");
  const audioStats = useRelayEvent("relay:audio-stats");
  const secureEvent = useRelayEvent("relay:secure-input");
  const permEvent = useRelayEvent("relay:permissions");
  const updateEvent = useRelayEvent("relay:update-available");

  const [perms, setPerms] = useState<PermissionsStatus>();
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [macDevices, setMacDevices] = useState<AudioDevices>();
  const [streamVolume, setStreamVolume] = useState(1);
  const [transfers, setTransfers] = useState<TransferProgress[]>([]);
  const [toast, setToast] = useState<ToastState>();
  const toastN = useRef(0);

  const refresh = useCallback(() => {
    invokeCommand("get_status").then(setInitial).catch(() => undefined);
  }, []);

  useEffect(() => {
    refresh();
    invokeCommand("check_permissions").then(setPerms).catch(() => undefined);
    invokeCommand("check_updates").then(setUpdateVersion).catch(() => undefined);
    invokeCommand("list_audio_devices").then(setMacDevices).catch(() => undefined);
    invokeCommand("get_config").then((c) => setStreamVolume(c.audio.stream_volume)).catch(() => undefined);
  }, [refresh]);

  useRelayEvent("relay:transfer-progress", (t) =>
    setTransfers((list) =>
      list.some((x) => x.transfer_id === t.transfer_id)
        ? list.map((x) => (x.transfer_id === t.transfer_id ? t : x))
        : [...list, t],
    ),
  );

  const show = (message: string, t: "ok" | "danger") => setToast({ message, tone: t, n: ++toastN.current });

  const act: Act = useCallback(async (label, fn) => {
    try {
      const r = await fn();
      if (isCommandResult(r) && !r.ok) {
        show(`${label}: ${errorText(r.error ?? "falhou")}`, "danger");
        return r;
      }
      show(`${label}: ok`, "ok");
      return r;
    } catch (e) {
      show(`${label}: ${errorText(e)}`, "danger");
      return undefined;
    }
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
    <Button onClick={() => void act("Reconectar", () => invokeCommand("reconnect"))}>Tentar de novo</Button>
  );

  return (
    <main className={styles.panel} style={{ position: "relative" }}>
      <header className={styles.header}>
        <StatusDot tone={tone(key)} label={LABELS[key] ?? key} />
        <div className={styles.headerMain}>
          <span className={styles.badge}>{LABELS[key] ?? key}{status.peer_name ? ` · ${status.peer_name}` : ""}</span>
          <span className={styles.sub}>
            {latency ? `${latency.rtt_ms.toFixed(1)} ms · jitter ${latency.jitter_ms.toFixed(1)} ms` : "latência —"}
          </span>
        </div>
        <LatencySparkline values={history} width={64} height={22} />
        <span className={styles.focus}>{status.focus === "Mac" ? "Foco: Mac" : "Foco: Windows"}</span>
      </header>

      <div className={styles.banners}>
        {secure ? <Banner tone="warn">Secure Input ativo: o teclado não é encaminhado ao PC.</Banner> : null}
        {missing.map(([kind, label]) => (
          <Banner
            key={kind} tone="danger"
            action={<Button onClick={() => void act("Ajustes do sistema", () => invokeCommand("open_permission_settings", { kind }))}>Abrir ajustes</Button>}
          >
            Permissão faltando: {label}
          </Banner>
        ))}
        {incompatible ? <Banner tone="danger" action={retry}>Versão incompatível (este Mac {incompatible.local}, PC {incompatible.remote}). Atualize os dois lados.</Banner> : null}
        {key === "Reconnecting" ? <Banner tone="warn" action={retry}>Reconectando ao PC…</Banner> : null}
        {update ? (
          <Banner tone="info" action={<Button variant="primary" onClick={() => void act("Atualização", () => invokeCommand("install_update"))}>Instalar</Button>}>
            Atualização disponível: {update}
          </Banner>
        ) : null}
      </div>

      <div className={styles.scroll}>
        <AudioSection
          act={act} refresh={refresh} online={online} audio={status.audio} stats={audioStats}
          macDevices={macDevices} streamVolume={streamVolume} setStreamVolume={setStreamVolume}
        />
        <MediaSection act={act} refresh={refresh} online={online} />
        <PowerSection act={act} refresh={refresh} online={online} canWake={canWake} />
        <CursorClipboardSections act={act} refresh={refresh} online={online} status={status} />
        <SystemSection metrics={metrics} />
        <UtilitiesSection act={act} refresh={refresh} online={online} />
        <TransfersSection act={act} transfers={transfers} />
      </div>

      {toast ? (
        <div className={styles.toast}>
          <Toast key={toast.n} message={toast.message} tone={toast.tone} onDismiss={() => setToast(undefined)} />
        </div>
      ) : null}
    </main>
  );
}
