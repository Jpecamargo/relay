// dono: MAC1. Um único build Vite serve o painel e a janela principal; o label da janela escolhe a página.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Main } from "./shell";
import { Panel } from "./panel";

const PAGES: Record<string, () => React.JSX.Element> = {
  panel: Panel,
  main: Main,
};

const label = getCurrentWindow().label;
const Page = PAGES[label] ?? Panel;

// Margem padrão do body (8px) fazia o conteúdo passar da janela e gerar rolagem em x e y.
document.body.style.margin = "0";
if (label !== "main") {
  // O painel tem a altura ajustada ao conteúdo e largura fixa: nunca rola.
  document.documentElement.style.overflow = "hidden";
  document.body.style.overflow = "hidden";
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
