import { invokeCommand, type DisplayInfo, type LayoutProfile, type LayoutWarning, type PlacedDisplay } from "@relay/protocol-types";
import { Banner, Button, KeyCombo, Section, Select, Slider, Toggle, useRelayEvent } from "@relay/ui";
import { useCallback, useEffect, useMemo, useState } from "react";
import { errorMessage } from "../shared/errors";
import { Canvas } from "./Canvas";
import styles from "./editor.module.css";
import { jumpIds } from "../shared/jump";
import { alignDisplays, clampSensitivity, profileId, reconcile, rotateDisplay, SENSITIVITY_MAX, SENSITIVITY_MIN } from "./geometry";

interface Loaded {
  profile: LayoutProfile;
  profiles: LayoutProfile[];
  connected: DisplayInfo[];
}

const ORIENTATIONS = [
  { value: "auto", label: "Automática" },
  { value: "0", label: "Paisagem (0°)" },
  { value: "90", label: "Retrato (90°)" },
  { value: "180", label: "Paisagem invertida (180°)" },
  { value: "270", label: "Retrato invertido (270°)" },
];

export function Editor() {
  const [loaded, setLoaded] = useState<Loaded>();
  const [displays, setDisplays] = useState<PlacedDisplay[]>([]);
  const [detached, setDetached] = useState(false);
  const [name, setName] = useState("");
  const [key, setKey] = useState<LayoutProfile["key"]>([]);
  const [selected, setSelected] = useState<string>();
  const [warnings, setWarnings] = useState<LayoutWarning[]>([]);
  const [error, setError] = useState<string>();
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);

  const apply = useCallback((p: LayoutProfile, connected: DisplayInfo[]) => {
    setDisplays(reconcile(p.displays, connected));
    setDetached(p.windows_detached);
    setName(p.name);
    setKey(p.key);
    setDirty(false);
    setWarnings([]);
  }, []);

  const load = useCallback(async () => {
    try {
      const [payload, profile, config] = await Promise.all([
        invokeCommand("get_displays"),
        invokeCommand("get_layout"),
        invokeCommand("get_config"),
      ]);
      const connected = [...payload.mac, ...payload.windows];
      setLoaded({ profile, profiles: config.layout.profiles, connected });
      apply(profile, connected);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [apply]);

  useEffect(() => void load(), [load]);
  // Tela conectada ou removida: recarrega (a combinação nova ganha o perfil padrão, D28).
  useRelayEvent("relay:displays", () => void load());

  const info = useMemo(() => new Map((loaded?.connected ?? []).map((d) => [d.id, d])), [loaded]);
  const jump = useMemo(() => jumpIds(displays), [displays]);

  const edit = (next: PlacedDisplay[]) => {
    setDisplays(next);
    setDirty(true);
  };
  const windowsIds = displays.filter((d) => d.machine === "Windows").map((d) => d.display_id);
  const selectedWin = selected && windowsIds.includes(selected) ? selected : undefined;
  const alignIds = new Set(detached && selectedWin ? [selectedWin] : windowsIds);

  const profileOptions = useMemo(() => {
    if (!loaded) return [];
    const all = [loaded.profile, ...loaded.profiles.filter((p) => profileId(p.key) !== profileId(loaded.profile.key))];
    return all.map((p) => ({ value: profileId(p.key), label: p.name }));
  }, [loaded]);

  const choose = (id: string) => {
    if (!loaded) return;
    const p = [loaded.profile, ...loaded.profiles].find((x) => profileId(x.key) === id);
    if (p) apply(p, loaded.connected);
  };

  const save = async () => {
    setSaving(true);
    setError(undefined);
    try {
      const profile: LayoutProfile = { key, name, displays, windows_detached: detached };
      const result = await invokeCommand("save_layout", { profile });
      setWarnings(result.warnings);
      setDirty(false);
      setLoaded((l) => l && { ...l, profile });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  if (!loaded) {
    return <main className={styles.page}>{error ? <Banner tone="danger">{error}</Banner> : <p className={styles.dim}>Carregando telas…</p>}</main>;
  }

  return (
    <main className={styles.page}>
      <header className={styles.toolbar}>
        <Select label="Perfil" value={profileId(key)} options={profileOptions} onChange={choose} />
        <Toggle label="Mover separadamente" checked={detached} onChange={(v) => { setDetached(v); setDirty(true); }} />
        <div className={styles.actions}>
          <Button onClick={() => edit(alignDisplays(displays, alignIds, "top"))}>Alinhar pelo topo</Button>
          <Button onClick={() => edit(alignDisplays(displays, alignIds, "center"))}>Alinhar pelo centro</Button>
          <Button variant="primary" disabled={!dirty || saving} onClick={() => void save()}>
            Salvar
          </Button>
        </div>
      </header>

      {error ? <Banner tone="danger">{error}</Banner> : null}
      {warnings.map((w, i) => (
        <Banner key={i} tone="warn">
          {w.message}
        </Banner>
      ))}

      <div className={styles.body}>
        <Canvas displays={displays} info={info} jump={jump} detached={detached} selected={selected} onSelect={setSelected} onChange={edit} />
        <aside className={styles.side}>
          <Section title="Telas">
            <span className={styles.dim}>Arraste para encaixar nas bordas; segure Option para desligar o ímã.</span>
            {displays.map((d) => (
              <div key={d.display_id} className={styles.item} data-selected={selected === d.display_id}>
                <button type="button" className={styles.itemHead} onClick={() => setSelected(d.display_id)}>
                  <span className={styles.itemName}>{info.get(d.display_id)?.name ?? d.display_id}</span>
                  <span className={styles.dim}>{d.machine === "Mac" ? "Mac" : "Windows"}</span>
                </button>
                <div className={styles.jump}>
                  <span className={styles.dim}>Pular para esta tela</span>
                  <KeyCombo combo={{ modifiers: ["Ctrl", "Option"], key: `Digit${jump.get(d.display_id)}` }} />
                </div>
                {d.machine === "Windows" ? (
                  <Select
                    label="Orientação"
                    value={d.rotation == null ? "auto" : String(d.rotation)}
                    options={ORIENTATIONS}
                    onChange={(v) => edit(rotateDisplay(displays, info, d.display_id, v === "auto" ? null : Number(v)))}
                  />
                ) : null}
                <Slider
                  label="Sensibilidade"
                  min={SENSITIVITY_MIN}
                  max={SENSITIVITY_MAX}
                  step={0.05}
                  debounceMs={0}
                  value={d.sensitivity}
                  format={(v) => `${v.toFixed(2).replace(".", ",")}×`}
                  onChange={(v) =>
                    edit(displays.map((x) => (x.display_id === d.display_id ? { ...x, sensitivity: clampSensitivity(v) } : x)))
                  }
                />
              </div>
            ))}
          </Section>
        </aside>
      </div>
    </main>
  );
}
