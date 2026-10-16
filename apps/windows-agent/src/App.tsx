import {
  invokeCommand,
  type Config,
  type ConnectionStatus,
  type PcAudioMode,
  type ServiceStatus,
  type StatusSnapshot,
} from "@relay/protocol-types";
import { Banner, Section, Select, StatusDot, Toggle, UpdateAbout, updateOf, useRelayEvent, useUpdate, type StatusTone } from "@relay/ui";
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

  useEffect(() => {
    const fail = (e: unknown) => setError(String(e));
    invokeCommand("agent_get_status").then(setSnapshot, fail);
    invokeCommand("agent_get_service_status").then(setService, fail);
    invokeCommand("agent_get_config").then(setConfig, fail);
  }, []);

  useRelayEvent("relay:status", setSnapshot);

  // A bandeja mostra "Atualização disponível" enquanto houver versão nova em mãos.
  const { state: updateState } = useUpdate();
  const newVersion = updateOf(updateState)?.version ?? null;
  useEffect(() => {
    invokeCommand("agent_set_update_available", { version: newVersion }).catch(() => undefined);
  }, [newVersion]);
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
          </>
        )}
      </Section>

      <UpdateAbout />
    </main>
  );
}
