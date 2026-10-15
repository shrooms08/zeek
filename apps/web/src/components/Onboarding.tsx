/**
 * First-time explainer (design 6A–6C): three steps, Zeb speaking in
 * pictures, the field telling the story (clean → dug → gold). Full screen
 * on phones, a modal on desktop. Remembered in this browser.
 */

import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { Zeb } from "../art/Characters";
import type { IconName } from "../art/icons";
import { EASE } from "../motion/tokens";
import { PictoBubble } from "./PictoBubble";
import { StoryField, type Story } from "./StoryField";

const STEPS: {
  title: string;
  text: string;
  story: Story;
  icons: IconName[];
  joiners: string[];
  bubble: string;
}[] = [
  {
    title: "Pick tiles",
    text: "One treasure hides in this field every round. Tap the tiles you think it’s under.",
    story: "pick",
    icons: ["hand", "tile"],
    joiners: ["+"],
    bubble: "Tap a tile.",
  },
  {
    title: "Dig privately",
    text: "Everyone sees how deep each tile is dug. Nobody sees who dug it.",
    story: "dig",
    icons: ["shovel", "eye", "nobody"],
    joiners: ["→", "·"],
    bubble: "Dig. Everyone sees the hole, nobody sees you.",
  },
  {
    title: "Winners share",
    text: "A public draw picks the treasure tile. Everyone who dug it shares the prize, by how much they dug.",
    story: "gold",
    icons: ["dice", "chest", "split"],
    joiners: ["→", "→"],
    bubble: "A fair draw picks the treasure; its diggers share.",
  },
];

const KEY = "zeek.seenOnboarding";

export function hasSeenOnboarding(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return true;
  }
}

export function Onboarding({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);
  const reduce = useReducedMotion() === true;
  const nextRef = useRef<HTMLButtonElement>(null);
  const finish = () => {
    try {
      localStorage.setItem(KEY, "1");
    } catch {
      // private window: show again next time
    }
    onDone();
  };
  useEffect(() => {
    nextRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") finish();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  });
  const current = STEPS[step] ?? STEPS[0];
  if (current === undefined) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-stretch sm:items-center justify-center bg-night/90">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-title"
        className="zk-onboarding relative flex w-full flex-col gap-4 bg-night p-5 sm:max-w-[520px] sm:border-[3px] sm:border-white sm:p-8 sm:zk-lift"
        data-testid="onboarding"
      >
        <div className="flex items-center justify-between">
          <div className="flex gap-1.5" aria-label={`Step ${step + 1} of ${STEPS.length}`}>
            {STEPS.map((_, k) => (
              <span
                key={k}
                className="h-1.5 w-8"
                style={{ background: k <= step ? "var(--color-white)" : "var(--color-slate)" }}
              />
            ))}
          </div>
          <button type="button" className="text-fog hover:text-white" onClick={finish}>
            Skip
          </button>
        </div>
        <AnimatePresence mode="wait">
          <motion.div
            key={step}
            className="flex flex-col gap-4"
            initial={reduce ? { opacity: 0 } : { opacity: 0, x: 24 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduce ? 0.15 : 0.24, ease: EASE.out }}
          >
            <div className="mx-auto w-full max-w-[380px]">
              <StoryField step={current.story} />
            </div>
            <div className="flex items-end gap-2">
              <Zeb pose={step === 2 ? "chest" : "idle"} gold={step === 2} width={80} label={null} />
              <div className="mb-10">
                <PictoBubble
                  icons={current.icons}
                  joiners={current.joiners}
                  tail="left"
                  label={current.bubble}
                  reduce={reduce}
                />
              </div>
            </div>
            <h2 id="onboarding-title" className="zk-title text-white">
              {current.title}
            </h2>
            <p className="text-body text-mist">{current.text}</p>
          </motion.div>
        </AnimatePresence>
        <button
          ref={nextRef}
          type="button"
          className="zk-dig mt-auto flex items-center justify-center"
          style={{ fontSize: 22, minHeight: 60, textTransform: "none" }}
          onClick={() => {
            if (step + 1 < STEPS.length) setStep(step + 1);
            else finish();
          }}
          data-testid="onboarding-next"
        >
          {step + 1 < STEPS.length ? "Next" : "Start digging"}
        </button>
      </div>
    </div>
  );
}
