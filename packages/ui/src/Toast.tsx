import { useEffect } from "react";
import styles from "./Toast.module.css";

export interface ToastProps {
  message: string;
  tone?: "info" | "ok" | "danger";
  onDismiss: () => void;
  /** Fecha sozinho depois de N ms (padrão 4000). 0 desliga. */
  durationMs?: number;
}

export function Toast({ message, tone = "info", onDismiss, durationMs = 4000 }: ToastProps) {
  useEffect(() => {
    if (durationMs <= 0) return;
    const t = setTimeout(onDismiss, durationMs);
    return () => clearTimeout(t);
  }, [durationMs, onDismiss, message]);

  return (
    <div role={tone === "danger" ? "alert" : "status"} className={`${styles.toast} ${styles[tone]}`}>
      <span>{message}</span>
      <button type="button" className={styles.close} aria-label="Fechar" onClick={onDismiss}>
        ×
      </button>
    </div>
  );
}
