import { LogicalSize, getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, type RefObject } from "react";

/** Ajusta a altura da janela ao conteúdo (a largura é fixa). Sem efeito fora do Tauri. */
export function useFitWindow(ref: RefObject<HTMLElement | null>, width: number) {
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    let last = 0;
    const fit = () => {
      const height = Math.ceil(el.getBoundingClientRect().height);
      if (height === last || height === 0) return;
      last = height;
      try {
        void getCurrentWindow().setSize(new LogicalSize(width, height)).catch(() => undefined);
      } catch {
        // fora do Tauri (testes/navegador)
      }
    };
    const observer = new ResizeObserver(fit);
    observer.observe(el);
    fit();
    return () => observer.disconnect();
  }, [ref, width]);
}
