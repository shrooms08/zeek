/**
 * <DigPanel> (design: <DigTray>): tile count, <AmountPicker>, Auto-pick,
 * "Dig separately", optional message, total with fees, <DigButton>, wallet
 * choice. Empty, it shrinks to "Tap tiles to dig" and Auto-pick (1B).
 * `compact` is the mobile bottom sheet (1C).
 *
 * Lite (the default) is that flow. Pro adds <ProTools> (ALL, quick
 * amounts, MAX, auto-dig) and "Your digs"; the mode is remembered in this
 * browser. A running auto-dig shows its Stop button in either mode.
 */

import { FitAmount, useFitInput } from "./FitAmount";
import { motion, useReducedMotion } from "framer-motion";
import { useRef, useState } from "react";
import { Link } from "react-router-dom";
import { digTotal } from "../lib/digPlan";
import type { DigMode } from "../lib/digMode";
import { AMOUNT_STEP, formatZec, parseZec, PRESETS, type Zat } from "../lib/money";
import { EASE } from "../motion/tokens";
import { Icon } from "./Icon";
import { AutoDigCard, ModeSwitch, ProTools, YourDigs } from "./ProTools";
import { DigButton, type ClosedReason } from "./ui";

/** Bytes of a message as the memo stores it (UTF-8). */
export function messageBytes(text: string): number {
  return new TextEncoder().encode(text).length;
}
export const MAX_DIG_MESSAGE_BYTES = 275;

