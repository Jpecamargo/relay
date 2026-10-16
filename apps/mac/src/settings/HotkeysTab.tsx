// dono: U3. Atalhos globais: um gravador por HotkeyAction, com conflito entre ações.
import { Section } from "@relay/ui";
import type { TabProps } from "./ConnectionTab";
import { HotkeyRecorder } from "./HotkeyRecorder";
import { actionKey, HOTKEY_ACTIONS, setHotkey } from "./model";
import styles from "./settings.module.css";

export function HotkeysTab({ config, update, errors }: TabProps) {
  return (
    <Section title="Atalhos globais" grouped>
      {HOTKEY_ACTIONS.map(({ action, label }) => {
        const k = actionKey(action);
        const current = config.hotkeys.find((b) => actionKey(b.action) === k)?.combo ?? null;
        return (
          <div key={k} className={styles.row}>
            <span className={styles.rowMain}>{label}</span>
            <HotkeyRecorder
              label={label}
              value={current}
              error={errors[`hotkey_${k}`]}
              onChange={(combo) => update((c) => ({ ...c, hotkeys: setHotkey(c.hotkeys, action, combo) }))}
            />
          </div>
        );
      })}
    </Section>
  );
}
