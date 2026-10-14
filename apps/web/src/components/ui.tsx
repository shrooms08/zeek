/**
 * Design-system components (names from Zeek Design System § 04). Minimal
 * Mono styling; Phase 7b restyles and animates them in place.
 */

import { motion } from "framer-motion";
import { useEffect, useId, useRef, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { Icon } from "./Icon";

/** <BigStat icon label value unit progress? /> */
export function BigStat(props: {
  icon: string;
  label: string;
  value: string;
  unit?: string;
  gold?: boolean;
  /** 0–1: the last-minute bar. */
  progress?: number | undefined;
  note?: string | undefined;
  stale?: boolean;
  testId?: string;
  size?: "xl" | "l" | "m";
}) {
  const size = props.size ?? "l";
  const fontSize =
    size === "xl"
      ? "clamp(64px, 11vw, 160px)"
      : size === "l"
        ? "clamp(44px, 7vw, 96px)"
        : "clamp(28px, 4vw, 56px)";
  return (
    <section className="panel p-4" aria-label={props.label} data-testid={props.testId}>
      <div className="label flex items-center gap-2">
        <Icon name={props.icon} size={16} />
        {props.label}
        {props.stale === true && (
          <span className="text-fog normal-case tracking-normal font-normal">· reconnecting</span>
        )}
      </div>
      <div className="flex items-baseline gap-2">
        <span
          className="numeral"
          style={{
            fontSize,
            lineHeight: 1,
            color: props.gold === true ? "var(--color-gold)" : undefined,
          }}
        >
          {props.value}
        </span>
        {props.unit !== undefined && <span className="label">{props.unit}</span>}
      </div>
      {props.progress !== undefined && (
        <div className="mt-2 h-2 border border-white" aria-hidden="true">
          <div
            className="h-full bg-white"
            style={{ width: `${Math.round(props.progress * 100)}%` }}
          />
        </div>
      )}
      {props.note !== undefined && <p className="text-fog text-sm mt-1">{props.note}</p>}
    </section>
  );
}

/** <NetBadge net /> */
export function NetBadge({ net }: { net: "test" | "main" }) {
  return (
    <span
      className="label px-2 py-1"
      style={{ border: `2px ${net === "test" ? "dashed" : "solid"} var(--color-white)` }}
      data-testid="net-badge"
    >
      {net === "test" ? "Testnet" : "Mainnet"}
    </span>
  );
}

/** <DigButton state count /> */
export function DigButton(props: {
  state: "idle" | "empty" | "loading" | "closed";
  count: number;
  onClick: () => void;
  testId?: string;
}) {
  const text =
    props.state === "empty"
      ? "Tap tiles to dig"
      : props.state === "loading"
        ? "Sealing…"
        : props.state === "closed"
          ? "Round closed"
          : `Dig ${props.count} ${props.count === 1 ? "tile" : "tiles"}`;
  return (
    <button
      type="button"
      className="btn btn-primary w-full flex items-center justify-center gap-3 uppercase"
      style={{ fontSize: 28, fontWeight: 900, fontStretch: "115%", minHeight: 64 }}
      disabled={props.state !== "idle"}
      onClick={props.onClick}
      data-testid={props.testId ?? "dig-button"}
    >
      <Icon name="shovel" size={24} />
      {text}
    </button>
  );
}

/** <Panel /> */
export function Panel({
  children,
  className = "",
  label,
  testId,
}: {
  children: ReactNode;
  className?: string;
  label?: string;
  testId?: string;
}) {
  return (
    <section className={`panel p-4 ${className}`} aria-label={label} data-testid={testId}>
      {children}
    </section>
  );
}

/** <Modal title onClose /> — one idea per modal, primary action at the bottom. */
export function Modal(props: {
  title: string;
  onClose?: () => void;
  children: ReactNode;
  testId?: string;
}) {
  const titleId = useId();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const first = ref.current?.querySelector<HTMLElement>("button, a, input, textarea, [tabindex]");
    first?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose?.();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [props]);
  return (
    <div className="fixed inset-0 z-40 flex items-end sm:items-center justify-center bg-night/80 p-0 sm:p-6">
      <motion.div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="panel w-full sm:max-w-[560px] max-h-[100dvh] overflow-auto p-6"
        style={{ boxShadow: "var(--shadow-lift)" }}
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.2, ease: [0.2, 0.9, 0.2, 1] }}
        data-testid={props.testId}
      >
        <div className="flex items-start justify-between gap-4 mb-4">
          <h2 id={titleId} className="text-heading font-extrabold text-white">
            {props.title}
          </h2>
          {props.onClose !== undefined && (
            <button type="button" className="btn" onClick={props.onClose} aria-label="Close">
              <Icon name="x" size={18} />
            </button>
          )}
        </div>
        {props.children}
      </motion.div>
    </div>
  );
}

