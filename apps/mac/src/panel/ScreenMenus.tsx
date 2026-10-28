import { invokeCommand, type CommandResult, type DisplayInfo, type LayoutBook, type LayoutProfile } from "@relay/protocol-types";
import { MenuRow, useRelayEvent } from "@relay/ui";
import { useEffect, useState } from "react";
import { errorText } from "../shared/errors";
import { jumpIds } from "../shared/jump";
import { send } from "../shared/send";
import type { Act } from "../shared/useActions";
import { hideCurrentWindow } from "../shared/windows";

interface Target {
  n: number;
  id: string;
  name: string;
}

/** Telas na ordem do pulo (1 = MacBook), com nome. Relê o layout sempre que o submenu abre. */
function useTargets(open: boolean): { targets: Target[]; windows: DisplayInfo[] } {
  const [data, setData] = useState<{ targets: Target[]; windows: DisplayInfo[] }>({ targets: [], windows: [] });
  useEffect(() => {
    if (!open) return;
    void Promise.all([invokeCommand("get_displays"), invokeCommand("get_layout")]).then(([d, layout]) => {
      const names = new Map([...d.local, ...d.peer].map((x) => [x.id, x.name]));
      const order = jumpIds(layout.displays);
      const targets = [...order].map(([id, n]) => ({ n, id, name: names.get(id) ?? id })).sort((a, b) => a.n - b.n);
      setData({ targets, windows: d.peer });
    }).catch(() => undefined);
  }, [open]);
  return data;
}

/** "Ir para ›": leva o cursor para uma tela (mesmo efeito do atalho ⌃⌥n). */
export function GotoMenu({ act, online, hotkey }: { act: Act; online: boolean; hotkey: (n: number) => string | undefined }) {
  const [open, setOpen] = useState(false);
  const { targets } = useTargets(open);
  return (
    <>
      <MenuRow label="Ir para" icon="cursor" chevron={open ? "open" : true} onClick={() => setOpen((o) => !o)} />
      {open
        ? targets.map((t) => (
            <MenuRow
              nested key={t.id} label={t.name} shortcut={hotkey(t.n)} disabled={!online && t.n !== 1}
              onClick={() => { void act("Ir para a tela", () => invokeCommand("jump_to", { n: t.n })); void hideCurrentWindow().catch(() => undefined); }}
            />
          ))
        : null}
    </>
  );
}

/** "Layout ›": troca o perfil de layout em uso. Só aparece com dois perfis ou mais. */
export function LayoutMenu({ act }: { act: Act }) {
  const [open, setOpen] = useState(false);
  const [profiles, setProfiles] = useState<LayoutProfile[]>([]);
  const [active, setActive] = useState("");
  // O painel vive o dia todo e o livro muda no editor ou no outro computador: relê ao abrir e
  // segue cada relay:layout. Sem isso o ativo ficava velho e o clique nele era ignorado.
  const show = (book: LayoutBook) => {
    setProfiles(book.profiles);
    setActive(book.profiles.some((p) => p.name === book.active) ? book.active ?? "" : book.profiles[0]?.name ?? "");
  };
  useEffect(() => {
    void invokeCommand("get_layout_book").then(show).catch(() => undefined);
  }, [open]);
  useRelayEvent("relay:layout", show);

  if (profiles.length < 2) return null;
  const choose = (name: string) => {
    if (name === active) return;
    setActive(name);
    void act("Trocar layout", () => invokeCommand("set_layout_profiles", { profiles, active: name }));
  };
  return (
    <>
      <MenuRow label="Layout" icon="layout" chevron={open ? "open" : true} onClick={() => setOpen((o) => !o)} />
      {open ? profiles.map((p) => <MenuRow nested key={p.name} label={p.name} selected={p.name === active} onClick={() => choose(p.name)} />) : null}
    </>
  );
}

/**
 * "Capturar tela do outro computador ›": só envia `CaptureScreen`. O agente responde ok e manda o PNG como
 * ClipboardOffer, que o app aplica direto no clipboard do Mac (janela aberta ao enviar o comando).
 * Com uma tela só, captura direto.
 */
export function CaptureMenu({ show, firstTime, peer }: {
  peer: string;
  show: (message: string, tone: "ok" | "danger") => void;
  firstTime: (r: CommandResult) => boolean;
}) {
  const [open, setOpen] = useState(false);
  const { windows } = useTargets(true);

  const capture = (display_id: string) => {
    void send({ CaptureScreen: { display_id } }).then(
      (r) => {
        if (r.ok) show("Captura copiada para o clipboard", "ok");
        else if (firstTime(r)) show(`Captura de tela: ${errorText(r.error ?? "falhou")}`, "danger");
      },
      (e) => show(`Captura de tela: ${errorText(e)}`, "danger"),
    );
  };

  if (windows.length === 0) return null;
  const single = windows.length === 1;
  return (
    <>
      <MenuRow
        label={`Capturar tela de ${peer}`} icon="monitor" chevron={single ? undefined : open ? "open" : true}
        onClick={() => (single ? capture(windows[0]!.id) : setOpen((o) => !o))}
      />
      {open && !single ? windows.map((d) => <MenuRow nested key={d.id} label={d.name} onClick={() => capture(d.id)} />) : null}
    </>
  );
}
