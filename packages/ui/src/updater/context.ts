import { createContext, useContext } from "react";
import type { Updater } from "./useUpdater";

export type UpdaterContextValue = Updater & { openModal: () => void; closeModal: () => void };

export const UpdaterContext = createContext<UpdaterContextValue | null>(null);

export function useUpdate(): UpdaterContextValue {
  const ctx = useContext(UpdaterContext);
  if (!ctx) throw new Error("useUpdate fora do UpdaterProvider");
  return ctx;
}
