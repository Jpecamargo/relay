import type { ReactNode } from "react";
import styles from "./Banner.module.css";
import { Icon } from "./Icon";

export interface BannerProps {
  tone?: "info" | "warn" | "danger";
  children: ReactNode;
  /** Ação única à direita ("Abrir Ajustes"). */
  action?: ReactNode;
  /** `line`: sem fundo, para painéis compactos. */
  variant?: "soft" | "line";
}

export function Banner({ tone = "info", children, action, variant = "soft" }: BannerProps) {
  return (
    <div
      role={tone === "danger" ? "alert" : "status"}
      className={`${styles.banner} ${styles[tone]} ${variant === "line" ? styles.line : ""}`}
    >
      <span className={styles.icon}>
        <Icon name={tone === "info" ? "info" : "alert"} />
      </span>
      <span className={styles.text}>{children}</span>
      {action}
    </div>
  );
}
