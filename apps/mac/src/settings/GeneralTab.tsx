// dono: U3. Iniciar no login, atualizações e permissões. Cada botão chama o comando na hora.
import type { PermissionKind, PermissionState } from "@relay/protocol-types";
import { invokeCommand } from "@relay/protocol-types";
import { Banner, Button, Section, StatusDot, Toggle, useRelayEvent } from "@relay/ui";
import { useEffect, useState } from "react";
import type { TabProps } from "./ConnectionTab";
import styles from "./settings.module.css";

const msg = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
const STATE_LABEL: Record<PermissionState, string> = { Granted: "Concedida", Denied: "Negada", Unknown: "Desconhecida" };
const PERMS: ReadonlyArray<{ kind: PermissionKind; key: "accessibility" | "input_monitoring"; label: string }> = [
  { kind: "Accessibility", key: "accessibility", label: "Acessibilidade" },
  { kind: "InputMonitoring", key: "input_monitoring", label: "Monitoramento de entrada" },
];

export function GeneralTab({ config, update }: TabProps) {
  const [perms, setPerms] = useState<Record<string, PermissionState>>({});
  const [updateMsg, setUpdateMsg] = useState("");
  const [version, setVersion] = useState<string | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    invokeCommand("check_permissions").then((p) => setPerms({ ...p }), () => undefined);
  }, []);
  useRelayEvent("relay:permissions", (p) => setPerms({ ...p }));

  const setLogin = async (enabled: boolean) => {
    setError("");
    try {
      await invokeCommand("set_launch_at_login", { enabled });
      update((c) => ({ ...c, general: { ...c.general, launch_at_login: enabled } }));
    } catch (e) {
      setError(msg(e));
    }
  };

  const check = async () => {
    setError("");
    setUpdateMsg("Verificando…");
    try {
      const v = await invokeCommand("check_updates");
      setVersion(v);
      setUpdateMsg(v ? `Versão ${v} disponível.` : "Você já está na versão mais recente.");
    } catch (e) {
      setUpdateMsg("");
      setError(msg(e));
    }
  };

  const install = () => invokeCommand("install_update").catch((e) => setError(msg(e)));

  return (
    <>
      {error ? <Banner tone="danger">{error}</Banner> : null}
      <Section title="Início" grouped>
        <Toggle label="Iniciar no login" checked={config.general.launch_at_login} onChange={setLogin} />
      </Section>
      <Section title="Atualizações" grouped>
        <div className={styles.row}>
          <span className={styles.rowMain} role="status">
            {updateMsg}
          </span>
          <Button onClick={check}>Verificar atualizações</Button>
          {version ? (
            <Button variant="primary" onClick={install}>
              Instalar
            </Button>
          ) : null}
        </div>
      </Section>
      <Section title="Permissões">
        {PERMS.map((p) => {
          const state = perms[p.key] ?? "Unknown";
          return (
            <div key={p.kind} className={styles.row}>
              <span className={styles.rowMain}>
                <StatusDot tone={state === "Granted" ? "ok" : state === "Denied" ? "danger" : "warn"} label={STATE_LABEL[state]} /> {p.label}:{" "}
                {STATE_LABEL[state]}
              </span>
              <Button onClick={() => invokeCommand("open_permission_settings", { kind: p.kind }).catch((e) => setError(msg(e)))}>
                Abrir Ajustes
              </Button>
            </div>
          );
        })}
        <Button variant="ghost" onClick={() => invokeCommand("check_permissions").then((p) => setPerms({ ...p }), (e) => setError(msg(e)))}>
          Verificar de novo
        </Button>
      </Section>
    </>
  );
}
