// Modo "Mostrar código": esta máquina gera o código, escuta como Listener (120 s) e o outro computador o digita.
import { invokeCommand, type PairingEvent } from "@relay/protocol-types";
import { Banner, Button, useRelayEvent } from "@relay/ui";
import { useCallback, useEffect, useState } from "react";
import { errorMessage } from "../errors";
import pairing from "./pairing.module.css";
import { useCountdown } from "./useCountdown";

const format = (seconds: number) => `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;

export function ShowCode({ size = "md" }: { size?: "md" | "lg" }) {
  const [code, setCode] = useState<string | null>(null);
  const [expiresAt, setExpiresAt] = useState<number | null>(null);
  const [error, setError] = useState("");
  const remaining = useCountdown(expiresAt);
  const expired = code !== null && remaining === 0;
  const showing = code !== null && !expired;

  const clear = () => {
    setCode(null);
    setExpiresAt(null);
  };

  const generate = useCallback(async () => {
    setError("");
    try {
      const r = await invokeCommand("new_pairing_code");
      setCode(r.code);
      setExpiresAt(r.expires_at);
    } catch (e) {
      setCode(null);
      setExpiresAt(null);
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    void generate();
  }, [generate]);

  useRelayEvent("relay:pairing", (e: PairingEvent) => {
    if (e.state === "paired") {
      setError("");
      clear();
    } else if (e.state === "failed") {
      setError("O pareamento falhou. Gere um novo código.");
      clear();
    } else if (e.code && e.expires_at) {
      setCode(e.code);
      setExpiresAt(e.expires_at);
    }
  });

  return (
    <div className={`${pairing.show} ${size === "lg" ? pairing.showLg : ""}`}>
      {error ? <Banner tone="danger">{error}</Banner> : null}
      {showing ? (
        <>
          <output className={pairing.shownCode} aria-label="Código de pareamento">
            {code.split("").map((d, i) => (
              <span key={i} className={pairing.digit}>{d}</span>
            ))}
          </output>
          <p className={pairing.expiry}>
            Expira em <span className={remaining !== null && remaining <= 30 ? pairing.soon : undefined}>{format(remaining ?? 0)}</span>
          </p>
        </>
      ) : (
        <p className={pairing.empty}>{expired ? "O código expirou." : "Gerando o código…"}</p>
      )}
      <Button variant={showing ? "default" : "primary"} onClick={() => void generate()}>
        {showing ? "Gerar novo código" : "Gerar código"}
      </Button>
    </div>
  );
}
