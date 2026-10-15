/**
 * React mounting for the ink kit's markup.
 *
 * <Markup> mounts generated SVG (numbers and constants only, never user
 * text) inside a <g>. <BoilStack> is the boiling-lines effect: three stacked
 * SVGs, one per line variant, each its own compositor layer, swapped at
 * 9 fps by CSS opacity alone.
 */

import { memo, useId, useMemo, type CSSProperties, type ReactNode, type SVGProps } from "react";
import { boilVariant, defs, safeId } from "./ink";

/** A <g> holding generated ink markup. */
export const Markup = memo(function Markup({
  html,
  ...rest
}: { html: string } & Omit<SVGProps<SVGGElement>, "dangerouslySetInnerHTML" | "children">) {
  return <g {...rest} dangerouslySetInnerHTML={{ __html: html }} />;
});

/** A stable, url()-safe id prefix for this component's patterns and clips. */
export function useInkPrefix(): string {
  return safeId(useId());
}

/** The defs every ink SVG needs (hatches, glows), once per SVG. */
export const InkDefs = memo(function InkDefs({ p }: { p: string }) {
  return <defs dangerouslySetInnerHTML={{ __html: defs(p) }} />;
});

const layerStyle: CSSProperties = {
  position: "absolute",
  inset: 0,
  width: "100%",
  height: "100%",
  overflow: "visible",
  pointerEvents: "none",
};

/**
 * Three boil variants of `render(p)` stacked over one box. `render` gets
 * the SVG's id prefix (each variant is its own SVG with its own defs).
 * `live=false` draws only the first variant (static art, thumbnails).
 */
export function BoilStack({
  viewBox,
  render,
  live = true,
  amp = 0.7,
  className = "",
  children,
}: {
  viewBox: string;
  render: (p: string) => string;
  live?: boolean;
  amp?: number;
  className?: string;
  /** Extra non-boiled content drawn into the first SVG (under the art). */
  children?: ReactNode;
}) {
  const p = useInkPrefix();
  const variants = useMemo(() => {
    const base = render(p);
    return live ? [0, 1, 2].map((v) => boilVariant(base, v, amp)) : [base];
  }, [render, p, live, amp]);
  return (
    <>
      {variants.map((markup, v) => (
        <svg
          key={v}
          viewBox={viewBox}
          style={layerStyle}
          className={live ? `zk-boil zk-boil-${v} ${className}` : className}
          aria-hidden="true"
          focusable="false"
        >
          {v === 0 && (
            <>
              {/* Ids resolve document-wide, so one copy serves all three variants. */}
              <InkDefs p={p} />
              {children}
            </>
          )}
          <Markup html={markup} />
        </svg>
      ))}
    </>
  );
}

export const LAYER_STYLE = layerStyle;
