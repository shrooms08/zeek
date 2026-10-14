/** First-time explainer: three steps, one tap each (told by Zeb in 7b). */

import { useState } from "react";
import { Modal } from "./ui";

const STEPS = [
  {
    title: "Pick tiles",
    text: "One treasure hides in this field every round. Tap the tiles you think it’s under.",
  },
  {
    title: "Dig privately",
    text: "Everyone sees how deep each tile is dug. Nobody sees who dug it.",
  },
  {
    title: "Winners share",
    text: "A public draw picks the treasure tile. Everyone who dug it shares the prize, by how much they dug.",
  },
] as const;

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
  const finish = () => {
    try {
      localStorage.setItem(KEY, "1");
    } catch {
      // private window: show again next time
    }
    onDone();
  };
  const current = STEPS[step] ?? STEPS[0];
  return (
    <Modal title={current.title} onClose={finish} testId="onboarding">
      <p className="text-body text-mist mb-6">{current.text}</p>
      <div className="flex items-center justify-between gap-3">
        <span className="label text-fog">
          {step + 1} / {STEPS.length}
        </span>
        <div className="flex gap-2">
          <button type="button" className="btn" onClick={finish}>
            Skip
          </button>
          <button
            type="button"
            className="btn btn-primary"
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
    </Modal>
  );
}
