import { useEffect, useState } from "react";

/** Segundos inteiros restantes até `expiresAt` (epoch ms); `null` sem prazo. Nunca fica negativo. */
export function useCountdown(expiresAt: number | null): number | null {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (expiresAt === null) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(id);
  }, [expiresAt]);
  return expiresAt === null ? null : Math.max(0, Math.ceil((expiresAt - now) / 1000));
}
