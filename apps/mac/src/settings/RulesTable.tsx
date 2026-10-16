// dono: U3. Tabela das regras de atalho (embutidas + próprias); usada no teclado e em cada exceção por app.
import type { KeymapRule } from "@relay/protocol-types";
import { Toggle } from "@relay/ui";
import { useState } from "react";
import { HotkeyRecorder } from "./HotkeyRecorder";
import { BUILTIN_RULES, patchRule } from "./model";
import styles from "./settings.module.css";

export interface RulesTableProps {
  rules: KeymapRule[];
  onChange: (rules: KeymapRule[]) => void;
  /** Prefixo do nome acessível, para diferenciar tabelas na mesma tela. */
  scope: string;
}

export function RulesTable({ rules, onChange, scope }: RulesTableProps) {
  const [draftFrom, setDraftFrom] = useState<KeymapRule["from"]>(null);
  const [draftTo, setDraftTo] = useState<KeymapRule["to"]>(null);
  const custom = rules.filter((r) => !BUILTIN_RULES.some((b) => b.id === r.id));

  const addCustom = () => {
    if (!draftFrom || !draftTo) return;
    const used = new Set(rules.map((r) => r.id));
    let n = 1;
    while (used.has(`custom_${n}`)) n++;
    onChange([...rules, { id: `custom_${n}`, enabled: true, from: draftFrom, to: draftTo }]);
    setDraftFrom(null);
    setDraftTo(null);
  };

  return (
    <table className={styles.table}>
      <caption className={styles.srOnly}>Regras de atalho {scope}</caption>
      <thead>
        <tr>
          <th scope="col">Mac</th>
          <th scope="col">Windows</th>
          <th scope="col">Ativa</th>
          <th scope="col">Sobrescrever</th>
        </tr>
      </thead>
      <tbody>
        {BUILTIN_RULES.map((b) => {
          const r = rules.find((x) => x.id === b.id);
          const label = `${scope} ${b.id}`;
          return (
            <tr key={b.id}>
              <td>
                <kbd className={styles.kbd}>{b.from}</kbd>
                {b.terminal ? <span className={styles.tag}>terminal</span> : null}
              </td>
              <td>{r?.to ? <span className={styles.override}>{b.to}</span> : b.to}</td>
              <td>
                <Toggle label={`Ativar ${label}`} checked={r?.enabled ?? true} onChange={(enabled) => onChange(patchRule(rules, b.id, { enabled }))} />
              </td>
              <td>
                <HotkeyRecorder
                  label={`sobrescrever ${label}`}
                  value={r?.to ?? null}
                  requireModifier={false}
                  onChange={(to) => onChange(patchRule(rules, b.id, { to }))}
                />
              </td>
            </tr>
          );
        })}
        {custom.map((r) => (
          <tr key={r.id}>
            <td>
              <HotkeyRecorder label={`origem ${scope} ${r.id}`} value={r.from} requireModifier={false} onChange={(from) => onChange(patchRule(rules, r.id, { from }))} />
            </td>
            <td>
              <HotkeyRecorder label={`destino ${scope} ${r.id}`} value={r.to} requireModifier={false} onChange={(to) => onChange(patchRule(rules, r.id, { to }))} />
            </td>
            <td>
              <Toggle label={`Ativar ${scope} ${r.id}`} checked={r.enabled} onChange={(enabled) => onChange(patchRule(rules, r.id, { enabled }))} />
            </td>
            <td>
              <button type="button" className={styles.linkBtn} onClick={() => onChange(rules.filter((x) => x.id !== r.id))}>
                Remover regra
              </button>
            </td>
          </tr>
        ))}
        <tr>
          <td>
            <HotkeyRecorder label={`nova regra, origem (${scope})`} value={draftFrom} requireModifier={false} onChange={setDraftFrom} />
          </td>
          <td>
            <HotkeyRecorder label={`nova regra, destino (${scope})`} value={draftTo} requireModifier={false} onChange={setDraftTo} />
          </td>
          <td colSpan={2}>
            <button type="button" className={styles.linkBtn} disabled={!draftFrom || !draftTo} onClick={addCustom}>
              Adicionar regra
            </button>
          </td>
        </tr>
      </tbody>
    </table>
  );
}
