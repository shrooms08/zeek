/**
 * The island around the field: ground, props (boiling lines), lamp light,
 * a campfire with Zeb behind the field, stars and fireflies, and the
 * interactive layer that lets people play with the props.
 *
 * Interactions are local delight only (they never touch the chain):
 *   tree   sways and drops a few ink leaves      lamp  flickers off and on
 *   cart   rolls along its rails and back        rock  wobbles
 *   mine   press and hold: rumbles, shows the Deep Cache in a bubble
 *   fire   sparks fly and the Campfire chat opens (when the screen gives
 *          `onCampfire`; otherwise just the sparks)
 *   Zeb    a random pictogram; five quick taps: he digs a hole and pops
 *          out of it.
 * Props sit outside the tile diamond, so they never take a tile pick. They
 * are buttons (Tab reaches them; the field's arrow keys skip them), are
 * disabled during the reveal and while a dig is sealing, and with reduced
 * motion a tap only highlights the prop.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { IconName } from "../../art/icons";
import { BoilStack, LAYER_STYLE, Markup } from "../../art/InkSvg";
import { fireBase, fireFlames, fireGlow, lamp, zeb } from "../../art/props";
import { PROPS, propMarkup, worldGround, type PropDef } from "../../art/world";
import { VIEWBOX_WORLD } from "../../art/ink";
import { prefersReducedMotion } from "../../lib/useNow";
import { PictoBubble } from "../PictoBubble";

export type PropReaction =
  "sway" | "flicker" | "roll" | "wobble" | "rumble" | "poke" | "zeb" | "egg";

/** Where Zeb and his campfire sit: behind the field, between the pines. */
const CAMP = { x: 4, y: -58, scale: 0.34 } as const;
const ZEB_AT = { x: -40, y: -96, scale: 0.36 } as const;

const ZEB_BUBBLES: readonly IconName[][] = [
  ["shovel", "tile", "chest"],
  ["eye", "nobody"],
  ["dice", "sparkle"],
  ["fire", "chat"],
  ["hourglass", "sparkle"],
  ["mine", "coin"],
];

interface Active {
  reaction: PropReaction;
  n: number;
}

function reactionFor(kind: PropDef["kind"]): PropReaction {
  switch (kind) {
    case "tree":
      return "sway";
    case "lamp":
      return "flicker";
    case "cart":
      return "roll";
    case "rock":
      return "wobble";
    case "mine":
      return "rumble";
  }
}

export interface WorldProps {
  mineStage: 1 | 2 | 3 | 4 | 5;
  erupt: boolean;
  /** The Deep Cache, formatted, for the mine's bubble. */
  cacheLabel: string;
  /** False during the reveal and while a dig is sealing. */
  interactive: boolean;
  /** Ambient life (boil, fireflies). False for still frames. */
  live: boolean;
  onReact?: (reaction: PropReaction) => void;
}

/** The back layers: ground, props behind the field, Zeb's camp. */
export function WorldBack({
  mineStage,
  erupt,
  live,
  active,
}: WorldProps & { active: Record<string, Active> }) {
  const ground = useMemo(() => worldGround(), []);
  const back = PROPS.filter((p) => p.layer === "back");
  const render = useCallback(
    (p: string) =>
      [...back]
        .sort((a, b) => a.y - b.y)
        .map((prop) =>
          propGroup(prop, propMarkup(prop, p, mineStage, erupt, true), active[prop.id]),
        )
        .join("") +
      `<g transform="translate(${CAMP.x} ${CAMP.y}) scale(${CAMP.scale})">${fireBase(0, 0)}</g>` +
      `<g transform="translate(${ZEB_AT.x} ${ZEB_AT.y}) scale(${ZEB_AT.scale})">${zebGroup(p, active.zeb)}</g>`,
    // active is part of the drawing: a prop's reaction class lives in its markup.
    [back, mineStage, erupt, active],
  );
  return (
    <>
      <svg viewBox={VIEWBOX_WORLD} style={LAYER_STYLE} aria-hidden="true">
        <Markup html={ground} />
        <Stars live={live} />
      </svg>
      <BoilStack viewBox={VIEWBOX_WORLD} render={render} live={live} amp={0.6} />
      <svg viewBox={VIEWBOX_WORLD} style={LAYER_STYLE} aria-hidden="true">
        <g transform={`translate(${CAMP.x} ${CAMP.y}) scale(${CAMP.scale})`}>
          <Markup html={fireFlames(0, 0)} />
        </g>
      </svg>
    </>
  );
}

