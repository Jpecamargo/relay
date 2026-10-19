import type { DiscoveredPeer } from "@relay/protocol-types";
import { Icon } from "@relay/ui";
import styles from "./pairing.module.css";

export interface PeerListProps {
  peers: DiscoveredPeer[];
  selectedId: string | null;
  onSelect: (peer: DiscoveredPeer) => void;
  emptyText: string;
}

export function PeerList({ peers, selectedId, onSelect, emptyText }: PeerListProps) {
  if (peers.length === 0) return <p className={styles.empty}>{emptyText}</p>;
  return (
    <ul className={styles.peers} aria-label="PCs encontrados">
      {peers.map((p) => (
        <li key={p.device_id}>
          <button type="button" className={styles.peer} aria-pressed={p.device_id === selectedId} onClick={() => onSelect(p)}>
            <span className={styles.peerName}>
              {p.name} <span className={styles.peerAddr}>{p.addrs[0]}</span>
            </span>
            {p.device_id === selectedId ? <Icon name="check" className={styles.peerCheck} /> : null}
          </button>
        </li>
      ))}
    </ul>
  );
}