export function AmountPicker(props: {
  value: Zat;
  min: Zat;
  onChange: (v: Zat) => void;
  compact?: boolean;
}) {
  const [text, setText] = useState<string | null>(null);
  const shown = text ?? formatZec(props.value, { decimals: 8 });
  const commit = (raw: string) => {
    try {
      const v = parseZec(raw);
      props.onChange(v < props.min ? props.min : v);
    } catch {
      // keep the previous amount
    }
    setText(null);
  };
  const compact = props.compact === true;
  // Every digit stays visible: the font shrinks to fit the field.
  const inputRef = useRef<HTMLInputElement>(null);
  const mirrorRef = useRef<HTMLSpanElement>(null);
  const fontSize = compact ? 22 : 30;
  const fitted = useFitInput(inputRef, mirrorRef, shown, fontSize, 11);
  const step = (
    <>
      <button
        type="button"
        className="btn zk-square"
        aria-label="Less per tile"
        onClick={() => {
          props.onChange(
            props.value - AMOUNT_STEP < props.min ? props.min : props.value - AMOUNT_STEP,
          );
        }}
      >
        <Icon name="minus" size={22} />
      </button>
      <label className="zk-amount-field relative min-w-0 flex-1 flex flex-col items-center justify-center">
        <span className="absolute left-0 top-0 w-0 h-0 overflow-hidden" aria-hidden="true">
          <span
            ref={mirrorRef}
            className="numeral inline-block invisible whitespace-pre"
            style={{ fontSize, lineHeight: 1 }}
          >
            {shown}
          </span>
        </span>
        <input
          ref={inputRef}
          className="numeral bg-transparent text-center w-full"
          style={{ fontSize: fitted, border: "none", lineHeight: 1, height: fontSize }}
          inputMode="decimal"
          value={shown}
          onChange={(e) => {
            setText(e.target.value);
          }}
          onBlur={(e) => {
            commit(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit((e.target as HTMLInputElement).value);
          }}
          data-testid="amount-input"
          aria-label="ZEC per tile"
        />
        <span className="label text-fog" style={{ fontSize: 10 }} aria-hidden="true">
          {compact ? "Per tile" : "ZEC per tile"}
        </span>
      </label>
      <button
        type="button"
        className="btn zk-square"
        aria-label="More per tile"
        onClick={() => {
          props.onChange(props.value + AMOUNT_STEP);
        }}
      >
        <Icon name="plus" size={22} />
      </button>
    </>
  );
  const presets = PRESETS.map((p) => (
    <button
      key={p.toString()}
      type="button"
      className={`btn zk-preset ${p === props.value ? "btn-primary" : ""}`}
      aria-pressed={p === props.value}
      onClick={() => {
        props.onChange(p);
      }}
    >
      {compact ? formatZec(p).replace(/^0/, "") : formatZec(p)}
    </button>
  ));
  return (
    <div className="flex flex-col gap-2" role="group" aria-label="Amount per tile">
      {compact ? (
        // Narrow phones: the presets drop under the stepper instead of spilling.
        <div className="flex flex-wrap items-stretch gap-2">
          <div className="flex flex-1 items-stretch gap-2 min-w-[200px]">{step}</div>
          <div className="flex items-stretch gap-2">{presets}</div>
        </div>
      ) : (
        <>
          <div className="flex items-stretch gap-2">{step}</div>
          <div className="grid grid-cols-3 gap-2">{presets}</div>
        </>
      )}
    </div>
  );
}

export interface DigPanelProps {
  tiles: number[];
  amountEach: Zat;
  minDig: Zat;
  separate: boolean;
  message: string;
  campfire: boolean;
  closed: boolean;
  /** Why digs are closed, when it is not the round (digging off). */
  closedReason?: ClosedReason | undefined;
  pocketLabel: string;
  /** The mobile bottom sheet. */
  compact?: boolean;
  onAmount: (v: Zat) => void;
  onSeparate: (v: boolean) => void;
  onMessage: (v: string) => void;
  onAutoPick: () => void;
  onClear: () => void;
  onDig: () => void;
  mode: DigMode;
  onMode: (mode: DigMode) => void;
  onTiles: (tiles: number[]) => void;
  /** The Pocket's spendable balance, when it is unlocked (Pro's MAX, auto-dig). */
  spendable: Zat | undefined;
  /** One round's length in ms. */
  roundMs: number;
}

function SeparateSwitch({
  on,
  onChange,
  short,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  short: boolean;
}) {
  return (
    <label className="flex items-center gap-2 text-white cursor-pointer">
      <input
        type="checkbox"
        role="switch"
        checked={on}
        onChange={(e) => {
          onChange(e.target.checked);
        }}
        className="zk-switch"
        data-testid="separate"
      />
      <span className="leading-tight">
        {short ? "Separate" : "Dig separately"}
        {!short && <span className="text-fog text-sm block">more private</span>}
      </span>
    </label>
  );
}

export function DigPanel(p: DigPanelProps) {
  const total = digTotal(p.tiles, p.amountEach, p.separate);
  const n = p.tiles.length;
  const tooLong = messageBytes(p.message) > MAX_DIG_MESSAGE_BYTES;
  const reduce = useReducedMotion() === true;
  const [showMessage, setShowMessage] = useState(p.message !== "");
  const compact = p.compact === true;
  const state = p.closed ? "closed" : n === 0 ? "empty" : tooLong ? "closed" : "idle";
  const pro = p.mode === "pro";
  const modeSwitch = <ModeSwitch mode={p.mode} onChange={p.onMode} />;
  const proTools = pro && (
    <ProTools
      tiles={p.tiles}
      amountEach={p.amountEach}
      minDig={p.minDig}
      separate={p.separate}
      spendable={p.spendable}
      closed={p.closed}
      roundMs={p.roundMs}
      compact={compact}
      onTiles={p.onTiles}
      onAmount={p.onAmount}
    />
  );

  const message = p.campfire && (!compact || showMessage) && (
    <label className="flex flex-col gap-1">
      <span className="label text-fog" style={{ fontSize: 11 }}>
        Campfire message (optional)
      </span>
      <input
        className="field-input"
        value={p.message}
        maxLength={400}
        onChange={(e) => {
          p.onMessage(e.target.value);
        }}
        placeholder="Say something with your dig"
        data-testid="dig-message"
        aria-describedby="message-count"
      />
      <span
        id="message-count"
        className={`text-xs ${tooLong ? "text-white font-bold" : "text-fog"}`}
      >
        {messageBytes(p.message)} / {MAX_DIG_MESSAGE_BYTES} bytes · public, anonymous, permanent
      </span>
    </label>
  );

  const count = (
    <motion.span
      key={n}
      className="numeral inline-block origin-left"
      style={{ fontSize: compact ? 18 : 56, lineHeight: 1 }}
      initial={reduce || n === 0 ? false : { scale: 1.12 }}
      animate={{ scale: 1 }}
      transition={{ duration: 0.16, ease: EASE.out }}
      data-testid="tile-count"
    >
      {n}
    </motion.span>
  );

  if (compact) {
    return (
      <section
        className="zk-panel-compact flex flex-col gap-2"
        aria-label="Dig"
        data-testid="dig-panel"
        data-mode={p.mode}
      >
        <AutoDigCard />
        {modeSwitch}
        <div className="flex items-center gap-2">
          <span className="flex items-center gap-2 text-white font-extrabold flex-1">
            <Icon name="tile" size={18} />
            {count} {n === 1 ? "tile" : "tiles"}
          </span>
          <button
            type="button"
            className="btn flex items-center gap-1.5"
            onClick={p.onAutoPick}
            disabled={p.closed}
            data-testid="auto-pick"
          >
            <Icon name="wand" size={16} /> Auto
          </button>
          <SeparateSwitch on={p.separate} onChange={p.onSeparate} short />
        </div>
        <AmountPicker value={p.amountEach} min={p.minDig} onChange={p.onAmount} compact />
        {proTools}
        {message}
        <DigButton
          state={state}
          count={n}
          onClick={p.onDig}
          total={n > 0 && !p.closed ? formatZec(total.total, { decimals: 8 }) : undefined}
          compact
          closed={p.closedReason}
        />
        <div className="flex justify-between text-xs">
          <span className="text-fog flex items-center gap-1.5">
            <Icon name="wallet" size={14} /> {p.pocketLabel}
          </span>
          <span className="flex gap-3">
            {p.campfire && (
              <button
                type="button"
                className="underline text-white"
                onClick={() => {
                  setShowMessage((s) => !s);
                }}
              >
                Message
              </button>
            )}
            <Link to="/dig/wallet" className="underline text-white" data-testid="other-wallet">
              Other wallet
            </Link>
          </span>
        </div>
        {pro && (
          <details className="text-sm">
            <summary className="cursor-pointer text-white">Your digs</summary>
            <div className="max-h-48 overflow-y-auto mt-2">
              <YourDigs />
            </div>
          </details>
        )}
      </section>
    );
  }

  if (n === 0 && !pro) {
    return (
      <section
        className="flex flex-col gap-4"
        aria-label="Dig"
        data-testid="dig-panel"
        data-mode={p.mode}
      >
        <AutoDigCard />
        {modeSwitch}
        <DigButton state={state} count={0} onClick={p.onDig} closed={p.closedReason} />
        <button
          type="button"
          className="btn flex items-center justify-center gap-2"
          onClick={p.onAutoPick}
          disabled={p.closed}
          data-testid="auto-pick"
        >
          <Icon name="wand" size={18} /> Auto-pick 3 quiet tiles
        </button>
        <span className="sr-only" data-testid="tile-count">
          0
        </span>
        <div className="flex justify-between text-sm">
          <span className="text-fog flex items-center gap-2">
            <Icon name="wallet" size={16} /> {p.pocketLabel}
          </span>
          <Link to="/dig/wallet" className="underline text-white" data-testid="other-wallet">
            Other wallet
          </Link>
        </div>
      </section>
    );
  }

  return (
    <motion.section
      className="panel zk-lift p-6 flex flex-col gap-4 zk-panel-scroll"
      aria-label="Dig"
      data-testid="dig-panel"
      data-mode={p.mode}
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: EASE.out }}
    >
      <AutoDigCard />
      {modeSwitch}
      <div className="flex items-end justify-between">
        <div>
          <div className="label flex items-center gap-2">
            <Icon name="tile" size={16} /> Tiles
          </div>
          {count}
        </div>
        <button type="button" className="underline text-fog hover:text-white" onClick={p.onClear}>
          Clear
        </button>
      </div>
      <AmountPicker value={p.amountEach} min={p.minDig} onChange={p.onAmount} />
      {proTools}
      <div className="flex gap-3 flex-wrap items-center justify-between">
        <button
          type="button"
          className="btn flex items-center gap-2"
          onClick={p.onAutoPick}
          disabled={p.closed}
          data-testid="auto-pick"
        >
          <Icon name="wand" size={18} /> Auto-pick
        </button>
        <SeparateSwitch on={p.separate} onChange={p.onSeparate} short={false} />
      </div>
      {message}
      <div
        className="border-t hair pt-4 flex flex-wrap items-end justify-between gap-3"
        data-testid="total"
        hidden={pro}
      >
        <div className="min-w-0">
          <div className="label flex items-center gap-2">
            <Icon name="coin" size={16} /> Total
          </div>
          <div className="font-mono text-xs text-fog mt-1">
            {n} × {formatZec(p.amountEach, { decimals: 8 })} + fee{" "}
            {formatZec(total.fee, { decimals: 8 })}
          </div>
          <div className="font-mono text-xs text-fog">
            fee 0.00005 per tile + per payment ({total.payments.length})
          </div>
        </div>
        <div className="flex-1 min-w-[8rem]">
          <FitAmount size={30} min={13} align="end" testId="total-amount">
            {formatZec(total.total, { decimals: 8 })}
          </FitAmount>
        </div>
      </div>
      <DigButton state={state} count={n} onClick={p.onDig} closed={p.closedReason} />
      <div className="flex justify-between text-sm">
        <span className="text-fog flex items-center gap-2">
          <Icon name="wallet" size={16} /> {p.pocketLabel}
        </span>
        <Link to="/dig/wallet" className="underline text-white" data-testid="other-wallet">
          Other wallet
        </Link>
      </div>
      {pro && <YourDigs />}
    </motion.section>
  );
}