/** The front layers: lamp light, props in front of the field, fireflies. */
export function WorldFront({ live, active }: WorldProps & { active: Record<string, Active> }) {
  const front = PROPS.filter((p) => p.layer === "front");
  const render = useCallback(
    (p: string) =>
      front
        .map((prop) => propGroup(prop, propMarkup(prop, p, 1, false, true), active[prop.id]))
        .join(""),
    [front, active],
  );
  const glows = useCallback(
    (p: string) =>
      PROPS.filter((x) => x.kind === "lamp")
        .map(
          (l) => `<g class="${actClass(active[l.id])}">${lamp(l.x, l.y, p, l.scale, "glow")}</g>`,
        )
        .join("") +
      `<g transform="translate(${CAMP.x} ${CAMP.y}) scale(${CAMP.scale})">${fireGlow(0, 0, p)}</g>`,
    [active],
  );
  return (
    <>
      <BoilStack viewBox={VIEWBOX_WORLD} render={glows} live={false} />
      <BoilStack viewBox={VIEWBOX_WORLD} render={render} live={live} amp={0.6} />
      <svg viewBox={VIEWBOX_WORLD} style={LAYER_STYLE} aria-hidden="true">
        {live && <Fireflies />}
        {Object.entries(active)
          .filter(([, a]) => a.reaction === "sway")
          .map(([id, a]) => {
            const prop = PROPS.find((x) => x.id === id);
            return prop === undefined ? null : (
              <Leaves key={`${id}-${a.n}`} x={prop.x} y={prop.y - 50 * prop.scale} />
            );
          })}
        {active.fire !== undefined && (
          <Sparks key={`fire-${active.fire.n}`} x={CAMP.x} y={CAMP.y - 30} />
        )}
      </svg>
    </>
  );
}

const actClass = (a: Active | undefined): string =>
  a === undefined ? "" : `zk-act zk-act-${a.reaction} zk-act-${a.reaction}-${a.n % 2}`;

/** A prop at its spot; its inner group carries the reaction animation. */
function propGroup(prop: PropDef, markup: string, active: Active | undefined): string {
  return `<g transform="translate(${prop.x} ${prop.y})"><g class="zk-prop ${actClass(active)}">${markup}</g></g>`;
}

function zebGroup(p: string, active: Active | undefined): string {
  if (active?.reaction === "egg")
    return `<g class="zk-act zk-act-egg-${active.n % 2}">${zeb("peek", p, false)}</g>`;
  return `<g class="zk-prop ${actClass(active)}">${zeb("campfire", p, false)}</g>`;
}

function Stars({ live }: { live: boolean }) {
  const stars = useMemo(
    () =>
      Array.from({ length: 14 }, (_, k) => ({
        x: -340 + ((k * 97) % 680),
        y: -168 + ((k * 53) % 70),
        r: k % 4 === 0 ? 1.8 : 1.1,
        delay: (k * 0.37) % 1.4,
      })),
    [],
  );
  return (
    <g aria-hidden="true">
      {stars.map((s, k) => (
        <circle
          key={k}
          cx={s.x}
          cy={s.y}
          r={s.r}
          fill="#D6D6D6"
          className={live ? "zk-twinkle" : undefined}
          style={{ animationDelay: `-${s.delay.toFixed(2)}s` }}
        />
      ))}
    </g>
  );
}

