import type { SVGProps } from "react";

/** Ícones de linha (24x24, traço 1.5), no estilo SF Symbols/Lucide. Um conjunto só para todo o app. */
const PATHS = {
  check: "M20 6 9 17l-5-5",
  chevronRight: "m9 18 6-6-6-6",
  chevronDown: "m6 9 6 6 6-6",
  lock: "M7 11V8a5 5 0 0 1 10 0v3M5 11h14v10H5z",
  pause: "M8 5v14M16 5v14",
  play: "M7 4.5v15l12-7.5z",
  playPause: "M4 5v14l9-7zM17 5v14M21 5v14",
  prev: "M19 5v14L8 12zM5 5v14",
  next: "M5 5v14l11-7zM19 5v14",
  volume: "M11 5 6 9H3v6h3l5 4zM15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13",
  volumeLow: "M11 5 6 9H3v6h3l5 4zM15.5 8.5a5 5 0 0 1 0 7",
  volumeMute: "M11 5 6 9H3v6h3l5 4zM22 9l-6 6M16 9l6 6",
  speaker: "M5 3h14v18H5zM12 15.5a3 3 0 1 0 0 .01M12 7.5v.01",
  mic: "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM6 11a6 6 0 0 0 12 0M12 17v4",
  clipboard: "M9 4h6v3H9zM8 5.5H6v15h12v-15h-2",
  monitor: "M3 4h18v12H3zM8 20h8M12 16v4",
  power: "M12 3v9M6.4 6.4a8 8 0 1 0 11.2 0",
  window: "M3 5h18v14H3zM3 9h18",
  gear: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1-2 3.4-.2-.1a1.7 1.7 0 0 0-1.9.1 1.7 1.7 0 0 0-.8 1.5V22h-4v-.2a1.7 1.7 0 0 0-.8-1.5 1.7 1.7 0 0 0-1.9-.1l-.2.1-2-3.4.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H2v-4h.2a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1 2-3.4.2.1a1.7 1.7 0 0 0 1.9-.1A1.7 1.7 0 0 0 8.2 2.2V2h4v.2a1.7 1.7 0 0 0 .8 1.5 1.7 1.7 0 0 0 1.9.1l.2-.1 2 3.4-.1.1a1.7 1.7 0 0 0-.3 1.8 1.7 1.7 0 0 0 1.5 1H22v4h-.2a1.7 1.7 0 0 0-1.5 1z",
  keyboard: "M3 6h18v12H3zM7 10h.01M11 10h.01M15 10h.01M7 14h10",
  cursor: "M5 3l14 7-6 2-2 6z",
  wifi: "M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0M12 19.5h.01",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  home: "M4 11 12 4l8 7v9h-5v-6H9v6H4z",
  quit: "M9 4H4v16h5M16 8l4 4-4 4M20 12H9",
  alert: "M12 4 2.5 20h19zM12 10v4M12 17h.01",
  info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v5M12 8h.01",
  refresh: "M20 11a8 8 0 0 0-14-4M4 5v4h4M4 13a8 8 0 0 0 14 4M20 19v-4h-4",
  restart: "M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7",
  moon: "M20 14.5A8 8 0 1 1 9.5 4 6.5 6.5 0 0 0 20 14.5z",
  sun: "M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
  download: "M12 4v12M7 11l5 5 5-5M5 20h14",
  file: "M6 3h8l5 5v13H6zM14 3v5h5",
  send: "M4 12 20 4l-6 16-3-7z",
  externalLink: "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6",
  chart: "M4 20V6M4 20h16M8 16v-4M12 16V8M16 16v-6",
  layout: "M2 7h10v8H2zM14 5h8v10h-8zM5 19h4M17 19h2",
} as const;

export type IconName = keyof typeof PATHS;

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, "name"> {
  name: IconName;
  size?: number;
}

export function Icon({ name, size = 15, ...rest }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
