// Primeiro uso (nenhum PC pareado): a única coisa na janela é digitar o código que o agente mostra.
import { Button, Icon } from "@relay/ui";
import { useEffect, useState, type FormEvent } from "react";
import { useSettings } from "../settings";
import { CodeInput } from "../shared/pairing/CodeInput";
import { PeerList } from "../shared/pairing/PeerList";
import { usePairing } from "../shared/pairing/usePairing";
import styles from "./firstPairing.module.css";

const SUCCESS_MS = 2200;

export interface FirstPairingProps {
  /** PC recém-pareado (`null` enquanto o pareamento ainda não aconteceu). */
  peerName: string | null;
  onDone: () => void;
}

export function FirstPairing({ peerName, onDone }: FirstPairingProps) {
  const { config, update } = useSettings();
  const p = usePairing({
    manualAddress: config?.network.manual_address ?? null,
    onManualApplied: (address) => update((c) => ({ ...c, network: { ...c.network, manual_address: address } })),
    poll: true,
  });
  const [manualOpen, setManualOpen] = useState(false);

  useEffect(() => {
    if (peerName === null) return;
    const id = setTimeout(onDone, SUCCESS_MS);
    return () => clearTimeout(id);
  }, [peerName, onDone]);

  if (peerName !== null) {
    return (
      <div className={styles.page}>
        <div className={styles.column}>
          <h2 className={styles.title}>
            <Icon name="check" size={22} className={styles.done} /> Conectado
          </h2>
          <p className={styles.lead}>{peerName} já pode ser controlado com este teclado e trackpad.</p>
          <Button variant="primary" onClick={onDone}>
            Começar
          </Button>
        </div>
      </div>
    );
  }

  const submitManual = (e: FormEvent) => {
    e.preventDefault();
    void p.applyManual();
  };

  return (
    <div className={styles.page}>
      <div className={styles.column}>
        <h2 className={styles.title}>Conecte seu PC</h2>
        <ol className={styles.steps}>
          <li>Instale o Relay Agent no PC.</li>
          <li>Abra o agente: o código de 6 dígitos aparece na janela.</li>
          <li>Digite o código aqui.</li>
        </ol>

        <PeerList
          peers={p.peers}
          selectedId={p.target?.device_id ?? null}
          onSelect={p.select}
          emptyText="Procurando PCs na rede… Deixe o agente aberto no PC."
        />

        <CodeInput size="lg" autoFocus value={p.code} onChange={p.changeCode} onComplete={(c) => void p.pair(c)} disabled={p.busy} invalid={p.error !== ""} />
        <p className={p.error ? styles.error : styles.status} role={p.error ? "alert" : "status"}>
          {p.error || (p.busy ? "Conectando…" : "")}
        </p>

        {manualOpen ? (
          <form className={styles.manual} onSubmit={submitManual}>
            <input
              className={styles.input}
              aria-label="IP do PC"
              autoFocus
              placeholder="192.168.0.20"
              value={p.manual}
              onChange={(e) => p.setManual(e.target.value)}
            />
            <Button type="submit">Usar</Button>
          </form>
        ) : (
          <button type="button" className={styles.link} onClick={() => setManualOpen(true)}>
            Não aparece? Informe o IP
          </button>
        )}
      </div>
    </div>
  );
}