function Fireflies() {
  const flies = useMemo(
    () =>
      Array.from({ length: 9 }, (_, k) => ({
        x: -300 + k * 72 + (k % 2) * 18,
        y: 40 + ((k * 41) % 150),
        delay: k * 1.1,
        dur: 7 + (k % 3) * 2,
      })),
    [],
  );
  return (
    <g aria-hidden="true">
      {flies.map((f, k) => (
        <circle
          key={k}
          cx={f.x}
          cy={f.y}
          r={k % 3 ? 1.4 : 2}
          fill="#FFFFFF"
          className="zk-drift"
          style={{ animationDelay: `-${f.delay}s`, animationDuration: `${f.dur}s` }}
        />
      ))}
    </g>
  );
}

function Leaves({ x, y }: { x: number; y: number }) {
  return (
    <g aria-hidden="true">
      {[-14, -2, 10, 20].map((dx, k) => (
        <path
          key={k}
          className="zk-leaf"
          d={`M${x + dx} ${y + k * 4} q3 -3 6 0 q-3 3 -6 0z`}
          fill="#151515"
          stroke="#D6D6D6"
          strokeWidth={1}
          style={
            { animationDelay: `${k * 90}ms`, "--sway": `${k % 2 ? 10 : -10}px` } as CSSProperties
          }
        />
      ))}
    </g>
  );
}

function Sparks({ x, y }: { x: number; y: number }) {
  return (
    <g aria-hidden="true">
      {Array.from({ length: 8 }, (_, k) => {
        const a = -Math.PI / 2 + ((k - 3.5) * Math.PI) / 8;
        return (
          <path
            key={k}
            className="zk-sparkout"
            d={`M${x} ${y - 2.5} v5 M${x - 2.5} ${y} h5`}
            stroke="#FFFFFF"
            strokeWidth={1.6}
            style={
              {
                "--dx": `${(Math.cos(a) * 30).toFixed(1)}px`,
                "--dy": `${(Math.sin(a) * 30).toFixed(1)}px`,
                animationDelay: `${k * 25}ms`,
              } as CSSProperties
            }
          />
        );
      })}
    </g>
  );
}

/** World-unit circle → percentage box over the world's viewBox. */
function hitBox(cx: number, cy: number, r: number): CSSProperties {
  const [vx, vy, vw, vh] = VIEWBOX_WORLD.split(" ").map(Number) as [number, number, number, number];
  return {
    left: `${(((cx - r - vx) / vw) * 100).toFixed(2)}%`,
    top: `${(((cy - r - vy) / vh) * 100).toFixed(2)}%`,
    width: `${(((2 * r) / vw) * 100).toFixed(2)}%`,
    height: `${(((2 * r) / vh) * 100).toFixed(2)}%`,
  };
}

/**
 * Owns the reactions and renders the prop buttons over the world. Returns
 * the active map for the drawing layers (so a reaction is drawn by the
 * boiled art itself).
 */
export function usePropReactions(onReact?: (r: PropReaction) => void) {
  const [active, setActive] = useState<Record<string, Active>>({});
  const timers = useRef<Map<string, number>>(new Map());
  const counter = useRef(0);
  const fire = useCallback(
    (id: string, reaction: PropReaction, ms: number) => {
      counter.current += 1;
      const n = counter.current;
      setActive((a) => ({ ...a, [id]: { reaction, n } }));
      onReact?.(reaction);
      const old = timers.current.get(id);
      if (old !== undefined) window.clearTimeout(old);
      timers.current.set(
        id,
        window.setTimeout(() => {
          setActive((a) => {
            if (a[id]?.n !== n) return a;
            return Object.fromEntries(Object.entries(a).filter(([k]) => k !== id));
          });
        }, ms),
      );
    },
    [onReact],
  );
  useEffect(() => {
    const t = timers.current;
    return () => {
      t.forEach((id) => {
        window.clearTimeout(id);
      });
    };
  }, []);
  return { active, fire };
}

