import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { UpdaterProvider } from "@relay/ui";
import { App } from "./App";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <UpdaterProvider>
      <App />
    </UpdaterProvider>
  </StrictMode>,
);
