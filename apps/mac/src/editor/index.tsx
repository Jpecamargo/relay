import { invokeCommand, type DisplayInfo, type StatusSnapshot, type LayoutProfile, type LayoutWarning, type PlacedDisplay } from "@relay/protocol-types";
import { Banner, Button, ConfirmDialog, KeyCombo, Section, Select, Slider, Toggle, useRelayEvent } from "@relay/ui";
import { useCallback, useEffect, useMemo, useState } from "react";
import { errorMessage } from "../shared/errors";
import { Canvas } from "./Canvas";
import styles from "./editor.module.css";
import { jumpIds } from "../shared/jump";
import { EMPTY_STATUS } from "../shared/status";
import { alignDisplays, clampSensitivity, reconcile, rotateDisplay, SENSITIVITY_MAX, SENSITIVITY_MIN } from "./geometry";

/** Versão do peer abaixo de 0.3.0 não sabe sincronizar o livro de layouts. */
function olderThan030(v: string | null): boolean {
  if (!v) return false;
  const [maj = 0, min = 0] = v.split(".").map((n) => Number.parseInt(n, 10) || 0);
  return maj === 0 && min < 3;
}

interface Loaded {
  profile: LayoutProfile;
  profiles: LayoutProfile[];
  connected: DisplayInfo[];
  localIds: ReadonlySet<string>;
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
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [selected, setSelected] = useState<string>();
  const [warnings, setWarnings] = useState<LayoutWarning[]>([]);
  const [error, setError] = useState<string>();
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<StatusSnapshot>(EMPTY_STATUS);
  // Muda a cada relay:layout: remonta o Canvas e descarta o arraste em andamento.
  const [rev, setRev] = useState(0);

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
      const [payload, profile, book] = await Promise.all([
        invokeCommand("get_displays"),
        invokeCommand("get_layout"),
        invokeCommand("get_layout_book"),
      ]);
      const connected = [...payload.local, ...payload.peer];
      setLoaded({ profile, profiles: book.profiles, connected, localIds: new Set(payload.local.map((d) => d.id)) });
      apply(profile, connected);
      setSelected((s) => (s && profile.displays.some((d) => d.display_id === s) ? s : undefined));
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [apply]);

  useEffect(() => void load(), [load]);
  useEffect(() => {
    invokeCommand("get_status").then((s) => s && setStatus(s), () => undefined);
  }, []);
  useRelayEvent("relay:status", setStatus);
  useRelayEvent("relay:layout", () => {
    setRev((n) => n + 1);
    void load();
  });
  // Tela conectada ou removida: recarrega (a combinação nova ganha o perfil padrão, D28).
  useRelayEvent("relay:displays", () => void load());

  const readOnly = !status.layout_editable;
  const hostName = status.layout_host === "Peer" ? (status.peer_name ?? "o outro computador") : "este computador";
  const needsUpdate = olderThan030(status.peer_version);

  const info = useMemo(() => new Map((loaded?.connected ?? []).map((d) => [d.id, d])), [loaded]);
  const jump = useMemo(() => jumpIds(displays), [displays]);

  const edit = (next: PlacedDisplay[]) => {
    setDisplays(next);
    setDirty(true);
  };
  const windowsIds = displays.filter((d) => d.machine === "Windows").map((d) => d.display_id);
  const selectedWin = selected && windowsIds.includes(selected) ? selected : undefined;
  const alignIds = new Set(detached && selectedWin ? [selectedWin] : windowsIds);

  // Sem perfil salvo, a lista é só o perfil em uso (o padrão, ainda não gravado).
  const saved = useMemo(() => (loaded ? (loaded.profiles.length ? loaded.profiles : [loaded.profile]) : []), [loaded]);
  const current = loaded?.profile.name ?? "";
  const profileOptions = saved.map((p) => ({ value: p.name, label: p.name }));

  const setProfiles = async (profiles: LayoutProfile[], active: string) => {
    setError(undefined);
    try {
      await invokeCommand("set_layout_profiles", { profiles, active });
      await load();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const choose = (n: string) => {
    if (n !== current) void setProfiles(saved, n);
  };

  const createProfile = () => {
    const names = new Set(saved.map((p) => p.name));
    let n = saved.length + 1;
    while (names.has(`Perfil ${n}`)) n++;
    const fresh: LayoutProfile = { key, name: `Perfil ${n}`, displays, windows_detached: detached };
    void setProfiles([...saved, fresh], fresh.name);
  };

  const deleteProfile = () => {
    setConfirmDelete(false);
    const rest = saved.filter((p) => p.name !== current);
    const next = rest[0];
    if (next) void setProfiles(rest, next.name);
  };

  const save = async () => {
    setSaving(true);
    setError(undefined);
    try {
      const trimmed = name.trim() || current;
      if (trimmed !== current) {
        if (saved.some((p) => p.name === trimmed)) throw new Error(`Já existe um perfil chamado ${trimmed}.`);
        const renamed = saved.map((p) => (p.name === current ? { ...p, name: trimmed } : p));
        await invokeCommand("set_layout_profiles", { profiles: renamed, active: trimmed });
      }
      const profile: LayoutProfile = { key, name: trimmed, displays, windows_detached: detached };
      const result = await invokeCommand("save_layout", { profile });
      await load();
      setWarnings(result.warnings);
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
        <Select label="Perfil" value={current} options={profileOptions} onChange={choose} disabled={readOnly} />
        <label className={styles.nameField}>
          <span className={styles.dim}>Nome</span>
          <input className={styles.input} aria-label="Nome do perfil" disabled={readOnly} value={name} onChange={(e) => { setName(e.target.value); setDirty(true); }} />
        </label>
        <Button disabled={readOnly} onClick={createProfile}>Novo perfil</Button>
        <Button disabled={readOnly || saved.length < 2} onClick={() => setConfirmDelete(true)}>Excluir perfil</Button>
        <Toggle label="Mover separadamente" checked={detached} disabled={readOnly} onChange={(v) => { setDetached(v); setDirty(true); }} />
        <div className={styles.actions}>
          <Button disabled={readOnly} onClick={() => edit(alignDisplays(displays, alignIds, "top"))}>Alinhar pelo topo</Button>
          <Button disabled={readOnly} onClick={() => edit(alignDisplays(displays, alignIds, "center"))}>Alinhar pelo centro</Button>
          <Button variant="primary" disabled={readOnly || !dirty || saving} onClick={() => void save()}>
            Salvar
          </Button>
        </div>
      </header>

      <ConfirmDialog
        open={confirmDelete}
        title={`Excluir o perfil ${current}?`}
        message="A disposição das telas deste perfil será perdida."
        confirmLabel="Excluir"
        destructive
        onConfirm={deleteProfile}
        onCancel={() => setConfirmDelete(false)}
      />
      {readOnly ? (
        <Banner tone="warn">
          {needsUpdate
            ? `Atualize o Relay em ${hostName} para editar os layouts: esta versão não os sincroniza.`
            : `Os layouts ficam em ${hostName}. Conecte-o para editar.`}
        </Banner>
      ) : null}
      {error ? <Banner tone="danger">{error}</Banner> : null}
      {warnings.map((w, i) => (
        <Banner key={i} tone="warn">
          {w.message}
        </Banner>
      ))}

      <div className={styles.body}>
        <Canvas key={rev} readOnly={readOnly} localIds={loaded.localIds} displays={displays} info={info} jump={jump} detached={detached} selected={selected} onSelect={setSelected} onChange={edit} />
        <aside className={styles.side}>
          <Section title="Telas">
            <span className={styles.dim}>Arraste as telas do PC para encaixar nas bordas; segure Option para desligar o ímã. As do Mac seguem Ajustes do Sistema › Monitores.</span>
            {displays.map((d) => (
              <div key={d.display_id} className={styles.item} data-selected={selected === d.display_id}>
                <button type="button" className={styles.itemHead} onClick={() => setSelected(d.display_id)}>
                  <span className={styles.itemName}>{info.get(d.display_id)?.name ?? d.display_id}</span>
                  <span className={styles.dim}>{loaded.localIds.has(d.display_id) ? "Este computador" : d.machine === "Mac" ? "Mac" : "Windows"}</span>
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
                    disabled={readOnly}
                    onChange={(v) => edit(rotateDisplay(displays, info, d.display_id, v === "auto" ? null : Number(v)))}
                  />
                ) : null}
                <Slider
                  label="Sensibilidade"
                  min={SENSITIVITY_MIN}
                  max={SENSITIVITY_MAX}
                  step={0.05}
                  debounceMs={0}
                  disabled={readOnly}
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
