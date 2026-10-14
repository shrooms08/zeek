/** App shell: <TopBar> (logo, <NetBadge>, round, Menu) and the menu sheet. */

import { AnimatePresence, motion } from "framer-motion";
import { useContext, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { LiveContext } from "../api/LiveProvider";
import { useBoard, useConfig } from "../api/queries";
import { Icon } from "./Icon";
import { MenuRow, NetBadge } from "./ui";

export function Logo() {
  return (
    <Link to="/" className="flex items-center gap-2 text-white" aria-label="Zeek home">
      <svg width={36} height={28} viewBox="0 0 36 28" aria-hidden="true">
        <path d="M2 2h24l-14 16h14v8H2l14-16H2z" fill="#FFFFFF" />
      </svg>
      <span className="font-display text-heading" style={{ fontFamily: "var(--font-display)" }}>
        Zeek
      </span>
    </Link>
  );
}

const MENU: { to: string; icon: string; label: string }[] = [
  { to: "/wallet", icon: "wallet", label: "My wallet" },
  { to: "/rounds", icon: "history", label: "Past rounds" },
  { to: "/check", icon: "shield", label: "Check a round" },
  { to: "/keys", icon: "key", label: "Who holds the keys" },
  { to: "/campfire", icon: "fire", label: "Campfire" },
  { to: "/how", icon: "info", label: "How to play" },
];

export function Shell({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const board = useBoard();
  const config = useConfig();
  const live = useContext(LiveContext);
  return (
    <div className="min-h-dvh flex flex-col">
      <a href="#main" className="sr-only focus:not-sr-only">
        Skip to content
      </a>
      <header className="flex items-center gap-4 px-4 py-3 border-b-[3px] border-white">
        <Logo />
        {config.data !== undefined && <NetBadge net={config.data.params.network} />}
        <div className="flex-1" />
        {live !== "open" && (
          <span
            className="label text-fog normal-case tracking-normal"
            role="status"
            data-testid="reconnecting"
          >
            {live === "connecting" ? "Connecting…" : "Reconnecting…"}
          </span>
        )}
        {board.data !== undefined && (
          <span className="label hidden sm:inline" data-testid="round-label">
            Round {board.data.round}
          </span>
        )}
        <button
          type="button"
          className="btn flex items-center gap-2"
          onClick={() => {
            setOpen(true);
          }}
          aria-expanded={open}
          aria-controls="menu-sheet"
          data-testid="menu-button"
        >
          <Icon name="menu" size={18} />
          Menu
        </button>
      </header>
      <div id="main" className="flex-1 flex flex-col">
        {children}
      </div>
      <AnimatePresence>
        {open && (
          <motion.nav
            id="menu-sheet"
            aria-label="Menu"
            className="fixed inset-0 z-50 bg-night flex flex-col"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2, ease: [0.2, 0.9, 0.2, 1] }}
            onKeyDown={(e) => {
              if (e.key === "Escape") setOpen(false);
            }}
          >
            <div className="flex items-center justify-between px-4 py-3 border-b-[3px] border-white">
              <Logo />
              <button
                type="button"
                className="btn"
                onClick={() => {
                  setOpen(false);
                }}
                aria-label="Close menu"
                autoFocus
              >
                <Icon name="x" size={18} />
              </button>
            </div>
            {MENU.map((m) => (
              <MenuRow
                key={m.to}
                {...m}
                onClick={() => {
                  setOpen(false);
                }}
              />
            ))}
            <p className="p-4 text-fog mt-auto">Everyone sees the field. No one sees you.</p>
          </motion.nav>
        )}
      </AnimatePresence>
    </div>
  );
}
