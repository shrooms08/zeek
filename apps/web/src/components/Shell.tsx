/** App shell: <TopBar> (logo, <NetBadge>, round, Menu) and the menu sheet. */

import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useContext, useEffect, useState, type ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import { LiveContext } from "../api/LiveProvider";
import { useBoard, useConfig } from "../api/queries";
import { Logo } from "../art/Logo";
import { Icon, type IconName } from "./Icon";
import { MenuRow, NetBadge } from "./ui";
import { EASE } from "../motion/tokens";
import { setSoundOn, soundOn } from "../sound/sound";

export function HomeLogo() {
  return (
    <Link to="/" className="flex items-center text-white" aria-label="Zeek home">
      <Logo variant="lockup" height={40} label={null} />
    </Link>
  );
}

const MENU: { to: string; icon: IconName; label: string }[] = [
  { to: "/wallet", icon: "wallet", label: "My wallet" },
  { to: "/rounds", icon: "history", label: "Past rounds" },
  { to: "/check", icon: "shield", label: "Check a round" },
  { to: "/keys", icon: "key", label: "Who holds the keys" },
  { to: "/campfire", icon: "fire", label: "Campfire" },
  { to: "/status", icon: "layers", label: "Status" },
  { to: "/how", icon: "book", label: "How to play" },
];

export function Shell({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  // Any navigation (a menu row, a link, back/forward) closes the menu.
  const { pathname } = useLocation();
  useEffect(() => {
    setOpen(false);
  }, [pathname]);
  const board = useBoard();
  const config = useConfig();
  const live = useContext(LiveContext);
  const reduce = useReducedMotion() === true;
  const [sound, setSound] = useState(soundOn);
  return (
    <div className="min-h-dvh flex flex-col">
      <a href="#main" className="sr-only focus:not-sr-only">
        Skip to content
      </a>
      <header className="zk-header flex items-center gap-4 px-4 sm:px-10">
        <HomeLogo />
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
          <span
            className="label text-fog hidden sm:inline"
            style={{ fontSize: 16 }}
            data-testid="round-label"
          >
            Round {board.data.round}
          </span>
        )}
        <button
          type="button"
          className="btn flex min-h-[52px] min-w-[52px] items-center justify-center gap-2.5 px-3 sm:px-5"
          style={{ fontSize: 18 }}
          onClick={() => {
            setOpen(true);
          }}
          aria-expanded={open}
          aria-controls="menu-sheet"
          aria-label="Menu"
          data-testid="menu-button"
        >
          <Icon name="menu" size={22} />
          <span className="hidden sm:inline">Menu</span>
        </button>
      </header>
      {config.data?.params.network === "main" && (
        <p className="px-4 sm:px-10 py-1 text-fog text-xs" data-testid="legal-notice">
          18+. Not available where prohibited.
        </p>
      )}
      <div id="main" className="flex-1 flex flex-col">
        {children}
      </div>
      <AnimatePresence>
        {open && (
          <>
            <motion.div
              className="fixed inset-0 z-40 bg-night/70 hidden sm:block"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.18 }}
              onClick={() => {
                setOpen(false);
              }}
              aria-hidden="true"
            />
            <motion.nav
              id="menu-sheet"
              aria-label="Menu"
              className="zk-menu fixed inset-y-0 right-0 z-50 w-full sm:w-[440px] bg-night flex flex-col"
              initial={reduce ? { opacity: 0 } : { opacity: 0, x: 48 }}
              animate={{ opacity: 1, x: 0, transition: { duration: 0.24, ease: EASE.out } }}
              exit={
                reduce
                  ? { opacity: 0 }
                  : { opacity: 0, x: 48, transition: { duration: 0.18, ease: EASE.in } }
              }
              onKeyDown={(e) => {
                if (e.key === "Escape") setOpen(false);
              }}
            >
              <div className="zk-header flex items-center justify-between px-4 sm:px-6">
                <HomeLogo />
                <button
                  type="button"
                  className="btn btn-primary zk-square"
                  onClick={() => {
                    setOpen(false);
                  }}
                  aria-label="Close menu"
                  autoFocus
                >
                  <Icon name="x" size={22} />
                </button>
              </div>
              <div className="flex flex-col px-1 sm:px-2">
                {MENU.map((m) => (
                  <MenuRow
                    key={m.to}
                    {...m}
                    onClick={() => {
                      setOpen(false);
                    }}
                  />
                ))}
              </div>
              <label className="mx-5 mt-4 flex items-center gap-3 text-white font-bold cursor-pointer">
                <input
                  type="checkbox"
                  role="switch"
                  className="zk-switch"
                  checked={sound}
                  onChange={(e) => {
                    setSound(e.target.checked);
                    setSoundOn(e.target.checked);
                  }}
                  data-testid="sound-toggle"
                />
                Sound
                <span className="text-fog font-normal text-sm">{sound ? "on" : "off"}</span>
              </label>
              <div className="mt-auto flex items-center justify-between gap-3 p-5 text-fog text-sm">
                <span>Everyone sees the field. No one sees you.</span>
                {config.data !== undefined && <NetBadge net={config.data.params.network} />}
              </div>
            </motion.nav>
          </>
        )}
      </AnimatePresence>
    </div>
  );
}
