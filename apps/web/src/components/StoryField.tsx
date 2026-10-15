/**
 * <StoryField step>: the explainer's illustration (onboarding, How to play).
 * Not round data: a fixed little story, clean → picked → dug → the draw →
 * gold, drawn with the real field so it looks exactly like the game.
 */

import { Field } from "./field/Field";

export type Story = "clean" | "pick" | "dig" | "holes" | "draw" | "gold";

const ZERO = Array.from({ length: 25 }, () => 0);
const DUG = ZERO.map((_, i) => ([6, 12].includes(i) ? 3 : 0));
const HOLES = ZERO.map((_, i) =>
  [6, 12].includes(i) ? 4 : [2, 8, 16, 18, 22].includes(i) ? (i % 3) + 1 : 0,
);

const STEPS: Record<
  Story,
  {
    stages: number[];
    selected: number[];
    treasure: number | null;
    burst: 0 | 1 | 2 | 3;
    cracks: boolean;
  }
> = {
  clean: { stages: ZERO, selected: [], treasure: null, burst: 0, cracks: false },
  pick: { stages: ZERO, selected: [6, 12], treasure: null, burst: 0, cracks: false },
  dig: { stages: DUG, selected: [6, 12], treasure: null, burst: 0, cracks: false },
  holes: { stages: HOLES, selected: [], treasure: null, burst: 0, cracks: false },
  draw: { stages: HOLES, selected: [], treasure: null, burst: 0, cracks: true },
  gold: { stages: HOLES, selected: [], treasure: 12, burst: 3, cracks: false },
};

export function StoryField({ step }: { step: Story }) {
  const s = STEPS[step];
  return (
    <Field
      stages={s.stages}
      selected={s.selected}
      treasure={s.treasure}
      burst={s.burst}
      closed={step === "draw" || step === "gold"}
      world={false}
      interactive={false}
      reveal={{ cracks: s.cracks, gold: true }}
      decorative
    />
  );
}