export function PropButtons({
  interactive,
  cacheLabel,
  fire,
  active,
  onCampfire,
}: {
  interactive: boolean;
  cacheLabel: string;
  fire: (id: string, reaction: PropReaction, ms: number) => void;
  active: Record<string, Active>;
  /** Opens the Campfire chat; without it the fire only throws sparks. */
  onCampfire?: (() => void) | undefined;
}) {
  const [bubble, setBubble] = useState<{
    at: { x: number; y: number };
    icons: IconName[];
    text: string;
  } | null>(null);
  const zebTaps = useRef<number[]>([]);
  const hold = useRef<number | null>(null);
  const reduce = prefersReducedMotion();

  useEffect(() => {
    if (bubble === null) return;
    const id = window.setTimeout(() => {
      setBubble(null);
    }, 2600);
    return () => {
      window.clearTimeout(id);
    };
  }, [bubble]);

  const showCache = () => {
    const mine = PROPS.find((p) => p.kind === "mine");
    if (mine === undefined) return;
    fire(mine.id, "rumble", 900);
    setBubble({
      at: { x: mine.hit.cx, y: mine.hit.cy - 50 },
      icons: ["mine", "coin"],
      text: `Deep Cache ${cacheLabel} ZEC`,
    });
  };

  const pokeFire = () => {
    fire("fire", "poke", 900);
    onCampfire?.();
  };

  const tapZeb = () => {
    const now = Date.now();
    zebTaps.current = [...zebTaps.current.filter((t) => now - t < 2000), now];
    if (zebTaps.current.length >= 5) {
      zebTaps.current = [];
      fire("zeb", "egg", 2400);
      setBubble({
        at: { x: ZEB_AT.x + 20, y: ZEB_AT.y - 10 },
        icons: ["shovel", "sparkle"],
        text: "Found you!",
      });
      return;
    }
    fire("zeb", "zeb", 700);
    const icons = ZEB_BUBBLES[Math.floor(Math.random() * ZEB_BUBBLES.length)] ?? ["sparkle"];
    setBubble({ at: { x: ZEB_AT.x + 20, y: ZEB_AT.y - 10 }, icons: [...icons], text: "" });
  };

  const disabled = !interactive;
  return (
    <div className="absolute inset-0" style={{ pointerEvents: "none" }} data-testid="world-props">
      {PROPS.map((prop) => (
        <button
          key={prop.id}
          type="button"
          className="zk-prop-button absolute"
          style={hitBox(prop.hit.cx, prop.hit.cy, prop.hit.r)}
          aria-label={prop.label}
          disabled={disabled}
          data-prop={prop.id}
          data-active={active[prop.id] !== undefined}
          onClick={() => {
            if (prop.kind === "mine") showCache();
            else fire(prop.id, reactionFor(prop.kind), prop.kind === "cart" ? 2600 : 1600);
          }}
          onPointerDown={() => {
            if (prop.kind !== "mine" || disabled) return;
            hold.current = window.setTimeout(showCache, 450);
          }}
          onPointerUp={() => {
            if (hold.current !== null) window.clearTimeout(hold.current);
            hold.current = null;
          }}
          onPointerLeave={() => {
            if (hold.current !== null) window.clearTimeout(hold.current);
            hold.current = null;
          }}
        />
      ))}
      <button
        type="button"
        className="zk-prop-button zk-camp-button absolute"
        style={hitBox(CAMP.x, CAMP.y - 12, 16)}
        aria-label={onCampfire === undefined ? "Poke the campfire" : "Open the Campfire chat"}
        title={onCampfire === undefined ? undefined : "Campfire chat"}
        disabled={disabled}
        data-prop="fire"
        data-testid="campfire-open"
        data-active={active.fire !== undefined}
        onClick={pokeFire}
      />
      <button
        type="button"
        className="zk-prop-button absolute"
        style={hitBox(ZEB_AT.x + 20, ZEB_AT.y + 26, 20)}
        aria-label="Zeb the zebra miner"
        disabled={disabled}
        data-prop="zeb"
        data-testid="zeb"
        data-active={active.zeb !== undefined}
        onClick={tapZeb}
      />
      {bubble !== null && (
        <div
          className="absolute"
          style={{ ...hitBox(bubble.at.x, bubble.at.y, 0), transform: "translate(-50%, -100%)" }}
          role="status"
        >
          <PictoBubble icons={bubble.icons} text={bubble.text} reduce={reduce} />
        </div>
      )}
    </div>
  );
}
