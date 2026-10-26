// dono: U3. Conexão e pareamento. Os comandos aqui valem na hora, sem passar pelo "Salvar".
import type { Config } from "@relay/protocol-types";
import { Banner, Button, ConfirmDialog, Section, StatusDot } from "@relay/ui";
import { useState } from "react";
import { PairingModes } from "../shared/pairing/PairingModes";
import { usePairing } from "../shared/pairing/usePairing";
import styles from "./settings.module.css";

export interface TabProps {
  config: Config;
  update: (fn: (c: Config) => Config) => void;
  errors: Record<string, string>;
}

export function ConnectionTab({ config, update }: TabProps) {
  const [confirming, setConfirming] = useState(false);
  const p = usePairing({
    manualAddress: config.network.manual_address,
    onManualApplied: (address) => update((c) => ({ ...c, network: { ...c.network, manual_address: address } })),
  });

  const unpair = async () => {
    setConfirming(false);
    await p.unpair();
  };

  return (
    <>
      {p.error ? <Banner tone="danger">{p.error}</Banner> : null}
      {p.paired && !p.error ? <Banner tone="info">Pareamento concluído.</Banner> : null}

      <Section title="Computador pareado">
        {p.peerName ? (
          <div className={styles.row}>
            <span className={styles.rowMain}>
              <StatusDot tone="ok" label="Pareado" /> {p.peerName}
            </span>
            <Button variant="danger" onClick={() => setConfirming(true)}>
              Revogar pareamento
            </Button>
          </div>
        ) : (
          <p className={styles.muted}>Nenhum computador pareado.</p>
        )}
      </Section>

      <Section
        title="Parear com outro computador"
        actions={
          <Button variant="ghost" onClick={p.refresh}>
            Atualizar
          </Button>
        }
      >
        <PairingModes pairing={p} />
      </Section>

      <Section title="Endereço manual">
        <div className={styles.inline}>
          <div className={styles.field}>
            <label htmlFor="manual-address" className={styles.fieldLabel}>
              IP, IP:porta ou hostname
            </label>
            <input
              id="manual-address"
              className={styles.input}
              value={p.manual}
              placeholder="192.168.0.20"
              onChange={(e) => p.setManual(e.target.value)}
            />
          </div>
          <Button onClick={() => void p.applyManual()}>Aplicar</Button>
        </div>
      </Section>

      <ConfirmDialog
        open={confirming}
        destructive
        title="Revogar o pareamento?"
        message={`${p.peerName ?? "O outro computador"} deixará de controlar e de ser controlado. Será preciso parear de novo.`}
        confirmLabel="Revogar"
        onConfirm={unpair}
        onCancel={() => setConfirming(false)}
      />
    </>
  );
}
