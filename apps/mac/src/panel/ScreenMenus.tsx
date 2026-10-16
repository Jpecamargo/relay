import { invokeCommand, type CommandResult, type DisplayInfo } from "@relay/protocol-types";
import { MenuRow } from "@relay/ui";
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
      const names = new Map([...d.mac, ...d.windows].map((x) => [x.id, x.name]));
      const order = jumpIds(layout.displays);
      const targets = [...order].map(([id, n]) => ({ n, id, name: names.get(id) ?? id })).sort((a, b) => a.n - b.n);
      setData({ targets, windows: d.windows });
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

/**
 * "Capturar tela do PC ›": a imagem vai para o clipboard do Mac. Com uma tela só, captura direto.
 * O `ClipboardItem` recebe a promessa da captura para manter o gesto do clique (WebKit).
 */
export function CaptureMenu({ show, firstTime }: {
  show: (message: string, tone: "ok" | "danger") => void;
  firstTime: (r: CommandResult) => boolean;
}) {
  const [open, setOpen] = useState(false);
  const { windows } = useTargets(true);

  const capture = (display_id: string) => {
    let reported = false;
    const png = send({ CaptureScreen: { display_id } }).then((r) => {
      const out = r.ok ? r.output : null;
      if (out && "Png" in out) return new Blob([new Uint8Array(out.Png)], { type: "image/png" });
      if (firstTime(r)) show(`Captura de tela: ${errorText(r.error ?? "falhou")}`, "danger");
      reported = true;
      throw new Error(r.error ?? "falhou");
    });
    navigator.clipboard.write([new ClipboardItem({ "image/png": png })]).then(
      () => show("Captura copiada para o clipboard", "ok"),
      (e) => { if (!reported) show(`Captura de tela: ${errorText(e)}`, "danger"); },
    );
  };

  if (windows.length === 0) return null;
  const single = windows.length === 1;
  return (
    <>
      <MenuRow
        label="Capturar tela do PC" icon="monitor" chevron={single ? undefined : open ? "open" : true}
        onClick={() => (single ? capture(windows[0]!.id) : setOpen((o) => !o))}
      />
      {open && !single ? windows.map((d) => <MenuRow nested key={d.id} label={d.name} onClick={() => capture(d.id)} />) : null}
    </>
  );
}
