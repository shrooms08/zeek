/**
 * How to play (design 6D): a 20-second explainer over the field. Five
 * steps, four seconds each, the field telling each one; pause and close.
 * Under it, the exact rules with numbers from the engine's config.
 * Reduced motion: the steps still advance, the field cross-fades.
 */

import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useConfig } from "../api/queries";
import type { IconName } from "../art/icons";
import { Icon } from "../components/Icon";
import { StoryField, type Story } from "../components/StoryField";
import { formatZec, zat } from "../lib/money";

const STEPS: { at: number; icon: IconName; text: string; story: Story }[] = [
  { at: 0, icon: "tile", text: "One treasure, 25 tiles", story: "clean" },
  { at: 4, icon: "shovel", text: "Dig tiles with private ZEC", story: "dig" },
  { at: 8, icon: "nobody", text: "Holes show. You don’t.", story: "holes" },
  { at: 12, icon: "dice", text: "Public draw picks the tile", story: "draw" },
  { at: 16, icon: "split", text: "Diggers of that tile share", story: "gold" },
];
const LENGTH = 20;

export default function HowToPlayScreen() {
  const navigate = useNavigate();
  const config = useConfig();
  const p = config.data?.params;
  const [t, setT] = useState(0);
  const [paused, setPaused] = useState(false);
  const close = () => {
    if (window.history.length > 1) void navigate(-1);
    else void navigate("/");
  };
  useEffect(() => {
    if (paused) return;
    const id = window.setInterval(() => {
      setT((x) => (x + 0.25 >= LENGTH ? 0 : x + 0.25));
    }, 250);
    return () => {
      window.clearInterval(id);
    };
  }, [paused]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  });
  const current = [...STEPS].reverse().find((s) => t >= s.at) ?? STEPS[0];
  const clock = (s: number) => `0:${String(Math.floor(s)).padStart(2, "0")}`;
  return (
    <div className="fixed inset-0 z-40 flex items-end sm:items-center justify-center bg-night/80 sm:p-6">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="how-title"
        className="panel zk-lift w-full sm:max-w-[1000px] max-h-[100dvh] overflow-auto"
        data-testid="how-to-play"
      >
        <div className="flex items-center gap-3 px-6 py-4 border-b-[3px] border-white">
          <Icon name="book" size={24} />
          <h2 id="how-title" className="flex-1 text-white font-extrabold text-xl">
            How to play · 20 seconds
          </h2>
          <button
            type="button"
            className="p-2 text-white"
            onClick={() => {
              setPaused((x) => !x);
            }}
            aria-label={paused ? "Play" : "Pause"}
            aria-pressed={paused}
          >
            <Icon name={paused ? "play" : "pause"} size={22} />
          </button>
          <button
            type="button"
            className="p-2 text-white"
            onClick={close}
            aria-label="Close"
            autoFocus
          >
            <Icon name="x" size={22} />
          </button>
        </div>
        <div className="grid sm:grid-cols-[1.2fr_1fr]">
          <div className="flex flex-col items-center justify-center gap-3 p-6 sm:border-r-[3px] sm:border-white">
            <div className="w-full max-w-[420px]">
              <StoryField step={current?.story ?? "clean"} />
            </div>
            <span className="font-mono text-fog text-sm">
              {clock(t)} / {clock(LENGTH)}
            </span>
          </div>
          <ol aria-label="Steps">
            {STEPS.map((s) => {
              const on = s === current;
              return (
                <li
                  key={s.at}
                  className={`flex items-center gap-4 px-6 py-4 border-b hair ${on ? "bg-white text-ink" : "text-white"}`}
                  aria-current={on ? "step" : undefined}
                >
                  <span className={`font-mono text-sm ${on ? "text-ink" : "text-fog"}`}>
                    {clock(s.at)}
                  </span>
                  <Icon name={s.icon} size={24} />
                  <span className="font-extrabold text-lg">{s.text}</span>
                </li>
              );
            })}
          </ol>
        </div>
        <div className="h-1.5 bg-slate" aria-hidden="true">
          <div
            className="h-full bg-white origin-left"
            style={{ transform: `scaleX(${t / LENGTH})`, transition: "transform 250ms linear" }}
          />
        </div>
        <details className="px-6 py-5 text-mist">
          <summary className="cursor-pointer text-white font-bold">The exact rules</summary>
          <ol className="mt-3 flex flex-col gap-2 list-decimal pl-5">
            <li>Pick one or more of the 25 tiles. One of them hides the treasure this round.</li>
            <li>
              Choose how much to dig on each
              {p === undefined ? "" : ` (at least ${formatZec(zat(p.min_dig_zat))} ZEC)`}. Every
              tile’s depth is public; who dug it never is.
            </li>
            <li>
              When the round closes{p === undefined ? "" : ` (every ${p.blocks_per_round} blocks)`},
              a public random draw from drand picks the treasure tile.
            </li>
            <li>
              Everyone who dug the treasure tile shares the prize in proportion to how much they dug
              there
              {p === undefined ? "" : `, after ${p.rake_bps / 100}% goes to the Deep Cache`}.
            </li>
            <li>
              If nobody dug the treasure tile, the pot rolls into the Deep Cache.
              {p === undefined
                ? ""
                : ` Each round there is a 1 in ${p.cache_odds} chance the Deep Cache breaks open.`}
            </li>
            <li>Digs that arrive after the round closes are refunded, minus the network fee.</li>
            <li>
              Every payout is signed by two of three keyholders, and anyone can check every round.
            </li>
          </ol>
          {p?.network === "test" && (
            <p className="text-fog text-sm mt-3">This is testnet: TAZ has no value.</p>
          )}
        </details>
      </div>
    </div>
  );
}
