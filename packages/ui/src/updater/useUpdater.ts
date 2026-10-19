import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { useCallback, useEffect, useRef, useState } from "react";

export type CheckResult =
  | { resultado: "disponivel"; versao: string }
  | { resultado: "atualizado" }
  | { resultado: "erro"; mensagem: string }
  | { resultado: "ignorado" };

export type UpdaterState =
  | { status: "nao-suportado" }
  | { status: "ocioso" | "verificando" | "atualizado" }
  | { status: "disponivel"; update: Update }
  | { status: "baixando"; update: Update; baixado: number; total: number | null }
  | { status: "reiniciando"; update: Update }
  | { status: "erro"; mensagem: string; update?: Update };

export const FIRST_CHECK_MS = 15_000;
export const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

const messageOf = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause));

/** `latest.json` ainda sem esta plataforma (release sendo publicada): para o usuário, não há versão nova. */
const platformMissing = (cause: unknown) => /platforms/i.test(messageOf(cause));

export const CHECK_ERROR = "Não foi possível procurar atualizações agora. Confira a internet e tente de novo.";
export const INSTALL_ERROR = "A atualização não foi concluída. Tente de novo em alguns minutos.";

/**
 * Só o app instalado (build de produção dentro do Tauri) procura atualizações: no
 * `pnpm tauri dev`, no navegador e nos testes o updater fica `nao-suportado`.
 */
// `import.meta.env` vem do Vite; o kit não carrega os tipos dele, então o ts-ignore vale nos dois lados.
// @ts-ignore
const supported = () => isTauri() && Boolean(import.meta.env.PROD);

export interface UseUpdaterOptions {
  /**
   * `true` (padrão): primeira checagem 15 s após abrir e depois a cada 6 h. `false`: uma única
   * checagem silenciosa ao montar, sem timers — para a janela que não é dona do agendamento.
   */
  schedule?: boolean;
}

/** Procura atualizações no `latest.json` do GitHub, baixa, instala e reinicia o app. */
export function useUpdater({ schedule = true }: UseUpdaterOptions = {}) {
  const [active] = useState(supported);
  const [state, setState] = useState<UpdaterState>(active ? { status: "ocioso" } : { status: "nao-suportado" });
  const [version, setVersion] = useState("");
  const [lastCheck, setLastCheck] = useState<number | null>(null);
  const busy = useRef(false);

  useEffect(() => {
    if (active) getVersion().then(setVersion, () => undefined);
  }, [active]);

  const verify = useCallback(
    async (manual = false): Promise<CheckResult> => {
      if (!active || busy.current) return { resultado: "ignorado" };
      busy.current = true;
      if (manual) setState({ status: "verificando" });
      try {
        const update = await check();
        setLastCheck(Date.now());
        setState(update ? { status: "disponivel", update } : { status: manual ? "atualizado" : "ocioso" });
        return update ? { resultado: "disponivel", versao: update.version } : { resultado: "atualizado" };
      } catch (cause) {
        console.warn("updater: check", messageOf(cause));
        if (platformMissing(cause)) {
          setLastCheck(Date.now());
          setState({ status: manual ? "atualizado" : "ocioso" });
          return { resultado: "atualizado" };
        }
        // Checagem automática falhando (offline, GitHub fora) não incomoda; só a manual mostra o erro.
        if (manual) setState({ status: "erro", mensagem: CHECK_ERROR });
        return { resultado: "erro", mensagem: CHECK_ERROR };
      } finally {
        busy.current = false;
      }
    },
    [active],
  );

  const install = useCallback(async (update: Update) => {
    if (busy.current) return;
    busy.current = true;
    let baixado = 0;
    let total: number | null = null;
    setState({ status: "baixando", update, baixado, total });
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") total = event.data.contentLength ?? null;
        if (event.event === "Progress") baixado += event.data.chunkLength;
        setState({ status: "baixando", update, baixado, total });
      });
      setState({ status: "reiniciando", update });
      await relaunch();
    } catch (cause) {
      console.warn("updater: install", messageOf(cause));
      setState({ status: "erro", mensagem: INSTALL_ERROR, update });
    } finally {
      busy.current = false;
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    if (!schedule) {
      void verify();
      return;
    }
    const first = window.setTimeout(() => void verify(), FIRST_CHECK_MS);
    const interval = window.setInterval(() => void verify(), CHECK_INTERVAL_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(interval);
    };
  }, [active, schedule, verify]);

  return { state, version, lastCheck, verify, install };
}

export type Updater = ReturnType<typeof useUpdater>;

/** Versão nova em mãos (disponível, baixando, reiniciando ou com erro após tentar instalar). */
export const updateOf = (state: UpdaterState): Update | undefined => ("update" in state ? state.update : undefined);
