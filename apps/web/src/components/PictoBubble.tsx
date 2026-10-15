/**
 * <PictoBubble icons joiners? tail text? />: Zeb speaks in pictures first
 * (design system § 04): at most three pictograms, optional joiner glyphs at
 * 60 % size, an optional one-line caption under the bubble, and an
 * aria-label sentence.
 */

import { motion } from "framer-motion";
import { Fragment } from "react";
import type { IconName } from "../art/icons";
import { EASE } from "../motion/tokens";
import { Icon } from "./Icon";

const WORDS: Partial<Record<IconName, string>> = {
  shovel: "dig",
  tile: "a tile",
  chest: "treasure",
  eye: "everyone sees",
  nobody: "nobody sees who",
  dice: "a fair draw",
  sparkle: "the reveal",
  fire: "the campfire",
  chat: "say something",
  hourglass: "wait for it",
  mine: "the Deep Cache",
  coin: "ZEC",
  split: "winners share",
  person: "you",
  lock: "private",
};

export function PictoBubble({
  icons,
  joiners,
  text = "",
  tail = "bottom",
  reduce = false,
  label,
}: {
  icons: readonly IconName[];
  /** Glyphs between pictograms, e.g. ["+", "="]. */
  joiners?: readonly string[];
  text?: string;
  tail?: "bottom" | "left";
  reduce?: boolean;
  label?: string;
}) {
  const shown = icons.slice(0, 3);
  const sentence = label ?? (text !== "" ? text : shown.map((i) => WORDS[i] ?? i).join(", ") + ".");
  return (
    <motion.div
      className="zk-bubble inline-flex flex-col items-center gap-1"
      initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.9 }}
      animate={reduce ? { opacity: 1 } : { opacity: 1, scale: [0.9, 1.04, 1] }}
      transition={{ duration: reduce ? 0.15 : 0.24, ease: EASE.out }}
      role="img"
      aria-label={sentence}
      data-testid="picto-bubble"
    >
      <div className={`zk-bubble-body zk-bubble-tail-${tail} flex items-center gap-2 px-3 py-2`}>
        {shown.map((name, k) => (
          <Fragment key={`${name}-${k}`}>
            {k > 0 && joiners?.[k - 1] !== undefined && (
              <span className="zk-bubble-joiner" aria-hidden="true">
                {joiners[k - 1]}
              </span>
            )}
            <Icon name={name} size={26} tone="ink" />
          </Fragment>
        ))}
      </div>
      {text !== "" && (
        <span className="text-white text-sm font-bold whitespace-nowrap" aria-hidden="true">
          {text}
        </span>
      )}
    </motion.div>
  );
}
