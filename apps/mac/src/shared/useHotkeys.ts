import { invokeCommand, type HotkeyAction } from "@relay/protocol-types";
import { comboLabel } from "@relay/ui";
import { useCallback, useEffect, useState } from "react";

type Bindings = Awaited<ReturnType<typeof loadHotkeys>>;
const loadHotkeys = () => invokeCommand("get_config").then((c) => c.hotkeys ?? []);

/**
 * Atalho global configurado para cada ação ("⌃⌥L"), lido do Config real. Relê ao abrir (foco da
 * janela), então o que o usuário mudou em Ajustes aparece no painel.
 */
export function useHotkeys(): (action: HotkeyAction) => string | undefined {
  const [bindings, setBindings] = useState<Bindings>([]);
  useEffect(() => {
    const load = () => void loadHotkeys().then(setBindings, () => undefined);
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, []);
  return useCallback(
    (action) => {
      const key = JSON.stringify(action);
      const b = bindings.find((x) => JSON.stringify(x.action) === key);
      return b ? comboLabel(b.combo) : undefined;
    },
    [bindings],
  );
}
