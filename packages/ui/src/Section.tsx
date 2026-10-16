import type { ReactNode } from "react";
import styles from "./Section.module.css";

export interface SectionProps {
  title: string;
  children: ReactNode;
  /** Controles alinhados à direita do título. */
  actions?: ReactNode;
  /** Agrupa as linhas num bloco arredondado (use só quando os filhos são linhas uniformes). */
  grouped?: boolean;
}

export function Section({ title, children, actions, grouped }: SectionProps) {
  return (
    <section className={`${styles.section} ${grouped ? styles.grouped : ""}`}>
      <header className={styles.header}>
        <h2 className={styles.title}>{title}</h2>
        {actions}
      </header>
      <div className={styles.body}>{children}</div>
    </section>
  );
}
