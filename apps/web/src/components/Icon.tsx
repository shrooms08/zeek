/** Pictograms: 24 grid, 2.25 stroke, square caps (design system § 05). */

const PATHS: Record<string, string> = {
  chest: "M3 10h18v10H3zM3 10l2-5h14l2 5M10 14h4",
  hourglass: "M6 3h12M6 21h12M7 3c0 5 10 5 10 9s-10 4-10 9M17 3c0 5-10 5-10 9",
  shovel: "M14 4l6 6M17 7l-9 9M5 15l4 4-3 2-3-3z",
  wallet: "M3 7h16v12H3zM3 7l12-3v3M15 13h3",
  menu: "M4 6h16M4 12h16M4 18h16",
  plus: "M12 5v14M5 12h14",
  minus: "M5 12h14",
  wand: "M4 20l10-10M14 4v3M18 6l-2 2M20 10h-3",
  lock: "M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3",
  check: "M5 12l4 4 10-10",
  x: "M6 6l12 12M18 6L6 18",
  fire: "M12 3c3 4 6 6 6 11a6 6 0 0 1-12 0c0-3 2-4 2-6 2 1 3 3 4 5 1-3 0-6 0-10z",
  key: "M8 15a4 4 0 1 1 3-6l9 0v3h-2v2h-3v-2h-4",
  eye: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
  copy: "M8 8h12v12H8zM4 4h12v4M4 4v12h4",
  qr: "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h3v3h-3zM18 18h2v2h-2z",
  back: "M15 5l-7 7 7 7",
  warning: "M12 3l10 18H2zM12 10v5M12 18v1",
  refresh: "M20 12a8 8 0 1 1-3-6M20 4v5h-5",
  mine: "M3 20V12a9 9 0 0 1 18 0v8M8 20v-6h8v6",
  shout: "M4 10h4l8-5v14l-8-5H4zM19 9v6",
  seal: "M12 3l3 3h4v4l3 3-3 3v4h-4l-3 3-3-3H5v-4l-3-3 3-3V6h4z",
  send: "M3 12l18-8-8 18-2-8z",
  info: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 11v6M12 7v1",
  history: "M4 12a8 8 0 1 0 3-6M4 4v5h5M12 8v5l3 2",
  sparkle: "M12 3v6M12 15v6M3 12h6M15 12h6",
  vault: "M3 4h18v16H3zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM7 20v2M17 20v2",
  link: "M10 14l4-4M8 12l-3 3a3 3 0 0 0 4 4l3-3M16 12l3-3a3 3 0 0 0-4-4l-3 3",
  share: "M12 3v12M7 8l5-5 5 5M5 14v6h14v-6",
  chat: "M3 4h18v12H8l-5 4z",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 3",
  coin: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM9 9h5a2 2 0 0 1 0 4H9v-4zM9 13h6",
  shield: "M12 3l8 3v6c0 5-4 8-8 9-4-1-8-4-8-9V6z",
  globe: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18",
  tile: "M12 4l9 5-9 5-9-5zM3 9v6l9 5 9-5V9",
  offline: "M3 3l18 18M8 15a6 6 0 0 1 8 0M5 12a10 10 0 0 1 4-3M15 9a10 10 0 0 1 4 3M12 19v1",
  download: "M12 3v12M7 10l5 5 5-5M5 20h14",
};

export type IconName = keyof typeof PATHS;

export function Icon({ name, size = 20, label }: { name: string; size?: number; label?: string }) {
  const d = PATHS[name] ?? PATHS.info;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.25}
      strokeLinecap="square"
      strokeLinejoin="miter"
      role={label === undefined ? undefined : "img"}
      aria-label={label}
      aria-hidden={label === undefined ? true : undefined}
      focusable="false"
    >
      <path d={d} />
    </svg>
  );
}
