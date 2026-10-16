import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { UpdaterContext } from "./context";
import { UpdateModal } from "./UpdateModal";
import { updateOf, useUpdater, type UseUpdaterOptions } from "./useUpdater";

/** Evento que outra janela (painel, bandeja) emite para pedir o modal da atualização. */
export const OPEN_UPDATE_EVENT = "relay:open-update";

/** Um único atualizador por janela; o modal só abre quando a pessoa pede. */
export function UpdaterProvider({ children, ...options }: UseUpdaterOptions & { children: ReactNode }) {
  const updater = useUpdater(options);
  const [modalOpen, setModalOpen] = useState(false);
  const verify = useRef(updater.verify);
  verify.current = updater.verify;

  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    listen(OPEN_UPDATE_EVENT, () => {
      // Pode ter chegado antes de esta janela achar a atualização: confere e então abre.
      void verify.current().then((r) => r.resultado === "disponivel" && setModalOpen(true));
    })
      .then((fn) => (gone ? fn() : (off = fn)))
      .catch(() => undefined);
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  const value = { ...updater, openModal: () => setModalOpen(true), closeModal: () => setModalOpen(false) };
  return (
    <UpdaterContext.Provider value={value}>
      {children}
      {modalOpen && updateOf(updater.state) ? <UpdateModal /> : null}
    </UpdaterContext.Provider>
  );
}
