import { Button } from "../Button";
import { Section } from "../Section";
import { useUpdate } from "./context";
import styles from "./updater.module.css";
import type { UpdaterState } from "./useUpdater";

function statusText(state: UpdaterState, lastCheck: number | null): string {
  switch (state.status) {
    case "nao-suportado":
      return "Atualizações automáticas funcionam só no app instalado.";
    case "verificando":
      return "Procurando atualizações…";
    case "atualizado":
      return "Você está na versão mais recente.";
    case "disponivel":
      return `Versão ${state.update.version} disponível.`;
    case "baixando":
      return `Baixando a versão ${state.update.version}…`;
    case "reiniciando":
      return "Reiniciando para concluir a atualização…";
    case "erro":
      return state.mensagem;
    case "ocioso":
      return lastCheck
        ? `Última verificação às ${new Date(lastCheck).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })}.`
        : "O app procura atualizações sozinho ao abrir e a cada 6 horas.";
  }
}

/** Versão instalada e atualização manual; a automática roda em segundo plano. */
export function UpdateAbout() {
  const { state, version, lastCheck, verify, openModal } = useUpdate();
  const hasUpdate = "update" in state && state.update;
  const checking = state.status === "verificando";

  return (
    <Section title="Sobre" grouped>
      <div>
        <div className={styles.aboutRow}>
          <span>
            Versão instalada <strong className={styles.mono}>{version || "—"}</strong>
          </span>
          {hasUpdate ? (
            <Button variant="primary" onClick={openModal}>Ver atualização</Button>
          ) : (
            <Button disabled={state.status === "nao-suportado" || checking} onClick={() => void verify(true)}>
              {checking ? "Procurando…" : "Procurar atualizações"}
            </Button>
          )}
        </div>
        <p role={state.status === "erro" ? "alert" : "status"} className={state.status === "erro" ? styles.error : styles.hint}>
          {statusText(state, lastCheck)}
        </p>
      </div>
    </Section>
  );
}
