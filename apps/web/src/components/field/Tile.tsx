/**
 * One tile's ink: the top face and grain, the hole at its depth stage, and
 * the treasure's light. Depth reads at a glance and never by colour alone:
 * each stage the hole widens, its walls deepen, the hatching gets denser
 * and darker and the rim heavier.
 *
 * When the depth grows (someone dug), the tile plays the design's landing:
 * a 240 ms shake, the hatch fading in and the rim drawing on (300 ms,
 * stroke-dashoffset). With reduced motion the new hole simply appears.
 * Picking, hover and focus live in <Field>'s hit layer, not here.
 */

import { memo, useEffect, useMemo, useRef } from "react";
import { hole, tileFace, tileKnot } from "../../art/field";
import { Markup } from "../../art/InkSvg";
import { CSS_EASE, DUR } from "../../motion/tokens";
import { prefersReducedMotion } from "../../lib/useNow";

export type TileState = "idle" | "hover" | "selected" | "focus" | "disabled" | "treasure";

export interface TileProps {
  index: number;
  depth: number;
  /** Ink prefix of the SVG's defs. */
  p: string;
  lifted: boolean;
  /** The revealed treasure tile's light ("gold" for the real treasure). */
  lit: "gold" | "white" | null;
  /** Tremble delay (ms) when the reveal's cracks run; null when still. */
  tremble: number | null;
}

export const Tile = memo(function Tile({ index, depth, p, lifted, lit, tremble }: TileProps) {
  const stage = Math.max(0, Math.min(5, depth));
  const ref = useRef<SVGGElement>(null);
  const previous = useRef(stage);
  const html = useMemo(
    () =>
      tileFace(index, lit) +
      (stage === 0 ? tileKnot(index) : "") +
      (stage > 0 ? hole(index, stage, p) : ""),
    [index, lit, stage, p],
  );

  useEffect(() => {
    const was = previous.current;
    previous.current = stage;
    const g = ref.current;
    if (g === null || stage <= was || prefersReducedMotion()) return;
    g.animate(
      [
        { transform: "translate(0,0)" },
        { transform: "translate(-1.6px,.6px)" },
        { transform: "translate(1.6px,-.4px)" },
        { transform: "translate(-1px,0)" },
        { transform: "translate(0,0)" },
      ],
      { duration: DUR.shake, delay: 120 },
    );
    g.querySelector("[data-hatch]")?.animate([{ opacity: 0.2 }, { opacity: 1 }], {
      duration: DUR.dig,
      delay: 100,
      easing: CSS_EASE.out,
      fill: "backwards",
    });
    const rim = g.querySelector<SVGPolygonElement>("[data-rim]");
    if (rim !== null) {
      rim.style.strokeDasharray = "1";
      const a = rim.animate([{ strokeDashoffset: 1 }, { strokeDashoffset: 0 }], {
        duration: DUR.dig,
        delay: 100,
        easing: CSS_EASE.out,
        fill: "backwards",
      });
      a.onfinish = () => {
        rim.style.strokeDasharray = "";
      };
    }
  }, [stage]);

  return (
    <g
      ref={ref}
      data-t={index}
      className={tremble === null ? "zk-tile" : "zk-tile zk-tremble"}
      style={{
        transform: lifted ? "translate(0px,-2px)" : undefined,
        animationDelay: tremble === null ? undefined : `${tremble}ms`,
      }}
    >
      <Markup html={html} />
    </g>
  );
});
