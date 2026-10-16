import type { ReactNode } from "react";
import { Icon, type IconName } from "./Icon";
import styles from "./MenuRow.module.css";

export interface MenuRowProps {
  label: string;
  icon?: IconName;
  /** Atalho mostrado à direita ("⌘,"). */
  shortcut?: string;
  /** Seta de submenu; `open` gira para baixo. */
  chevron?: boolean | "open";
  onClick?: () => void;
  disabled?: boolean;
  danger?: boolean;
  /** Vira linha de alternância: o estado aparece como switch discreto à direita. */
  checked?: boolean;
  /** Conteúdo livre à direita (ex.: valor). */
  trailing?: ReactNode;
  /** Item de escolha única: marca com check quando `true` (e `false` quando outro está marcado). */
  selected?: boolean;
  /** Recuo de item de submenu. */
  nested?: boolean;
}

/** Linha de menu do macOS: ícone + texto + atalho/switch/chevron, realce arredondado no hover. */
export function MenuRow({ label, icon, shortcut, chevron, onClick, disabled, danger, checked, trailing, nested, selected }: MenuRowProps) {
  const isSwitch = checked !== undefined;
  const isChoice = selected !== undefined;
  return (
    <button
      type="button"
      role={isSwitch ? "switch" : isChoice ? "menuitemradio" : undefined}
      aria-checked={isSwitch ? checked : isChoice ? selected : undefined}
      aria-expanded={chevron ? chevron === "open" : undefined}
      disabled={disabled}
      className={[styles.row, danger ? styles.danger : "", nested ? styles.nested : ""].filter(Boolean).join(" ")}
      onClick={onClick}
    >
      <span className={styles.icon}>{icon ? <Icon name={icon} /> : null}</span>
      <span className={styles.label}>{label}</span>
      {trailing}
      {shortcut ? <span className={styles.shortcut}>{shortcut}</span> : null}
      {chevron ? (
        <span className={styles.chevron}>
          <Icon name={chevron === "open" ? "chevronDown" : "chevronRight"} size={13} />
        </span>
      ) : null}
      {selected ? (
        <span className={styles.check}>
          <Icon name="check" size={13} />
        </span>
      ) : null}
      {isSwitch ? (
        <span className={styles.switch} data-on={checked} aria-hidden="true">
          <span className={styles.knob} />
        </span>
      ) : null}
    </button>
  );
}
