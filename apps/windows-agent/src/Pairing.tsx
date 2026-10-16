import { invokeCommand, type PairingEvent } from "@relay/protocol-types";
import { Banner, Button, ConfirmDialog, Section, useRelayEvent } from "@relay/ui";
import { useCallback, useEffect, useState } from "react";
import { useCountdown } from "./useCountdown";

export interface PairingProps {
  /** Nome do peer pareado hoje (`null` se nenhum). */
  peerName: string | null;
}

function format(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export function Pairing({ peerName }: PairingProps) {
  const [code, setCode] = useState<string | null>(null);
  const [expiresAt, setExpiresAt] = useState<number | null>(null);
  const [error, setError] = useState<string>();
  const [paired, setPaired] = useState(false);
  const [confirmRevoke, setConfirmRevoke] = useState(false);
  const remaining = useCountdown(expiresAt);
  const expired = code !== null && remaining === 0;

  const generate = useCallback(async () => {
    setError(undefined);
    setPaired(false);
    try {
      const r = await invokeCommand("agent_new_pairing_code");
      setCode(r.code);
      setExpiresAt(r.expires_at);
    } catch (e) {
      setCode(null);
      setExpiresAt(null);
      setError(String(e));
    }
  }, []);

  // A bandeja só traz a janela à frente: quem gera o código ao abrir é esta página.
  useEffect(() => {
    void generate();
  }, [generate]);

  useRelayEvent("relay:pairing", (e: PairingEvent) => {
    if (e.state === "paired") {
      setPaired(true);
      setCode(null);
      setExpiresAt(null);
    } else if (e.state === "failed") {
      setError("O pareamento falhou. Gere um novo código.");
      setCode(null);
      setExpiresAt(null);
    } else if (e.code && e.expires_at) {
      setCode(e.code);
      setExpiresAt(e.expires_at);
    }
  });

  const revoke = async () => {
    setConfirmRevoke(false);
    try {
      await invokeCommand("agent_revoke_pair");
      setPaired(false);
      await generate();
    } catch (e) {
      setError(String(e));
    }
  };

  const hasPeer = paired || peerName !== null;

  return (
    <Section
      title="Pareamento"
      actions={
        hasPeer ? (
          <Button variant="danger" onClick={() => setConfirmRevoke(true)}>
            Revogar
          </Button>
        ) : null
      }
    >
      {error ? <Banner tone="danger">{error}</Banner> : null}
      {paired ? <Banner tone="info">Pareado com sucesso.</Banner> : null}
      {hasPeer ? <p className="muted">Pareado com {peerName ?? "o Mac"}.</p> : null}
      {code !== null && !expired ? (
        <div className="code-box">
          <output className="code" aria-label="Código de pareamento">
            {code}
          </output>
          <span className="muted">
            Expira em <span aria-live="off">{format(remaining ?? 0)}</span>
          </span>
        </div>
      ) : (
        <p className="muted">{expired ? "O código expirou." : hasPeer ? "Para trocar de Mac, gere um novo código." : "Gere um código e digite-o no Mac."}</p>
      )}
      <Button variant={code !== null && !expired ? "default" : "primary"} onClick={() => void generate()}>
        {code === null || expired ? "Gerar código" : "Gerar novo código"}
      </Button>
      <ConfirmDialog
        open={confirmRevoke}
        destructive
        title="Revogar pareamento?"
        message="O Mac perde o acesso e precisará ser pareado de novo com um código."
        confirmLabel="Revogar"
        onConfirm={() => void revoke()}
        onCancel={() => setConfirmRevoke(false)}
      />
    </Section>
  );
}
