import { useId } from "react";
import { Button } from "../Button";
import { ProgressBar } from "../ProgressBar";
import { useUpdate } from "./context";
import styles from "./updater.module.css";
import { updateOf } from "./useUpdater";

const megabytes = (bytes: number) => `${(bytes / 1_048_576).toFixed(1).replace(".", ",")} MB`;

/** O plugin devolve a data como "2026-09-15 19:00:00.0 +00:00:00"; só o dia interessa. */
function publishedOn(date?: string): string | null {
  const day = date?.match(/^(\d{4})-(\d{2})-(\d{2})/);
  return day ? `${day[3]}/${day[2]}/${day[1]}` : null;
}

/** Nova versão: o que mudou, aviso de reinício e o andamento do download. */
export function UpdateModal() {
  const { state, version, install, closeModal, matchPeer } = useUpdate();
  const titleId = useId();
  const update = updateOf(state);
  if (!update) return null;

  const working = state.status === "baixando" || state.status === "reiniciando";
  const published = publishedOn(update.date);
  const notes = update.body?.trim();

  return (
    <div className={styles.overlay} onKeyDown={(e) => e.key === "Escape" && !working && closeModal()}>
      <div role="dialog" aria-modal="true" aria-labelledby={titleId} className={styles.dialog}>
        <h2 id={titleId} className={styles.title}>Nova versão do Relay</h2>
        <p className={styles.sub}>
          {version ? <>Você está na <strong>{version}</strong> · disponível: </> : null}
          <strong>{update.version}</strong>
          {published ? ` · publicada em ${published}` : ""}
        </p>
        {matchPeer ? (
          <p className={styles.sub}>
            O {matchPeer.name} já está na versão <strong>{matchPeer.version}</strong>. Atualize para os dois ficarem na mesma versão.
          </p>
        ) : null}

        {notes ? (
          <section>
            <h3 className={styles.heading}>O que mudou</h3>
            <div className={styles.notes}>{notes}</div>
          </section>
        ) : (
          <p className={styles.hint}>Sem notas de versão.</p>
        )}

        <p className={styles.hint}>O app reinicia para concluir a atualização. Suas configurações ficam salvas neste computador.</p>

        {state.status === "baixando" ? (
          <div aria-live="polite" className={styles.progress}>
            <ProgressBar label="Progresso do download" value={state.total ? state.baixado : 0} max={state.total ?? 1} />
            <span className={styles.hint}>
              Baixando {megabytes(state.baixado)}
              {state.total ? ` de ${megabytes(state.total)}` : ""}…
            </span>
          </div>
        ) : null}
        {state.status === "reiniciando" ? <p role="status" className={styles.hint}>Instalado. Reiniciando…</p> : null}
        {state.status === "erro" ? <p role="alert" className={styles.error}>{state.mensagem}</p> : null}

        <div className={styles.actions}>
          {!working ? <Button onClick={closeModal}>Depois</Button> : null}
          <Button variant="primary" disabled={working} onClick={() => void install(update)}>
            {state.status === "erro" ? "Tentar de novo" : working ? "Atualizando…" : "Atualizar e reiniciar"}
          </Button>
        </div>
      </div>
    </div>
  );
}