/** <StepList /> with <BatchProgress batches current />. */
export function StepList({
  steps,
  current,
  done,
}: {
  steps: readonly string[];
  current: number;
  done?: boolean;
}) {
  return (
    <ol className="flex flex-col gap-2" aria-label="Steps">
      {steps.map((s, i) => {
        const state = done === true || i < current ? "done" : i === current ? "now" : "next";
        return (
          <li
            key={s}
            className="flex items-center gap-3"
            data-state={state}
            aria-current={state === "now" ? "step" : undefined}
          >
            <span
              className="w-7 h-7 flex items-center justify-center"
              style={{
                border: `3px ${state === "next" ? "dashed" : "solid"} var(--color-white)`,
                background: state === "done" ? "var(--color-white)" : undefined,
                color: state === "done" ? "var(--color-ink)" : undefined,
              }}
            >
              {state === "done" ? <Icon name="check" size={16} /> : i + 1}
            </span>
            <span className={state === "next" ? "text-fog" : "text-white"}>{s}</span>
          </li>
        );
      })}
    </ol>
  );
}

export function BatchProgress({
  batches,
  current,
}: {
  batches: readonly number[][];
  current: number;
}) {
  return (
    <div data-testid="batch-progress">
      <div className="label">
        Payment {Math.min(current + 1, batches.length)} of {batches.length}
      </div>
      <div className="text-fog">{batches.map((b) => b.length).join(" + ")} tiles</div>
    </div>
  );
}

/** <QrCard uri amount index total /> */
export function QrCard(props: {
  uri: string;
  qrSvg: string;
  amount: string;
  index: number;
  total: number;
  tiles: number[];
  status?: string;
}) {
  return (
    <section
      className="panel p-4 flex flex-col gap-3"
      aria-label={`Payment ${props.index + 1} of ${props.total}`}
      data-testid="qr-card"
    >
      <div className="label">
        Payment {props.index + 1} of {props.total} ·{" "}
        {props.tiles.length === 1
          ? `tile ${(props.tiles[0] ?? 0) + 1}`
          : `${props.tiles.length} tiles`}
      </div>
      <div
        className="bg-white p-2 max-w-[240px]"
        dangerouslySetInnerHTML={{ __html: props.qrSvg }}
        aria-label="QR code of the payment request"
        role="img"
      />
      <div className="numeral" style={{ fontSize: 32 }}>
        {props.amount} <span className="label">ZEC</span>
      </div>
      <p className="text-fog text-sm">Plus your wallet’s network fee. The memo is filled in.</p>
      <div className="flex gap-2 flex-wrap">
        <CopyButton text={props.uri} label="Copy request" />
        <a className="btn" href={props.uri}>
          Open wallet
        </a>
      </div>
      {props.status !== undefined && (
        <p className="label" data-testid="payment-status">
          {props.status}
        </p>
      )}
    </section>
  );
}

export function CopyButton({ text, label = "Copy" }: { text: string; label?: string }) {
  return (
    <button
      type="button"
      className="btn flex items-center gap-2"
      onClick={() => {
        void navigator.clipboard.writeText(text);
      }}
    >
      <Icon name="copy" size={16} />
      {label}
    </button>
  );
}

