// dono: U3. Iniciar no login, permissões e bloco Sobre (versão e atualização). Cada botão chama o comando na hora.
import type { PermissionKind, PermissionState } from "@relay/protocol-types";
import { invokeCommand } from "@relay/protocol-types";
import { Banner, Button, Section, StatusDot, Toggle, UpdateAbout, useRelayEvent } from "@relay/ui";
import { useEffect, useState } from "react";
import type { TabProps } from "./ConnectionTab";
import styles from "./settings.module.css";
import { errorMessage } from "../shared/errors";

const STATE_LABEL: Record<PermissionState, string> = { Granted: "Concedida", Denied: "Negada", Unknown: "Desconhecida" };
const PERMS: ReadonlyArray<{ kind: PermissionKind; key: "accessibility" | "input_monitoring"; label: string }> = [
  { kind: "Accessibility", key: "accessibility", label: "Acessibilidade" },
  { kind: "InputMonitoring", key: "input_monitoring", label: "Monitoramento de entrada" },
];

export function GeneralTab({ config, update }: TabProps) {
  const [perms, setPerms] = useState<Record<string, PermissionState>>({});
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
      setError(errorMessage(e));
    }
  };

  return (
    <>
      {error ? <Banner tone="danger">{error}</Banner> : null}
      <Section title="Início" grouped>
        <Toggle label="Iniciar no login" checked={config.general.launch_at_login} onChange={setLogin} />
      </Section>
      <UpdateAbout />
      <Section title="Permissões">
        {PERMS.map((p) => {
          const state = perms[p.key] ?? "Unknown";
          return (
            <div key={p.kind} className={styles.row}>
              <span className={styles.rowMain}>
                <StatusDot tone={state === "Granted" ? "ok" : state === "Denied" ? "danger" : "warn"} label={STATE_LABEL[state]} /> {p.label}:{" "}
                {STATE_LABEL[state]}
              </span>
              <Button onClick={() => invokeCommand("open_permission_settings", { kind: p.kind }).catch((e) => setError(errorMessage(e)))}>
                Abrir Ajustes
              </Button>
            </div>
          );
        })}
        <Button variant="ghost" onClick={() => invokeCommand("check_permissions").then((p) => setPerms({ ...p }), (e) => setError(errorMessage(e)))}>
          Verificar de novo
        </Button>
      </Section>
    </>
  );
}
