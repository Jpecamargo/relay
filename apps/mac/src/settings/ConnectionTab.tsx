// dono: U3. Conexão e pareamento. Os comandos aqui valem na hora, sem passar pelo "Salvar".
import type { Config } from "@relay/protocol-types";
import { Banner, Button, ConfirmDialog, Section, StatusDot } from "@relay/ui";
import { useState } from "react";
import { CodeInput } from "../shared/pairing/CodeInput";
import { PeerList } from "../shared/pairing/PeerList";
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

      <Section title="PC pareado">
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
          <p className={styles.muted}>Nenhum PC pareado.</p>
        )}
      </Section>

      <Section
        title="Parear com um PC"
        actions={
          <Button variant="ghost" onClick={p.refresh}>
            Atualizar
          </Button>
        }
      >
        <PeerList peers={p.peers} selectedId={p.target?.device_id ?? null} onSelect={p.select} emptyText="Nenhum PC encontrado na rede." />
        <div className={styles.field}>
          <span className={styles.fieldLabel}>Código do agente do Windows</span>
          <CodeInput value={p.code} onChange={p.changeCode} onComplete={(c) => void p.pair(c)} disabled={p.busy} invalid={p.error !== ""} />
        </div>
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
        message={`${p.peerName ?? "O PC"} deixará de controlar e de ser controlado. Será preciso parear de novo.`}
        confirmLabel="Revogar"
        onConfirm={unpair}
        onCancel={() => setConfirming(false)}
      />
    </>
  );
}
