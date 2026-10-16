// dono: MAC1. Um único build Vite serve as três janelas; o label da janela escolhe a página.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Editor } from "./editor";
import { Panel } from "./panel";
import { Settings } from "./settings";

const PAGES: Record<string, () => React.JSX.Element> = {
  panel: Panel,
  settings: Settings,
  editor: Editor,
};

const Page = PAGES[getCurrentWindow().label] ?? Panel;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
