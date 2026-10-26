// Aba "Este computador": o que é configuração desta máquina e depende do SO dela (D45), não do par.
import type { PermissionKind, PermissionState, ServiceStatus } from "@relay/protocol-types";
import { invokeCommand } from "@relay/protocol-types";
import { Banner, Button, Section, StatusDot, Toggle, useRelayEvent } from "@relay/ui";
import { useEffect, useState } from "react";
import { errorMessage } from "../shared/errors";
import { useStatus } from "../shared/useStatus";
import type { TabProps } from "./ConnectionTab";
import styles from "./settings.module.css";

const STATE_LABEL: Record<PermissionState, string> = { Granted: "Concedida", Denied: "Negada", Unknown: "Desconhecida" };
const PERMS: ReadonlyArray<{ kind: PermissionKind; key: "accessibility" | "input_monitoring"; label: string }> = [
  { kind: "Accessibility", key: "accessibility", label: "Acessibilidade" },
  { kind: "InputMonitoring", key: "input_monitoring", label: "Monitoramento de entrada" },
];

export function ComputerTab({ config, update }: TabProps) {
  const status = useStatus();
  const [error, setError] = useState("");

  const setLogin = async (enabled: boolean) => {
    setError("");
    try {
      await invokeCommand("set_launch_at_login", { enabled });
      update((c) => ({ ...c, general: { ...c.general, launch_at_login: enabled } }));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  if (!status) return <p className={styles.muted}>Carregando…</p>;
  return (
    <>
      {error ? <Banner tone="danger">{error}</Banner> : null}
      <Section title="Início" grouped>
        <Toggle
          label={status.local_os === "Windows" ? "Iniciar com o Windows" : "Iniciar no login"}
          checked={config.general.launch_at_login}
          onChange={setLogin}
        />
      </Section>
      {status.local_os === "Mac" ? <Permissions onError={setError} /> : null}
      {status.local_os === "Windows" ? <WindowsSection onError={setError} /> : null}
    </>
  );
}

function Permissions({ onError }: { onError: (m: string) => void }) {
  const [perms, setPerms] = useState<Record<string, PermissionState>>({});
  useEffect(() => {
    invokeCommand("check_permissions").then((p) => setPerms({ ...p }), () => undefined);
  }, []);
  useRelayEvent("relay:permissions", (p) => setPerms({ ...p }));
  return (
    <Section title="Permissões">
      {PERMS.map((p) => {
        const state = perms[p.key] ?? "Unknown";
        return (
          <div key={p.kind} className={styles.row}>
            <span className={styles.rowMain}>
              <StatusDot tone={state === "Granted" ? "ok" : state === "Denied" ? "danger" : "warn"} label={STATE_LABEL[state]} /> {p.label}:{" "}
              {STATE_LABEL[state]}
            </span>
            <Button onClick={() => invokeCommand("open_permission_settings", { kind: p.kind }).catch((e) => onError(errorMessage(e)))}>
              Abrir Ajustes
            </Button>
          </div>
        );
      })}
      <Button variant="ghost" onClick={() => invokeCommand("check_permissions").then((p) => setPerms({ ...p }), (e) => onError(errorMessage(e)))}>
        Verificar de novo
      </Button>
    </Section>
  );
}

function WindowsSection({ onError }: { onError: (m: string) => void }) {
  const [svc, setSvc] = useState<ServiceStatus | null>(null);
  useEffect(() => {
    invokeCommand("get_service_status").then(setSvc, (e) => onError(errorMessage(e)));
  }, [onError]);
  if (!svc) return null;
  const rows: ReadonlyArray<[string, boolean, string, string]> = [
    ["Serviço do Relay", svc.installed && svc.running, svc.installed ? (svc.running ? "Em execução" : "Parado") : "Não instalado", svc.installed ? "warn" : "danger"],
    ["Helper de entrada", svc.helper_active, svc.helper_active ? "Ativo" : "Inativo", "warn"],
    ["VB-CABLE (microfone)", svc.vbcable_detected, svc.vbcable_detected ? "Detectado" : "Não detectado", "warn"],
  ];
  return (
    <Section title="Serviço e áudio" grouped>
      {rows.map(([label, ok, text, bad]) => (
        <div key={label} className={styles.row}>
          <span className={styles.rowMain}>
            <StatusDot tone={ok ? "ok" : (bad as "warn" | "danger")} label={text} /> {label}
          </span>
          <span className={styles.muted}>{text}</span>
        </div>
      ))}
    </Section>
  );
}
