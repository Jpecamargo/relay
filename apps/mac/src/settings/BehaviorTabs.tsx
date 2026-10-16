// dono: U3. Abas Transição, Clipboard e Áudio.
import type { ClipboardFormat, ClipboardMode, Modifier, PcAudioMode } from "@relay/protocol-types";
import { NumberField, Section, Select, Toggle } from "@relay/ui";
import type { TabProps } from "./ConnectionTab";
import styles from "./settings.module.css";

const Err = ({ text }: { text?: string }) => (text ? <span role="alert" className={styles.error}>{text}</span> : null);

type Resistance = "none" | "light" | "medium";
const RESISTANCE_PT: Record<Resistance, number> = { none: 0, light: 8, medium: 20 };
const resistanceOf = (pt: number): Resistance => (pt <= 0 ? "none" : pt <= 12 ? "light" : "medium");

export function TransitionTab({ config, update, errors }: TabProps) {
  const t = config.transition;
  const set = (patch: Partial<typeof t>) => update((c) => ({ ...c, transition: { ...c.transition, ...patch } }));
  return (
    <Section title="Troca de máquina" grouped>
      <Select<Resistance>
        label="Resistência da borda"
        value={resistanceOf(t.edge_resistance_pt)}
        onChange={(v) => set({ edge_resistance_pt: RESISTANCE_PT[v], dwell_ms: 0 })}
        options={[
          { value: "none", label: "Nenhuma" },
          { value: "light", label: "Leve" },
          { value: "medium", label: "Média" },
        ]}
      />
      <Select<Modifier | "none">
        label="Modificador exigido"
        value={t.require_modifier ?? "none"}
        onChange={(v) => set({ require_modifier: v === "none" ? null : v })}
        options={[
          { value: "none", label: "Nenhum" },
          { value: "Ctrl", label: "Control" },
          { value: "Option", label: "Option" },
          { value: "Shift", label: "Shift" },
          { value: "Cmd", label: "Command" },
        ]}
      />
      <NumberField label="Zona morta nos cantos" unit="mm" min={0} step={0.5} value={t.corner_dead_zone_mm} onChange={(v) => set({ corner_dead_zone_mm: v })} />
      <Err text={errors.corner_dead_zone_mm} />
    </Section>
  );
}

const FORMATS: ReadonlyArray<{ id: ClipboardFormat; label: string }> = [
  { id: "Text", label: "Texto" },
  { id: "Html", label: "HTML" },
  { id: "Rtf", label: "RTF" },
  { id: "Png", label: "Imagem (PNG)" },
  { id: "FileList", label: "Arquivos" },
];

export function ClipboardTab({ config, update, errors }: TabProps) {
  const cb = config.clipboard;
  const set = (patch: Partial<typeof cb>) => update((c) => ({ ...c, clipboard: { ...c.clipboard, ...patch } }));
  const MB = 1024 * 1024;
  return (
    <Section title="Clipboard compartilhado" grouped>
      <Toggle label="Sincronizar clipboard" checked={cb.enabled} onChange={(enabled) => set({ enabled })} />
      <Select<ClipboardMode>
        label="Quando sincronizar"
        value={cb.mode}
        onChange={(mode) => set({ mode })}
        options={[
          { value: "on_switch", label: "Ao trocar de máquina" },
          { value: "on_copy", label: "A cada cópia" },
        ]}
      />
      <NumberField label="Tamanho máximo" unit="MB" min={0.1} step={1} value={cb.max_bytes / MB} onChange={(v) => set({ max_bytes: Math.round(v * MB) })} />
      <Err text={errors.max_bytes} />
      <fieldset className={styles.fieldset}>
        <legend className={styles.fieldLabel}>Formatos</legend>
        {FORMATS.map((f) => (
          <Toggle
            key={f.id}
            label={f.label}
            checked={cb.formats.includes(f.id)}
            onChange={(on) => set({ formats: on ? [...cb.formats, f.id] : cb.formats.filter((x) => x !== f.id) })}
          />
        ))}
      </fieldset>
      <Err text={errors.formats} />
    </Section>
  );
}

export function AudioTab({ config, update, errors }: TabProps) {
  const a = config.audio;
  const set = (patch: Partial<typeof a>) => update((c) => ({ ...c, audio: { ...c.audio, ...patch } }));
  return (
    <Section title="Áudio do PC" grouped>
      <Select<PcAudioMode>
        label="Captura no PC"
        value={a.pc_mode}
        onChange={(pc_mode) => set({ pc_mode })}
        options={[
          { value: "system", label: "Sistema inteiro" },
          { value: "process", label: "Só processos listados" },
        ]}
      />
      {a.pc_mode === "process" ? (
        <div className={styles.field}>
          <label htmlFor="process-names" className={styles.fieldLabel}>
            Processos (um por linha)
          </label>
          <textarea
            id="process-names"
            className={styles.input}
            rows={3}
            value={a.process_names.join("\n")}
            onChange={(e) => set({ process_names: e.target.value.split("\n").map((s) => s.trim()).filter(Boolean) })}
          />
          <Err text={errors.process_names} />
        </div>
      ) : null}
      <div className={styles.field}>
        <label htmlFor="virtual-output" className={styles.fieldLabel}>
          Saída virtual (Mac para PC)
        </label>
        <input
          id="virtual-output"
          className={styles.input}
          placeholder="CABLE Input"
          value={a.virtual_output_name ?? ""}
          onChange={(e) => set({ virtual_output_name: e.target.value || null })}
        />
      </div>
      <NumberField label="Jitter alvo" unit="ms" min={1} value={a.jitter_target_ms} onChange={(v) => set({ jitter_target_ms: v })} />
      <Err text={errors.jitter_target_ms} />
    </Section>
  );
}
