import {
  invokeCommand,
  type Config,
  type ConnectionStatus,
  type PcAudioMode,
  type ServiceStatus,
  type StatusSnapshot,
} from "@relay/protocol-types";
import { Banner, Button, Section, Select, StatusDot, Toggle, useRelayEvent, type StatusTone } from "@relay/ui";
import { useEffect, useState } from "react";
import "./app.css";
import { Pairing } from "./Pairing";

const STATUS_LABEL: Record<string, string> = {
  Disconnected: "Desconectado",
  Discovering: "Procurando o Mac",
  Connecting: "Conectando",
  Pairing: "Pareando",
  Connected: "Conectado",
  Reconnecting: "Reconectando",
};

function statusInfo(s: ConnectionStatus | undefined): { tone: StatusTone; label: string } {
  if (s === undefined) return { tone: "idle", label: "Carregando" };
  if (typeof s !== "string") return { tone: "danger", label: "Versão incompatível" };
  const tone: StatusTone = s === "Connected" ? "ok" : s === "Disconnected" ? "idle" : "warn";
  return { tone, label: STATUS_LABEL[s] ?? s };
}

const PC_MODES = [
  { value: "process", label: "Só o app (por processo)" },
  { value: "system", label: "Todo o sistema" },
] as const;

export function App() {
  const [snapshot, setSnapshot] = useState<StatusSnapshot>();
  const [service, setService] = useState<ServiceStatus>();
  const [config, setConfig] = useState<Config>();
  const [error, setError] = useState<string>();
  const [update, setUpdate] = useState<{ checking: boolean; version?: string | null; installing: boolean }>({ checking: false, installing: false });

  useEffect(() => {
    const fail = (e: unknown) => setError(String(e));
    invokeCommand("agent_get_status").then(setSnapshot, fail);
    invokeCommand("agent_get_service_status").then(setService, fail);
    invokeCommand("agent_get_config").then(setConfig, fail);
  }, []);

  useRelayEvent("relay:status", setSnapshot);
  const latency = useRelayEvent("relay:latency");

  const save = async (next: Config) => {
    const previous = config;
    setConfig(next);
    try {
      await invokeCommand("agent_save_config", { config: next });
    } catch (e) {
      setConfig(previous);
      setError(String(e));
    }
  };

  const setAutostart = async (enabled: boolean) => {
    if (!config) return;
    try {
      await invokeCommand("agent_set_autostart", { enabled });
      await save({ ...config, general: { ...config.general, launch_at_login: enabled } });
    } catch (e) {
      setError(String(e));
    }
  };

  const checkUpdates = async () => {
    setUpdate({ checking: true, installing: false });
    try {
      const found = await invokeCommand("agent_check_updates");
      setUpdate({ checking: false, installing: false, version: found?.version ?? null });
    } catch (e) {
      setUpdate({ checking: false, installing: false });
      setError(String(e));
    }
  };

  const installUpdate = async () => {
    setUpdate((u) => ({ ...u, installing: true }));
    try {
      await invokeCommand("agent_install_update");
    } catch (e) {
      setUpdate((u) => ({ ...u, installing: false }));
      setError(String(e));
    }
  };

  const { tone, label } = statusInfo(snapshot?.status);
  const connected = snapshot?.status === "Connected";
  const vbcable = service?.vbcable_detected ?? false;

  return (
    <main className="app">
      {error ? <Banner tone="danger">{error}</Banner> : null}

      <Section title="Status">
        <div className="row">
          <StatusDot tone={tone} label={label} />
          <strong>{label}</strong>
          {snapshot?.peer_name ? <span className="muted">· {snapshot.peer_name}</span> : null}
        </div>
        {connected ? (
          <dl className="facts">
            <dt>Foco</dt>
            <dd>{snapshot?.focus === "Windows" ? "Este PC" : "Mac"}</dd>
            <dt>Latência</dt>
            <dd>{latency ? `${latency.rtt_ms.toFixed(1)} ms (jitter ${latency.jitter_ms.toFixed(1)} ms)` : "medindo…"}</dd>
          </dl>
        ) : null}
      </Section>

      <Pairing peerName={snapshot?.peer_name ?? null} />

      <Section title="Serviço">
        {service === undefined ? (
          <p className="muted">Verificando…</p>
        ) : (
          <>
            <div className="row">
              <StatusDot tone={service.running ? "ok" : "danger"} label={service.running ? "Rodando" : "Parado"} />
              <span>
                {service.installed ? (service.running ? "Serviço rodando" : "Serviço instalado, parado") : "Serviço não instalado"}
              </span>
            </div>
            <div className="row">
              <StatusDot tone={service.helper_active ? "ok" : "warn"} label={service.helper_active ? "Ativo" : "Inativo"} />
              <span>{service.helper_active ? "Helper de sessão ativo" : "Helper de sessão inativo"}</span>
            </div>
            {!service.installed ? (
              <Banner tone="warn">
                Sem o serviço, o Relay não controla a tela de login nem janelas elevadas. Reinstale o Relay Agent como administrador.
              </Banner>
            ) : null}
          </>
        )}
      </Section>

      <Section title="Ajustes">
        {config === undefined ? (
          <p className="muted">Carregando…</p>
        ) : (
          <>
            <Select<PcAudioMode>
              label="Áudio do PC"
              value={config.audio.pc_mode}
              options={PC_MODES}
              onChange={(pc_mode) => void save({ ...config, audio: { ...config.audio, pc_mode } })}
            />
            <div className="row">
              <StatusDot tone={vbcable ? "ok" : "warn"} label={vbcable ? "Detectado" : "Não detectado"} />
              <span>{vbcable ? "VB-CABLE detectado" : "VB-CABLE não detectado"}</span>
            </div>
            {!vbcable ? <p className="muted">Instale o VB-CABLE para enviar o áudio do Mac a este PC.</p> : null}
            <Toggle
              label="Sincronizar área de transferência"
              checked={config.clipboard.enabled}
              onChange={(enabled) => void save({ ...config, clipboard: { ...config.clipboard, enabled } })}
            />
            <Toggle label="Iniciar com o Windows" checked={config.general.launch_at_login} onChange={(v) => void setAutostart(v)} />
            <div className="row">
              <Button onClick={() => void checkUpdates()} disabled={update.checking || update.installing}>
                {update.checking ? "Verificando…" : "Verificar atualizações"}
              </Button>
              {update.version ? (
                <Button variant="primary" onClick={() => void installUpdate()} disabled={update.installing}>
                  {update.installing ? "Instalando…" : `Instalar ${update.version}`}
                </Button>
              ) : update.version === null ? (
                <span className="muted">Já está atualizado.</span>
              ) : null}
            </div>
          </>
        )}
      </Section>
    </main>
  );
}
