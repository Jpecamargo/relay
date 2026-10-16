// dono: U3. Regras de atalho, exceções por exe do Windows e cmd_tap_timeout_ms.
import { Button, NumberField, Section } from "@relay/ui";
import { RulesTable } from "./RulesTable";
import type { TabProps } from "./ConnectionTab";
import styles from "./settings.module.css";

export function KeyboardTab({ config, update, errors }: TabProps) {
  const km = config.keymap;
  const setKm = (patch: Partial<typeof km>) => update((c) => ({ ...c, keymap: { ...c.keymap, ...patch } }));

  return (
    <>
      <Section title="Toque do Cmd" grouped>
        <NumberField label="Tempo do toque do Cmd" unit="ms" min={1} value={km.cmd_tap_timeout_ms} onChange={(v) => setKm({ cmd_tap_timeout_ms: v })} />
        {errors.cmd_tap_timeout_ms ? <span role="alert" className={styles.error}>{errors.cmd_tap_timeout_ms}</span> : null}
      </Section>

      <Section title="Regras de atalho">
        <RulesTable scope="global" rules={km.rules} onChange={(rules) => setKm({ rules })} />
      </Section>

      <Section
        title="Exceções por app"
        actions={
          <Button onClick={() => setKm({ app_exceptions: [...km.app_exceptions, { exe: "", rules: [] }] })}>Adicionar exceção</Button>
        }
      >
        {km.app_exceptions.length === 0 ? <p className={styles.muted}>Nenhuma exceção. Use para dar regras próprias a um app do Windows.</p> : null}
        {km.app_exceptions.map((x, i) => {
          const setEx = (patch: Partial<typeof x>) =>
            setKm({ app_exceptions: km.app_exceptions.map((e, j) => (j === i ? { ...e, ...patch } : e)) });
          return (
            <details key={i} className={styles.exception} open={!x.exe}>
              <summary>{x.exe || "Novo app"}</summary>
              <div className={styles.inline}>
                <div className={styles.field}>
                  <label htmlFor={`exe-${i}`} className={styles.fieldLabel}>
                    Executável do Windows
                  </label>
                  <input id={`exe-${i}`} className={styles.input} placeholder="Code.exe" value={x.exe} onChange={(e) => setEx({ exe: e.target.value })} />
                  {errors[`exception_${i}`] ? <span role="alert" className={styles.error}>{errors[`exception_${i}`]}</span> : null}
                </div>
                <Button variant="danger" onClick={() => setKm({ app_exceptions: km.app_exceptions.filter((_, j) => j !== i) })}>
                  Remover exceção
                </Button>
              </div>
              <RulesTable scope={x.exe || `exceção ${i + 1}`} rules={x.rules} onChange={(rules) => setEx({ rules })} />
            </details>
          );
        })}
      </Section>
    </>
  );
}
