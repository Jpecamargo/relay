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

const Page = PAGES[getCurrentWindow().label] ?? Panel;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
