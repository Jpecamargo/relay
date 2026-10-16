// dono: U3. Conexão e pareamento. Os comandos aqui valem na hora, sem passar pelo "Salvar".
import type { Config, DiscoveredPeer } from "@relay/protocol-types";
import { invokeCommand } from "@relay/protocol-types";
import { Banner, Button, ConfirmDialog, Section, StatusDot, useRelayEvent } from "@relay/ui";
import { useCallback, useEffect, useState } from "react";
import { CodeInput } from "./CodeInput";
import { isValidPairCode } from "./model";
import styles from "./settings.module.css";
import { errorMessage } from "../shared/errors";


export interface TabProps {
  config: Config;
  update: (fn: (c: Config) => Config) => void;
  errors: Record<string, string>;
}

export function ConnectionTab({ config, update }: TabProps) {
  const [peers, setPeers] = useState<DiscoveredPeer[]>([]);
  const [peerName, setPeerName] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [target, setTarget] = useState<DiscoveredPeer | null>(null);
  const [manual, setManual] = useState(config.network.manual_address ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [paired, setPaired] = useState(false);
  const [confirming, setConfirming] = useState(false);

  const refresh = useCallback(() => {
    invokeCommand("list_discovered").then(setPeers, (e) => setError(errorMessage(e)));
    invokeCommand("get_status").then((s) => setPeerName(s.peer_name), () => undefined);
  }, []);
  useEffect(refresh, [refresh]);

  useRelayEvent("relay:status", (s) => setPeerName(s.peer_name));
  useRelayEvent("relay:pairing", (p) => {
    if (p.state === "paired") {
      setPaired(true);
      setCode("");
      refresh();
    } else if (p.state === "failed") setError("Pareamento recusado: confira o código e tente de novo.");
  });

  const pair = async () => {
    setBusy(true);
    setError("");
    try {
      const address = target ? `${target.addrs[0]}:${target.port}` : undefined;
      await invokeCommand("pair_with_code", address ? { code, address } : { code });
      setPaired(true);
      setCode("");
      refresh();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const applyManual = async () => {
    const address = manual.trim() || undefined;
    setError("");
    try {
      await invokeCommand("set_manual_address", address ? { address } : {});
      update((c) => ({ ...c, network: { ...c.network, manual_address: address ?? null } }));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const unpair = async () => {
    setConfirming(false);
    try {
      await invokeCommand("unpair");
      setPaired(false);
      setPeerName(null);
      refresh();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <>
      {error ? <Banner tone="danger">{error}</Banner> : null}
      {paired && !error ? <Banner tone="info">Pareamento concluído.</Banner> : null}

      <Section title="PC pareado">
        {peerName ? (
          <div className={styles.row}>
            <span className={styles.rowMain}>
              <StatusDot tone="ok" label="Pareado" /> {peerName}
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
          <Button variant="ghost" onClick={refresh}>
            Atualizar
          </Button>
        }
      >
        <ul className={styles.list} aria-label="PCs encontrados">
          {peers.length === 0 ? <li className={styles.muted}>Nenhum PC encontrado na rede.</li> : null}
          {peers.map((p) => (
            <li key={p.device_id} className={styles.row}>
              <span className={styles.rowMain}>
                {p.name} <span className={styles.muted}>{p.addrs[0]}</span>
              </span>
              <Button
                variant={target?.device_id === p.device_id ? "primary" : "default"}
                aria-pressed={target?.device_id === p.device_id}
                onClick={() => setTarget(target?.device_id === p.device_id ? null : p)}
              >
                {target?.device_id === p.device_id ? "Selecionado" : "Selecionar"}
              </Button>
            </li>
          ))}
        </ul>
        <div className={styles.inline}>
          <CodeInput value={code} onChange={setCode} disabled={busy} />
          <Button variant="primary" disabled={!isValidPairCode(code) || busy} onClick={pair}>
            Parear
          </Button>
        </div>
        <p className={styles.hint}>O código aparece no agente do Windows.</p>
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
              value={manual}
              placeholder="192.168.0.20"
              onChange={(e) => setManual(e.target.value)}
            />
          </div>
          <Button onClick={applyManual}>Aplicar</Button>
        </div>
      </Section>

      <ConfirmDialog
        open={confirming}
        destructive
        title="Revogar o pareamento?"
        message={`${peerName ?? "O PC"} deixará de controlar e de ser controlado. Será preciso parear de novo.`}
        confirmLabel="Revogar"
        onConfirm={unpair}
        onCancel={() => setConfirming(false)}
      />
    </>
  );
}
