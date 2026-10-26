// Ajustes › Layout: quem guarda e edita os layouts (D42/D44). A troca vale na hora, sem passar pelo "Salvar".
import { invokeCommand, type Host } from "@relay/protocol-types";
import { Banner, Section, Select } from "@relay/ui";
import { useState } from "react";
import { errorMessage } from "../shared/errors";
import { useStatus } from "../shared/useStatus";
import styles from "./settings.module.css";

export function LayoutTab() {
  const status = useStatus();
  const [error, setError] = useState("");
  const [pending, setPending] = useState<Host | null>(null);
  if (!status) return null;

  const peer = status.peer_name ?? "O outro computador";
  const options = [
    { value: "Local", label: "Este computador" },
    { value: "Peer", label: peer },
  ] as const;
  const change = async (host: Host) => {
    setError("");
    setPending(host);
    try {
      await invokeCommand("set_layout_host", { host });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setPending(null);
    }
  };

  return (
    <Section title="Layout">
      {error ? <Banner tone="danger">{error}</Banner> : null}
      <Select<Host> label="Guardar os layouts em" value={pending ?? status.layout_host} options={options} disabled={!status.layout_editable} onChange={(h) => void change(h)} />
      <p className={styles.hint}>
        {status.layout_editable
          ? "Quem guarda os layouts é o único que os edita sem pedir; o outro computador mantém uma cópia para consulta."
          : "Só leitura agora: a troca precisa do outro computador conectado e com o Relay 0.3 ou mais novo."}
      </p>
    </Section>
  );
}
