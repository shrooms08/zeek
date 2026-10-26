/**
 * <FitAmount>: a big number and its unit that always fit their box.
 *
 * The number keeps every digit; the font shrinks instead. The pair is laid
 * out at `size` px, measured against the container's width, and scaled down
 * (never below `min` px) until it fits, again whenever the container or the
 * value changes. If even `min` is too wide, the unit wraps under the number
 * rather than spilling. The unit is sized relative to the number (`unit` em).
 */

import {
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
  type RefObject,
} from "react";

export function FitAmount({
  children,
  unit,
  size,
  min = 16,
  unitScale = 0.34,
  color,
  lineHeight = 0.95,
  align = "start",
  testId,
  className = "",
}: {
  children: ReactNode;
  /** "ZEC", or none. */
  unit?: string | undefined;
  /** Largest size, in px. */
  size: number;
  /** Smallest size, in px. */
  min?: number;
  /** Unit size relative to the number. */
  unitScale?: number;
  color?: string;
  lineHeight?: number;
  align?: "start" | "center" | "end";
  testId?: string;
  className?: string;
}) {
  const box = useRef<HTMLDivElement>(null);
  const inner = useRef<HTMLSpanElement>(null);
  const [px, setPx] = useState(size);
  useLayoutEffect(() => {
    const outer = box.current;
    const content = inner.current;
    if (outer === null || content === null) return;
    const fit = () => {
      const available = outer.clientWidth;
      // The content's width scales with its font size: its width at the
      // largest size follows from its width now. The size is read from the
      // page, not from the last one asked for: a resize can be observed
      // before React has applied it, and a stale size would spiral down.
      const rendered = parseFloat(getComputedStyle(content).fontSize);
      const natural = (content.scrollWidth / rendered) * size;
      if (available <= 0 || natural <= 0 || !(rendered > 0)) return;
      const next = Math.max(min, Math.min(size, Math.floor((size * available) / natural)));
      if (next !== rendered) setPx(next);
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(outer);
    ro.observe(content);
    return () => {
      ro.disconnect();
    };
  }, [size, min]);
  const justify = align === "center" ? "center" : align === "end" ? "flex-end" : "flex-start";
  const style: CSSProperties = { fontSize: px, lineHeight, justifyContent: justify };
  return (
    <div ref={box} className={`zk-fit ${className}`} data-testid={testId}>
      <span ref={inner} className={`zk-fit-inner ${px <= min ? "zk-fit-wrap" : ""}`} style={style}>
        <span className="numeral" style={color === undefined ? undefined : { color }}>
          {children}
        </span>
        {unit !== undefined && (
          <span className="zk-unit" style={{ fontSize: `${unitScale}em` }}>
            {unit}
          </span>
        )}
      </span>
    </div>
  );
}

/**
 * The same fit for a text input, which cannot report how wide its text is:
 * a hidden copy of the text (`mirror`, same class, laid out at `size` px)
 * is measured against the input's inner width. Returns the font size.
 */
export function useFitInput(
  input: RefObject<HTMLInputElement | null>,
  mirror: RefObject<HTMLSpanElement | null>,
  text: string,
  size: number,
  min: number,
): number {
  const [px, setPx] = useState(size);
  useLayoutEffect(() => {
    const field = input.current;
    const copy = mirror.current;
    if (field === null || copy === null) return;
    const fit = () => {
      const cs = getComputedStyle(field);
      const available =
        field.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight) - 2;
      const natural = copy.offsetWidth;
      if (available <= 0 || natural <= 0) return;
      setPx(Math.max(min, Math.min(size, Math.floor((size * available) / natural))));
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(field);
    ro.observe(copy);
    // A web font arriving late changes the text's width: fit again then
    // (WebKit does not always report it as a resize of the hidden copy).
    document.fonts.addEventListener("loadingdone", fit);
    void document.fonts.ready.then(fit);
    return () => {
      ro.disconnect();
      document.fonts.removeEventListener("loadingdone", fit);
    };
  }, [input, mirror, text, size, min]);
  return px;
}
