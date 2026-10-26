/**
 * The in-page check behind e2e/overflow.spec.ts: no text may overflow its
 * box, and the page may not scroll sideways.
 *
 * It looks at every rendered element that carries text (its own text nodes),
 * every button, and every big number (`.numeral`):
 *   - its content may not be wider than its own box (scrollWidth >
 *     clientWidth), unless it is a scroll container or a free-text input
 *     (where text scrolls by design; a number input may not hide digits);
 *   - its box must sit inside its parent's box and inside its nearest
 *     bordered ancestor (a card, badge, button, modal), unless an ancestor
 *     on the way scrolls;
 *   - a fixed or absolutely placed one must sit inside the viewport.
 * SVG art and screen-reader-only text are skipped: the field's illustration
 * deliberately draws past its layers.
 */

import type { Page } from "@playwright/test";

export interface Overflow {
  kind: "content" | "outside-parent" | "outside-box" | "outside-viewport" | "page-scroll";
  what: string;
  text: string;
  detail: string;
}

/**
 * `within`: a selector for the only part of the page on screen (an overlay
 * covering the rest); elements outside it are not checked, page scroll is.
 */
export async function findOverflows(page: Page, within?: string): Promise<Overflow[]> {
  return page.evaluate((within) => {
    const TOL = 1.5;
    const out: {
      kind: "content" | "outside-parent" | "outside-box" | "outside-viewport" | "page-scroll";
      what: string;
      text: string;
      detail: string;
    }[] = [];
    const doc = document.documentElement;
    const describeEarly = (el: Element): string => {
      const id = el.getAttribute("data-testid");
      return `${el.tagName.toLowerCase()}${id === null ? "" : `[data-testid=${id}]`}.${String(el.getAttribute("class")).split(/\s+/).slice(0, 3).join(".")}`;
    };
    if (doc.scrollWidth > doc.clientWidth + TOL) {
      // Name the outermost elements that stick out past the viewport.
      const culprits = [...document.body.querySelectorAll("*")]
        .filter((el) => {
          const r = el.getBoundingClientRect();
          const pr = el.parentElement?.getBoundingClientRect();
          return (
            r.width > 0 &&
            r.right > doc.clientWidth + TOL &&
            !(pr !== undefined && pr.right > doc.clientWidth + TOL)
          );
        })
        .slice(0, 3)
        .map((el) => `${describeEarly(el)} right ${Math.round(el.getBoundingClientRect().right)}`);
      out.push({
        kind: "page-scroll",
        what: "html",
        text: "",
        detail: `scrollWidth ${doc.scrollWidth} > clientWidth ${doc.clientWidth}; ${culprits.join("; ")}`,
      });
    }
    const describe = (el: Element): string => {
      const id = el.getAttribute("data-testid");
      const cls =
        typeof el.className === "string" ? el.className.split(/\s+/).slice(0, 3).join(".") : "";
      return `${el.tagName.toLowerCase()}${id === null ? "" : `[data-testid=${id}]`}${cls === "" ? "" : `.${cls}`}`;
    };
    const scrolls = (el: Element): boolean => {
      const s = getComputedStyle(el);
      return ["auto", "scroll"].includes(s.overflowX);
    };
    // Deliberate truncation (overflow hidden with an ellipsis) is allowed.
    const ellipsis = (el: Element): boolean => {
      const s = getComputedStyle(el);
      return s.textOverflow === "ellipsis" && ["hidden", "clip"].includes(s.overflowX);
    };
    const hidden = (el: Element): boolean => {
      for (let e: Element | null = el; e !== null; e = e.parentElement) {
        const s = getComputedStyle(e);
        if (s.display === "none" || s.visibility === "hidden" || Number(s.opacity) === 0)
          return true;
        if (e.classList.contains("sr-only")) return true;
      }
      return false;
    };
    const bordered = (el: Element): boolean => {
      const s = getComputedStyle(el);
      return (
        parseFloat(s.borderLeftWidth) > 0 &&
        parseFloat(s.borderRightWidth) > 0 &&
        s.borderLeftStyle !== "none"
      );
    };
    const ownText = (el: Element): string =>
      [...el.childNodes]
        .filter((n) => n.nodeType === Node.TEXT_NODE)
        .map((n) => n.textContent ?? "")
        .join("")
        .trim();
    const scope = within === undefined ? document.body : document.querySelector(within);
    const candidates = [...(scope ?? document.body).querySelectorAll("*")].filter(
      (el) =>
        !(el instanceof SVGElement) &&
        (ownText(el) !== "" || el.tagName === "BUTTON" || el.classList.contains("numeral")),
    );
    for (const el of candidates) {
      if (hidden(el)) continue;
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      const text = ((el.textContent as string | null) ?? "").trim().slice(0, 60);
      const what = describe(el);
      // Free text may scroll inside its field; a number may not hide digits.
      const isInput =
        (el.tagName === "INPUT" || el.tagName === "TEXTAREA") && !el.classList.contains("numeral");
      if (
        !isInput &&
        !scrolls(el) &&
        !ellipsis(el) &&
        el.scrollWidth > el.clientWidth + TOL &&
        el.clientWidth > 0
      ) {
        out.push({
          kind: "content",
          what,
          text,
          detail: `scrollWidth ${el.scrollWidth} > clientWidth ${el.clientWidth}`,
        });
      }
      const pos = getComputedStyle(el).position;
      if (pos === "fixed" || pos === "absolute") {
        if (r.left < -TOL || r.right > doc.clientWidth + TOL) {
          out.push({
            kind: "outside-viewport",
            what,
            text,
            detail: `x ${Math.round(r.left)}..${Math.round(r.right)} of ${doc.clientWidth}`,
          });
        }
        continue;
      }
      // Inside the parent and the nearest bordered ancestor (stop at a
      // scroller, which may legitimately hold wider content).
      const parent = el.parentElement;
      for (let a = parent; a !== null && a !== document.body; a = a.parentElement) {
        if (scrolls(a)) break;
        const isParent = a === parent;
        const box = bordered(a);
        if (isParent || box) {
          const pr = a.getBoundingClientRect();
          if (pr.width > 0 && (r.left < pr.left - TOL || r.right > pr.right + TOL)) {
            out.push({
              kind: isParent ? "outside-parent" : "outside-box",
              what,
              text,
              detail: `x ${Math.round(r.left)}..${Math.round(r.right)} outside ${describe(a)} ${Math.round(pr.left)}..${Math.round(pr.right)}`,
            });
            break;
          }
        }
        const ap = getComputedStyle(a).position;
        if ((!isParent && box) || ap === "fixed" || ap === "absolute") break;
      }
    }
    return out;
  }, within);
}
