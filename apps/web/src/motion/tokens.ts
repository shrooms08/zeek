/**
 * Motion tokens (design/mono Zeek Motion § 02–03). Durations in ms; easings
 * as cubic-bezier control points for Framer Motion and as CSS strings.
 */

export const EASE = {
  snap: [0.2, 0.9, 0.2, 1],
  out: [0.16, 1, 0.3, 1],
  in: [0.5, 0, 0.75, 0],
  heave: [0.6, 0, 0.2, 1],
  spring: [0.34, 1.56, 0.64, 1],
} as const satisfies Record<string, readonly [number, number, number, number]>;

export const CSS_EASE = {
  snap: "cubic-bezier(.2,.9,.2,1)",
  out: "cubic-bezier(.16,1,.3,1)",
  in: "cubic-bezier(.5,0,.75,0)",
  heave: "cubic-bezier(.6,0,.2,1)",
  spring: "cubic-bezier(.34,1.56,.64,1)",
} as const;

export const DUR = {
  hover: 120,
  tick: 160,
  pin: 180,
  modal: 200,
  modalOut: 150,
  sheet: 240,
  sheetOut: 180,
  card: 320,
  dig: 300,
  puff: 380,
  shake: 240,
  stage: 800,
  count: 400,
  pulse: 420,
  reducedFade: 150,
} as const;

/** The reveal timeline (ms from its start), one clock for the whole sequence. */
export const REVEAL_AT = {
  dim: 0,
  star: 200,
  crack: 860,
  split: 1700,
  burst: 1950,
  result: 2600,
  /** Deep Cache only. */
  erupt: 2600,
  headline: 3500,
  /** "Not this time": hold, then the new round. */
  hold: 4600,
} as const;
