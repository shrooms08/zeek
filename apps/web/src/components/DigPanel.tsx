/**
 * <DigPanel>: <TileCount>, <AmountPicker>, Auto-pick, "Dig separately",
 * optional message, total with fees, <DigButton>, wallet choice.
 */

import { useState } from "react";
import { Link } from "react-router-dom";
import { digTotal } from "../lib/digPlan";
import { AMOUNT_STEP, formatZec, parseZec, PRESETS, type Zat } from "../lib/money";
import { Icon } from "./Icon";
import { DigButton } from "./ui";

/** Bytes of a message as the memo stores it (UTF-8). */
export function messageBytes(text: string): number {
  return new TextEncoder().encode(text).length;
}
export const MAX_DIG_MESSAGE_BYTES = 275;

export function AmountPicker(props: { value: Zat; min: Zat; onChange: (v: Zat) => void }) {
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
  return (
    <div className="flex flex-col gap-2" role="group" aria-label="Amount per tile">
      <div className="flex items-stretch gap-2">
        <button
          type="button"
          className="btn"
          aria-label="Less per tile"
          onClick={() => {
            props.onChange(
              props.value - AMOUNT_STEP < props.min ? props.min : props.value - AMOUNT_STEP,
            );
          }}
        >
          <Icon name="minus" />
        </button>
        <label className="flex-1 panel flex flex-col items-center justify-center p-1">
          <input
            className="numeral bg-transparent text-center w-full"
            style={{ fontSize: 28, border: "none" }}
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
          <span className="label text-fog" aria-hidden="true">
            ZEC per tile
          </span>
        </label>
        <button
          type="button"
          className="btn"
          aria-label="More per tile"
          onClick={() => {
            props.onChange(props.value + AMOUNT_STEP);
          }}
        >
          <Icon name="plus" />
        </button>
      </div>
      <div className="grid grid-cols-3 gap-2">
        {PRESETS.map((p) => (
          <button
            key={p.toString()}
            type="button"
            className={`btn ${p === props.value ? "btn-primary" : ""}`}
            aria-pressed={p === props.value}
            onClick={() => {
              props.onChange(p);
            }}
          >
            {formatZec(p)}
          </button>
        ))}
      </div>
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
  pocketLabel: string;
  onAmount: (v: Zat) => void;
  onSeparate: (v: boolean) => void;
  onMessage: (v: string) => void;
  onAutoPick: () => void;
  onClear: () => void;
  onDig: () => void;
}

export function DigPanel(p: DigPanelProps) {
  const total = digTotal(p.tiles, p.amountEach, p.separate);
  const n = p.tiles.length;
  const tooLong = messageBytes(p.message) > MAX_DIG_MESSAGE_BYTES;
  return (
    <section className="panel p-4 flex flex-col gap-4" aria-label="Dig" data-testid="dig-panel">
      <div className="flex items-end justify-between">
        <div>
          <div className="label flex items-center gap-2">
            <Icon name="tile" size={16} /> Tiles
          </div>
          <div className="numeral" style={{ fontSize: 48 }} data-testid="tile-count">
            {n}
          </div>
        </div>
        {n > 0 && (
          <button type="button" className="underline text-white" onClick={p.onClear}>
            Clear
          </button>
        )}
      </div>
      <AmountPicker value={p.amountEach} min={p.minDig} onChange={p.onAmount} />
      <div className="flex gap-2 flex-wrap items-center">
        <button
          type="button"
          className="btn flex items-center gap-2"
          onClick={p.onAutoPick}
          disabled={p.closed}
          data-testid="auto-pick"
        >
          <Icon name="wand" size={18} /> Auto-pick
        </button>
        <label className="flex items-center gap-2 text-white">
          <input
            type="checkbox"
            checked={p.separate}
            onChange={(e) => {
              p.onSeparate(e.target.checked);
            }}
            className="w-6 h-6"
            data-testid="separate"
          />
          <span>
            Dig separately <span className="text-fog text-sm block">more private</span>
          </span>
        </label>
      </div>
      {p.campfire && (
        <label className="flex flex-col gap-1">
          <span className="label">Message (optional)</span>
          <input
            className="field-input"
            value={p.message}
            maxLength={400}
            onChange={(e) => {
              p.onMessage(e.target.value);
            }}
            placeholder="Say something at the campfire"
            data-testid="dig-message"
            aria-describedby="message-count"
          />
          <span
            id="message-count"
            className={`text-sm ${tooLong ? "text-white font-bold" : "text-fog"}`}
          >
            {messageBytes(p.message)} / {MAX_DIG_MESSAGE_BYTES} bytes · public, anonymous, permanent
          </span>
        </label>
      )}
      <div className="border-t hair pt-3 flex items-end justify-between gap-2" data-testid="total">
        <div>
          <div className="label flex items-center gap-2">
            <Icon name="coin" size={16} /> Total
          </div>
          <div className="font-mono text-mono text-fog">
            {n} × {formatZec(p.amountEach, { decimals: 8 })} + fee{" "}
            {formatZec(total.fee, { decimals: 8 })}
          </div>
          <div className="font-mono text-xs text-fog">
            fee 0.00005 per tile + 0.00005 per payment ({total.payments.length})
          </div>
        </div>
        <div className="numeral" style={{ fontSize: 32 }} data-testid="total-amount">
          {formatZec(total.total, { decimals: 8 })}
        </div>
      </div>
      <DigButton
        state={p.closed ? "closed" : n === 0 ? "empty" : tooLong ? "closed" : "idle"}
        count={n}
        onClick={p.onDig}
      />
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
