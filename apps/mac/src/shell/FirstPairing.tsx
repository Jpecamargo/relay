// Primeiro uso (nenhum computador pareado): a janela é só o pareamento, nos dois modos.
import { Button, Icon } from "@relay/ui";
import { useEffect, useState, type FormEvent } from "react";
import { useSettings } from "../settings";
import { PairingModes } from "../shared/pairing/PairingModes";
import { usePairing } from "../shared/pairing/usePairing";
import styles from "./firstPairing.module.css";

const SUCCESS_MS = 2200;

export interface FirstPairingProps {
  /** Computador recém-pareado (`null` enquanto o pareamento ainda não aconteceu). */
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
          <p className={styles.lead}>{peerName} está conectado.</p>
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
        <h2 className={styles.title}>Conecte outro computador</h2>
        <ol className={styles.steps}>
          <li>Abra o Relay no outro computador.</li>
          <li>Lá, escolha "Mostrar código" e digite aqui o código de 6 dígitos, ou mostre o código aqui e digite lá.</li>
        </ol>

        <div className={styles.modes}>
        <PairingModes
          pairing={p} size="lg" autoFocus
          emptyText="Procurando computadores na rede… Deixe o Relay aberto no outro computador."
        />
        </div>
        <p className={p.error ? styles.error : styles.status} role={p.error ? "alert" : "status"}>
          {p.error || (p.busy ? "Conectando…" : "")}
        </p>

        {manualOpen ? (
          <form className={styles.manual} onSubmit={submitManual}>
            <input
              className={styles.input}
              aria-label="IP do outro computador"
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
