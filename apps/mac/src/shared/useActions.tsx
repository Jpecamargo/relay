import type { CommandResult } from "@relay/protocol-types";
import { Toast } from "@relay/ui";
import { useCallback, useRef, useState, type ReactNode } from "react";
import { errorText, isCommandResult } from "./errors";
import styles from "./shared.module.css";

/**
 * Executa uma ação; só o erro vira toast. Sucesso é silencioso, salvo `okMessage` (para o que o
 * usuário não vê acontecer, como "Captura copiada para o clipboard").
 */
export type Act = <T>(label: string, fn: () => Promise<T>, okMessage?: string) => Promise<T | undefined>;

interface ToastState { message: string; tone: "ok" | "danger"; n: number }

export function useActions(): {
  act: Act;
  show: (message: string, tone: "ok" | "danger") => void;
  firstTime: (r: CommandResult) => boolean;
  toast: ReactNode;
} {
  const [toast, setToast] = useState<ToastState>();
  const toastN = useRef(0);
  const show = useCallback(
    (message: string, tone: "ok" | "danger") => setToast({ message, tone, n: ++toastN.current }),
    [],
  );

  // Resultados já mostrados (invoke e relay:command-result chegam em qualquer ordem). Id 0 não
  // identifica comando: nesse caso a chave é o erro, com validade curta.
  const seen = useRef(new Map<string, number>());
  const firstTime = useCallback((r: CommandResult) => {
    const k = r.id !== 0 ? `id:${r.id}` : `0:${r.error ?? ""}`;
    const now = Date.now();
    const at = seen.current.get(k);
    seen.current.set(k, now);
    for (const [key, t] of seen.current) if (now - t > 5000) seen.current.delete(key);
    return at === undefined || (r.id === 0 && now - at > 1500);
  }, []);

  const act: Act = useCallback(
    async (label, fn, okMessage) => {
      try {
        const r = await fn();
        if (isCommandResult(r) && !firstTime(r)) return r;
        if (isCommandResult(r) && !r.ok) {
          show(`${label}: ${errorText(r.error ?? "falhou")}`, "danger");
          return r;
        }
        if (okMessage) show(okMessage, "ok");
        return r;
      } catch (e) {
        show(`${label}: ${errorText(e)}`, "danger");
        return undefined;
      }
    },
    [firstTime, show],
  );

  const node = toast ? (
    <div className={styles.toast}>
      <Toast key={toast.n} message={toast.message} tone={toast.tone} onDismiss={() => setToast(undefined)} />
    </div>
  ) : null;
  return { act, show, firstTime, toast: node };
}
