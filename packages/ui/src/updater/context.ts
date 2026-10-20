import { createContext, useContext } from "react";
import type { Updater } from "./useUpdater";

/** O modal foi aberto para igualar a versão do outro lado ("o PC já está na 0.2.0"). */
export interface MatchPeer {
  name: string;
  version: string;
}

export type UpdaterContextValue = Updater & {
  openModal: () => void;
  closeModal: () => void;
  matchPeer?: MatchPeer;
};

export const UpdaterContext = createContext<UpdaterContextValue | null>(null);

export function useUpdate(): UpdaterContextValue {
  const ctx = useContext(UpdaterContext);
  if (!ctx) throw new Error("useUpdate fora do UpdaterProvider");
  return ctx;
}
