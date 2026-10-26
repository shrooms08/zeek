/**
 * Pictograms from the design system (§ 05): 24 grid, 2.25 stroke, square
 * caps; names ending in "ink" are hand-drawn marks with round caps.
 * Paths copied verbatim from design/mono (zeek-art.js IC).
 */

export const ICONS = {
  // Sound controls (drawn for the intro-sound work in the same 24 grid).
  speaker: "M3 9h4l5-4v14l-5-4H3z M16 9c1.3 1.6 1.3 4.4 0 6 M19 6c2.7 3.4 2.7 8.6 0 12",
  speakeroff: "M3 9h4l5-4v14l-5-4H3z M16 9l6 6 M22 9l-6 6",
  note: "M9 18V5l11-2v13 M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M17 19a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
  noteoff:
    "M9 18V5l11-2v13 M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M17 19a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M2 2l20 20",
  chest: "M3 10h18v10H3z M3 10c0-4 3-6 9-6s9 2 9 6 M10 11h4v4h-4z M3 14h7 M14 14h7",
  hourglass: "M6 3h12 M6 21h12 M7 3v3l5 6-5 6v3 M17 3v3l-5 6 5 6v3 M9.5 18.5h5",
  shovel: "M14.5 2.5l7 7 M18 6L9.5 14.5 M5.5 12.5l6 6-3.5 3.5-6-6z",
  tile: "M12 4l9 5-9 5-9-5z M3 9v4l9 5 9-5V9",
  eye: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z",
  person: "M12 3a3.5 3.5 0 1 0 0 7a3.5 3.5 0 1 0 0-7z M5 21c0-4 3-7 7-7s7 3 7 7",
  nobody: "M12 3a3.5 3.5 0 1 0 0 7a3.5 3.5 0 1 0 0-7z M5 21c0-4 3-7 7-7s7 3 7 7 M3 3l18 18",
  split:
    "M12 21v-8 M12 13L5 6 M12 13l7-7 M12 13V3 M5 6v4.5 M5 6h4.5 M19 6v4.5 M19 6h-4.5 M9 6l3-3 3 3",
  key: "M3 12a4 4 0 1 0 8 0a4 4 0 1 0-8 0z M11 12h10 M17.5 12v3.5 M21 12v3.5",
  lock: "M5 11h14v10H5z M8 11V7a4 4 0 0 1 8 0v4 M12 15v2.5",
  check: "M4 12.5l5 5L20 6.5",
  checkink: "M3.5 13c2 1.2 3.6 2.9 5.4 5.2C11.6 12.6 15.2 8.4 20.8 5",
  xink: "M5 5.5c4.4 4 9 8.6 14 13.5 M18.6 5c-4.2 4.4-8.6 9-13.4 14",
  star: "M12 3l2.6 5.9 6.4.6-4.9 4.2 1.5 6.3L12 16.7 6.4 20l1.5-6.3L3 9.5l6.4-.6z",
  x: "M5 5l14 14 M19 5L5 19",
  fire: "M12 2.5c1 4 6 6 6 11.5a6 6 0 0 1-12 0c0-3 2-4.5 3-6.5 1 2 2 3 3 3 0-3-1-5 0-8z",
  wallet: "M3 7h16v13H3z M3 7l12-4v4 M15 11.5h6v5h-6z",
  clock: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M12 7v5l3.5 2",
  coin: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M8.5 8.5h7l-7 7h7",
  dice: "M4 4h16v16H4z M8.5 8.5h.01 M15.5 15.5h.01 M12 12h.01 M15.5 8.5h.01 M8.5 15.5h.01",
  chain:
    "M10 14a4 4 0 0 1 0-5.6l3-3a4 4 0 0 1 5.6 5.6l-1.5 1.5 M14 10a4 4 0 0 1 0 5.6l-3 3a4 4 0 0 1-5.6-5.6l1.5-1.5",
  math: "M4 4h16v16H4z M7 9h4 M9 7v4 M13 9h4 M7.5 14.5l3 3 M10.5 14.5l-3 3 M13 14.5h4 M13 17.5h4",
  menu: "M4 6h16 M4 12h16 M4 18h16",
  plus: "M12 4v16 M4 12h16",
  minus: "M4 12h16",
  copy: "M8 8h12v12H8z M4 16V4h12",
  qr: "M4 4h6v6H4z M14 4h6v6h-6z M4 14h6v6H4z M14 14h2.5v2.5H14z M18 18h2v2h-2z M14 18.5h2.5 M18 14h2",
  arrow: "M4 12h16 M14 6l6 6-6 6",
  back: "M20 12H4 M10 6l-6 6 6 6",
  down: "M6 9l6 6 6-6",
  up: "M6 15l6-6 6 6",
  shield: "M12 3l8 3v6c0 5-4 8-8 9-4-1-8-4-8-9V6z M8.5 12l2.5 2.5 4.5-5",
  refresh: "M20 12a8 8 0 1 1-2.3-5.7 M20 3.5v4h-4",
  mine: "M3 20h18 M6 20v-9a6 6 0 0 1 12 0v9 M10 20v-5h4v5",
  shout: "M3 10v4h3l8 5V5L6 10z M18 9c1.3 1.3 1.3 4.7 0 6 M20.5 6.5c2.6 2.6 2.6 8.4 0 11",
  seal: "M3 6h18v12H3z M3 6l9 7 9-7",
  send: "M3 11l18-8-8 18-2-8z",
  warning: "M12 3l10 18H2z M12 10v5 M12 18v.01",
  info: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M12 11v6 M12 7.5v.01",
  history: "M4 12a8 8 0 1 0 2.3-5.7 M4 3.5v4h4 M12 8v4l3 2",
  sparkle:
    "M12 3v5 M12 16v5 M3 12h5 M16 12h5 M6 6l2.5 2.5 M15.5 15.5L18 18 M18 6l-2.5 2.5 M8.5 15.5L6 18",
  down2: "M12 4v12 M6 10l6 6 6-6 M4 20h16",
  up2: "M12 20V8 M6 14l6-6 6 6 M4 4h16",
  vault: "M3 4h18v15H3z M12 7.5a4 4 0 1 0 0 8a4 4 0 1 0 0-8z M12 11.5h.01 M6 19v2 M18 19v2",
  pen: "M4 20l4-1 11-11-3-3L5 16z M14 7l3 3",
  offline:
    "M3 3l18 18 M2 9a15 15 0 0 1 5-3 M12 19h.01 M8.5 15.5a5 5 0 0 1 7 0 M17 9.5a15 15 0 0 1 5 2",
  wand: "M4 20L15 9 M13 5V3 M19 11h2 M17 5l1.5-1.5 M17 7l-2-2",
  book: "M4 5h7a2 2 0 0 1 2 2v13a2 2 0 0 0-2-2H4z M20 5h-5a2 2 0 0 0-2 2 M20 5v13h-7",
  chat: "M4 5h16v11H9l-5 4z",
  flask: "M9 3h6 M10 3v6l-5 10h14l-5-10V3",
  globe: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z M3 12h18 M12 3c3 3 3 15 0 18 M12 3c-3 3-3 15 0 18",
  link: "M14 4h6v6 M20 4l-9 9 M18 14v6H4V6h6",
  play: "M7 4l13 8-13 8z",
  pause: "M7 4h3v16H7z M14 4h3v16h-3z",
  layers: "M12 3l9 5-9 5-9-5z M3 13l9 5 9-5 M3 17.5l9 5 9-5",
  hand: "M8 13V5a1.5 1.5 0 0 1 3 0v6 M11 10V3.5a1.5 1.5 0 0 1 3 0V10 M14 10V5a1.5 1.5 0 0 1 3 0v8c0 4-2.5 7-6 7s-5-2-6.5-5L3 11.5a1.5 1.5 0 0 1 2.5-1.5L8 13",
  share:
    "M18 3a3 3 0 1 0 0 6a3 3 0 1 0 0-6z M6 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z M18 15a3 3 0 1 0 0 6a3 3 0 1 0 0-6z M8.6 10.5l6.8-3.5 M8.6 13.5l6.8 3.5",
} as const;

export type IconName = keyof typeof ICONS;
