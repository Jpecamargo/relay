import styles from "./StatusDot.module.css";

export type StatusTone = "ok" | "warn" | "danger" | "idle";

export interface StatusDotProps {
  tone: StatusTone;
  /** Texto lido por leitor de tela e mostrado como dica. */
  label: string;
}

export function StatusDot({ tone, label }: StatusDotProps) {
  return <span role="img" aria-label={label} title={label} className={`${styles.dot} ${styles[tone]}`} />;
}
