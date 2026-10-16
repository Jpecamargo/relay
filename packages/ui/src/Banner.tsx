import type { ReactNode } from "react";
import styles from "./Banner.module.css";

export interface BannerProps {
  tone?: "info" | "warn" | "danger";
  children: ReactNode;
  /** Ação única à direita ("Abrir Ajustes"). */
  action?: ReactNode;
}

export function Banner({ tone = "info", children, action }: BannerProps) {
  return (
    <div role={tone === "danger" ? "alert" : "status"} className={`${styles.banner} ${styles[tone]}`}>
      <span className={styles.text}>{children}</span>
      {action}
    </div>
  );
}
