import { render } from "@testing-library/react";
import { beforeAll, describe, expect, it } from "vitest";
import { burstRays, chestBox } from "../../art/field";
import { ctr } from "../../art/ink";
import { Field, placePins } from "./Field";

const STAGES = Array.from({ length: 25 }, () => 2);

// jsdom has no matchMedia; the field asks it for prefers-reduced-motion.
beforeAll(() => {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      addListener: () => undefined,
      removeListener: () => undefined,
      dispatchEvent: () => false,
    }) as MediaQueryList;
});

/** The treasure's layers, in document (paint) order. */
function layers(container: HTMLElement): string[] {
  const treasure = container.querySelector("[data-treasure]");
  if (treasure === null) throw new Error("no treasure drawn");
  return [...treasure.querySelectorAll("[data-treasure-layer]")].map(
    (el) => el.getAttribute("data-treasure-layer") ?? "",
  );
}

describe("the treasure's draw order", () => {
  it("puts the chest over the tile outline and glow, under the rays (win reveal)", () => {
    const { container } = render(
      <Field stages={STAGES} selected={[7, 12]} treasure={7} burst={3} closed />,
    );
    expect(layers(container)).toEqual(["split", "outline", "glow", "chest", "rays"]);
  });

  it("keeps the same order in the Deep Cache reveal (mine erupting)", () => {
    const { container } = render(
      <Field
        stages={STAGES}
        selected={[]}
        treasure={12}
        burst={3}
        closed
        world
        mineStage={5}
        erupt
        interactive={false}
      />,
    );
    expect(layers(container)).toEqual(["split", "outline", "glow", "chest", "rays"]);
  });

  it("draws no chest before the burst settles, but keeps the order", () => {
    const { container } = render(<Field stages={STAGES} selected={[]} treasure={3} burst={2} />);
    expect(layers(container)).toEqual(["split", "outline", "glow", "rays"]);
  });
});

describe("pins during the reveal", () => {
  it("never stand inside the treasure's chest", () => {
    for (let t = 0; t < 25; t += 1) {
      const [cx, cy] = ctr(t);
      const box = chestBox(cx, cy);
      const insideChest = (x: number, y: number) =>
        x > box.x0 && x < box.x1 && y > box.y0 && y < box.y1;
      for (const pin of placePins([t, (t + 1) % 25, (t + 5) % 25], t)) {
        for (let s = 0; s <= 1; s += 0.05) {
          const x = pin.base[0] + (pin.head[0] - pin.base[0]) * s;
          const y = pin.base[1] + (pin.head[1] - pin.base[1]) * s;
          expect(insideChest(x, y), `pin ${pin.i} for treasure ${t}`).toBe(false);
        }
      }
    }
  });
});

describe("the burst's rays and sparkles", () => {
  it("never cross the chest they are drawn over", () => {
    for (const t of [0, 7, 12, 24]) {
      const [cx, cy] = ctr(t);
      const box = chestBox(cx, cy);
      const inside = ([x, y]: number[]) =>
        (x ?? 0) > box.x0 && (x ?? 0) < box.x1 && (y ?? 0) > box.y0 && (y ?? 0) < box.y1;
      for (const b of [1, 2, 3] as const) {
        const doc = new DOMParser().parseFromString(
          `<svg xmlns="http://www.w3.org/2000/svg">${burstRays(cx, cy, b, true)}</svg>`,
          "image/svg+xml",
        );
        for (const poly of doc.querySelectorAll("polygon")) {
          const [p0, p1, p2] = (poly.getAttribute("points") ?? "")
            .trim()
            .split(/\s+/)
            .map((xy) => xy.split(",").map(Number));
          // Sample the whole wedge: lines from the inner point to the far edge.
          for (let u = 0; u <= 1; u += 0.1) {
            const far = [
              (p1?.[0] ?? 0) + ((p2?.[0] ?? 0) - (p1?.[0] ?? 0)) * u,
              (p1?.[1] ?? 0) + ((p2?.[1] ?? 0) - (p1?.[1] ?? 0)) * u,
            ];
            for (let v = 0; v <= 1; v += 0.05) {
              const pt = [
                (p0?.[0] ?? 0) + ((far[0] ?? 0) - (p0?.[0] ?? 0)) * v,
                (p0?.[1] ?? 0) + ((far[1] ?? 0) - (p0?.[1] ?? 0)) * v,
              ];
              expect(inside(pt), `ray of tile ${t} frame ${b} at ${pt.join(",")}`).toBe(false);
            }
          }
        }
        for (const path of doc.querySelectorAll("path")) {
          const nums = (path.getAttribute("d") ?? "").match(/-?[0-9.]+/g)?.map(Number) ?? [];
          for (let k = 0; k + 1 < nums.length; k += 2) {
            expect(inside([nums[k] ?? 0, nums[k + 1] ?? 0]), `sparkle of tile ${t}`).toBe(false);
          }
        }
      }
    }
  });
});