/** <CheckRow icon text state /> */
export function CheckRow(props: {
  title: string;
  text: string;
  state: "pending" | "checking" | "pass" | "fail" | "na";
  detail: string[];
  testId?: string;
}) {
  const mark = { pending: "·", checking: "…", pass: "✓", fail: "✕", na: "–" }[props.state];
  const sr = {
    pending: "waiting",
    checking: "checking",
    pass: "passed",
    fail: "failed",
    na: "not applicable",
  }[props.state];
  return (
    <li
      className="flex gap-3 py-3 border-b hair"
      data-state={props.state}
      data-testid={props.testId}
    >
      <span
        aria-hidden="true"
        className="w-8 h-8 shrink-0 flex items-center justify-center font-black"
        style={{
          border: `3px ${props.state === "fail" || props.state === "pending" ? "dashed" : "solid"} var(--color-white)`,
          background: props.state === "pass" ? "var(--color-white)" : undefined,
          color: props.state === "pass" ? "var(--color-ink)" : undefined,
        }}
      >
        {mark}
      </span>
      <div>
        <div className="text-white font-extrabold">
          {props.title} <span className="sr-only">: {sr}</span>
        </div>
        <p className="text-mist">{props.text}</p>
        {props.detail.length > 0 && (
          <details className="text-fog text-sm mt-1">
            <summary className="cursor-pointer">Technical detail</summary>
            <ul className="font-mono text-xs mt-1 break-all">
              {props.detail.map((d) => (
                <li key={d}>{d}</li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </li>
  );
}

/** <MenuRow /> */
export function MenuRow({
  to,
  icon,
  label,
  onClick,
}: {
  to: string;
  icon: string;
  label: string;
  onClick?: () => void;
}) {
  return (
    <Link
      to={to}
      onClick={onClick}
      className="flex items-center gap-4 p-4 text-white font-extrabold text-heading border-b hair"
      data-testid={`menu-${to}`}
    >
      <Icon name={icon} size={24} />
      {label}
    </Link>
  );
}

/** Loading state. */
export function Loading({ text = "Finding the field…" }: { text?: string }) {
  return (
    <div
      className="flex flex-col items-center justify-center gap-3 p-12 text-fog"
      role="status"
      data-testid="loading"
    >
      <Icon name="hourglass" size={32} />
      {text}
    </div>
  );
}

/** Empty state. */
export function Empty({ title, text }: { title: string; text: string }) {
  return (
    <div
      className="flex flex-col items-center justify-center gap-2 p-12 text-center"
      data-testid="empty"
    >
      <h2 className="text-heading font-extrabold text-white">{title}</h2>
      <p className="text-fog">{text}</p>
    </div>
  );
}

/** Offline/error state. */
export function Offline({
  onRetry,
  text = "Can’t reach the field",
}: {
  onRetry?: () => void;
  text?: string;
}) {
  return (
    <div
      className="flex flex-col items-center justify-center gap-3 p-12 text-center"
      role="alert"
      data-testid="offline"
    >
      <Icon name="offline" size={32} />
      <h2 className="text-heading font-extrabold text-white">{text}</h2>
      <p className="text-fog">Your digs are safe. We’ll keep trying.</p>
      {onRetry !== undefined && (
        <button type="button" className="btn" onClick={onRetry}>
          Try again
        </button>
      )}
    </div>
  );
}

/** Screen wrapper with a back link and title. */
export function Screen({
  title,
  children,
  back = "/",
  testId,
  wide = false,
}: {
  title: string;
  children: ReactNode;
  back?: string;
  testId?: string;
  wide?: boolean;
}) {
  return (
    <main
      className={`mx-auto w-full ${wide ? "max-w-[1200px]" : "max-w-[720px]"} px-4 py-6 flex flex-col gap-6`}
      data-testid={testId}
    >
      <div className="flex items-center gap-3">
        <Link to={back} className="btn" aria-label="Back">
          <Icon name="back" size={18} />
        </Link>
        <h1
          className="text-title font-black text-white"
          style={{ fontSize: "clamp(28px, 5vw, 40px)" }}
        >
          {title}
        </h1>
      </div>
      {children}
    </main>
  );
}
