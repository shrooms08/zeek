/**
 * Design-system components (names from Zeek Design System § 04), styled to
 * Zeek Mono and animated per Zeek Motion (transform, opacity and
 * stroke-dashoffset only; reduced motion: fades or instant).
 */

import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { Zeb } from "../art/Characters";
import { EASE } from "../motion/tokens";
import { Icon, type IconName } from "./Icon";

/** <NetBadge net />: Testnet dashed with a flask, Mainnet solid with a globe. */
export function NetBadge({ net }: { net: "test" | "main" }) {
  const test = net === "test";
  return (
    <span
      className={`zk-netbadge label flex items-center gap-1.5 px-2.5 py-1 ${test ? "" : "zk-netbadge-main"}`}
      data-testid="net-badge"
    >
      <Icon name={test ? "flask" : "globe"} size={14} strokeWidth={2.5} />
      {test ? "Testnet" : "Mainnet"}
    </span>
  );
}

/** <DigButton state count />: the one big action. */
export function DigButton(props: {
  state: "idle" | "empty" | "loading" | "closed";
  count: number;
  onClick: () => void;
  testId?: string;
  /** Compact sheet: the total sits on the button's right. */
  total?: string | undefined;
  compact?: boolean;
}) {
  const reduce = useReducedMotion() === true;
  if (props.state === "empty") {
    return (
      <button
        type="button"
        className="zk-dig-empty w-full flex items-center justify-center gap-3"
        disabled
        data-testid={props.testId ?? "dig-button"}
      >
        <Icon name="hand" size={24} />
        Tap tiles to dig
      </button>
    );
  }
  const text =
    props.state === "loading"
      ? "Sealing…"
      : props.state === "closed"
        ? "Round closed"
        : props.compact === true
          ? "Dig"
          : `Dig ${props.count} ${props.count === 1 ? "tile" : "tiles"}`;
  return (
    <motion.button
      type="button"
      className="zk-dig w-full flex items-center gap-3"
      style={{ justifyContent: props.total === undefined ? "center" : "space-between" }}
      disabled={props.state !== "idle"}
      onClick={props.onClick}
      {...(reduce || props.state !== "idle" ? {} : { whileTap: { x: 3, y: 3 } })}
      transition={{ duration: 0.09 }}
      data-testid={props.testId ?? "dig-button"}
    >
      <span className="flex items-center gap-3">
        <Icon name="shovel" size={props.compact === true ? 24 : 28} />
        {text}
      </span>
      {props.total !== undefined && (
        <span className="numeral" style={{ fontSize: 20, color: "inherit" }}>
          {props.total} ZEC
        </span>
      )}
    </motion.button>
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
    <section className={`panel p-5 ${className}`} aria-label={label} data-testid={testId}>
      {children}
    </section>
  );
}

/**
 * <Modal title onClose />: one idea per modal, primary action at the bottom,
 * full width. Rises 12 px in 200 ms (out), fades out in 150 ms (in); a
 * bottom sheet on phones. Focus moves in; Esc closes.
 */
export function Modal(props: {
  title: string;
  onClose?: () => void;
  children: ReactNode;
  testId?: string;
  /** Wider body (QR codes side by side on desktop). */
  wide?: boolean;
  /** Dim what's behind (false: the field stays visible, e.g. ink flight). */
  dim?: boolean;
}) {
  const titleId = useId();
  const ref = useRef<HTMLDivElement>(null);
  const reduce = useReducedMotion() === true;
  const onClose = useRef(props.onClose);
  onClose.current = props.onClose;
  useEffect(() => {
    const first = ref.current?.querySelector<HTMLElement>("button, a, input, textarea, [tabindex]");
    first?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose.current?.();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, []);
  return (
    <motion.div
      className={`fixed inset-0 z-40 flex items-end sm:items-center justify-center p-0 sm:p-6 ${props.dim === false ? "pointer-events-none sm:items-end" : "bg-night/80"}`}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.15 }}
    >
      <motion.div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={`panel zk-lift pointer-events-auto w-full ${props.wide === true ? "sm:max-w-[880px]" : "sm:max-w-[520px]"} max-h-[100dvh] overflow-auto p-6 sm:p-8`}
        initial={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.2, ease: EASE.out }}
        data-testid={props.testId}
      >
        <div className="flex items-start justify-between gap-4 mb-5">
          <h2
            id={titleId}
            className="text-heading font-extrabold text-white"
            style={{ fontSize: 28, lineHeight: 1.1 }}
          >
            {props.title}
          </h2>
          {props.onClose !== undefined && (
            <button
              type="button"
              className="btn zk-square"
              onClick={props.onClose}
              aria-label="Close"
            >
              <Icon name="x" size={18} />
            </button>
          )}
        </div>
        {props.children}
      </motion.div>
    </motion.div>
  );
}

/** <StepList />: each step ticks with a hand-drawn check (250 ms). */
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
    <ol className="flex flex-col gap-3" aria-label="Steps">
      {steps.map((s, i) => {
        const state = done === true || i < current ? "done" : i === current ? "now" : "next";
        return (
          <li
            key={s}
            className="flex items-center gap-3"
            data-state={state}
            aria-current={state === "now" ? "step" : undefined}
          >
            <span className={`zk-step zk-step-${state}`}>
              {state === "done" ? <Icon name="checkink" size={18} draw strokeWidth={3} /> : i + 1}
            </span>
            <span className={state === "next" ? "text-fog" : "text-white font-semibold"}>{s}</span>
          </li>
        );
      })}
    </ol>
  );
}

/** <BatchProgress batches current />: one segment per payment (≤ 6 tiles). */
export function BatchProgress({
  batches,
  current,
  fill,
}: {
  batches: readonly number[][];
  current: number;
  /** 0–1 progress of the current payment's segment. */
  fill?: number;
}) {
  const reduce = useReducedMotion() === true;
  return (
    <div data-testid="batch-progress" className="flex flex-col gap-2">
      <div className="flex items-baseline justify-between gap-2">
        <div className="label">
          Payment {Math.min(current + 1, batches.length)} of {batches.length}
        </div>
        <div className="text-fog text-sm">{batches.map((b) => b.length).join(" + ")} tiles</div>
      </div>
      <div className="flex gap-1.5" aria-hidden="true">
        {batches.map((b, k) => (
          <div key={k} className="zk-segment" style={{ flexGrow: Math.max(1, b.length) }}>
            <div
              className="zk-segment-fill"
              style={{
                transform: `scaleX(${k < current ? 1 : k === current ? (fill ?? 0) : 0})`,
                transition: reduce ? "none" : "transform 5s linear",
              }}
            />
          </div>
        ))}
      </div>
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
      className="zk-qr flex flex-col gap-3"
      aria-label={`Payment ${props.index + 1} of ${props.total}`}
      data-testid="qr-card"
      data-uri={props.uri}
    >
      <div className="label">
        Payment {props.index + 1} of {props.total} ·{" "}
        {props.tiles.length === 1
          ? `tile ${(props.tiles[0] ?? 0) + 1}`
          : `${props.tiles.length} tiles`}
      </div>
      <div
        className="bg-white p-3 w-full max-w-[220px] self-center"
        dangerouslySetInnerHTML={{ __html: props.qrSvg }}
        aria-label="QR code of the payment request"
        role="img"
      />
      <div className="zk-amount justify-center">
        <span className="numeral" style={{ fontSize: 32 }}>
          {props.amount}
        </span>
        <span className="zk-unit">ZEC</span>
      </div>
      <p className="text-fog text-sm text-center">
        {props.tiles.length} {props.tiles.length === 1 ? "tile" : "tiles"} · fee included · memo
        filled in
      </p>
      <div className="flex gap-2 justify-center flex-wrap">
        <CopyButton text={props.uri} label="Copy" />
        <a className="btn flex items-center gap-2" href={props.uri}>
          <Icon name="wallet" size={16} /> Open wallet
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

/** Copy: the icon swaps to an ink check for 1.2 s. */
export function CopyButton({ text, label = "Copy" }: { text: string; label?: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const id = window.setTimeout(() => {
      setCopied(false);
    }, 1200);
    return () => {
      window.clearTimeout(id);
    };
  }, [copied]);
  return (
    <button
      type="button"
      className="btn flex items-center gap-2"
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
        });
      }}
    >
      {copied ? (
        <Icon name="checkink" size={16} draw strokeWidth={3} />
      ) : (
        <Icon name="copy" size={16} />
      )}
      <span aria-live="polite">{copied ? "Copied" : label}</span>
    </button>
  );
}

const CHECK_ICONS: Record<string, IconName> = {
  beacon: "dice",
  digs: "chain",
  timing: "clock",
  settlement: "math",
  payout: "coin",
};

/**
 * <CheckRow icon text state />: the icon draws, then a hand-drawn check (or
 * an ink X that shakes). Not applicable: a dash.
 */
export function CheckRow(props: {
  id?: string;
  title: string;
  text: string;
  state: "pending" | "checking" | "pass" | "fail" | "na";
  detail: string[];
  testId?: string;
}) {
  const reduce = useReducedMotion() === true;
  const sr = {
    pending: "waiting",
    checking: "checking",
    pass: "passed",
    fail: "failed",
    na: "not applicable",
  }[props.state];
  const icon = CHECK_ICONS[props.id ?? ""] ?? "check";
  return (
    <li
      className="flex gap-4 py-4 border-b hair"
      data-state={props.state}
      data-testid={props.testId}
    >
      <span className={`zk-check zk-check-${props.state}`} aria-hidden="true">
        <Icon name={icon} size={20} />
      </span>
      <div className="flex-1 min-w-0">
        <div className="text-white font-extrabold text-lg">
          {props.title} <span className="sr-only">: {sr}</span>
        </div>
        <p className={props.state === "fail" ? "text-white" : "text-mist"}>{props.text}</p>
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
      <span className="zk-mark" aria-hidden="true">
        <AnimatePresence initial={false}>
          {props.state === "pass" && (
            <Icon key="pass" name="checkink" size={28} draw strokeWidth={3} />
          )}
          {props.state === "fail" && (
            <motion.span
              key="fail"
              className="inline-flex"
              {...(reduce ? {} : { animate: { x: [0, -6, 6, -4, 4, -2, 0] } })}
              transition={{ duration: 0.3, delay: 0.25 }}
            >
              <Icon name="xink" size={28} draw strokeWidth={3} />
            </motion.span>
          )}
          {props.state === "na" && <span className="text-fog font-black text-xl">–</span>}
          {props.state === "checking" && <span className="zk-ellipsis text-fog font-black">…</span>}
        </AnimatePresence>
      </span>
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
  icon: IconName;
  label: string;
  onClick?: () => void;
}) {
  return (
    <Link
      to={to}
      onClick={onClick}
      className="zk-menurow flex items-center gap-4 px-5 py-4 text-white font-extrabold"
      data-testid={`menu-${to}`}
    >
      <Icon name={icon} size={24} />
      <span className="flex-1">{label}</span>
      <Icon name="arrow" size={20} />
    </Link>
  );
}

/** Loading: Zeb peeks out while we wait. */
export function Loading({ text = "Finding the field…" }: { text?: string }) {
  return (
    <div
      className="flex flex-col items-center justify-center gap-3 p-12 text-fog"
      role="status"
      data-testid="loading"
    >
      <Zeb pose="peek" width={96} label={null} />
      <span className="flex items-center gap-2">
        <span className="zk-hourglass inline-flex">
          <Icon name="hourglass" size={18} />
        </span>
        {text}
      </span>
    </div>
  );
}

/** Empty state: Zeb waits with his lantern. */
export function Empty({ title, text }: { title: string; text: string }) {
  return (
    <div
      className="flex flex-col items-center justify-center gap-2 p-10 text-center"
      data-testid="empty"
    >
      <Zeb pose="idle" width={88} label={null} />
      <h2 className="text-heading font-extrabold text-white">{title}</h2>
      <p className="text-fog">{text}</p>
    </div>
  );
}

/** Offline/error state: Zeb shrugs. */
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
      <Zeb pose="shrug" width={96} label={null} />
      <h2 className="text-heading font-extrabold text-white flex items-center gap-2">
        <Icon name="offline" size={24} /> {text}
      </h2>
      <p className="text-fog">Your digs are safe. We’ll keep trying.</p>
      {onRetry !== undefined && (
        <button type="button" className="btn flex items-center gap-2" onClick={onRetry}>
          <Icon name="refresh" size={18} /> Try again
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
  kicker,
}: {
  title: string;
  children: ReactNode;
  back?: string;
  testId?: string;
  wide?: boolean;
  /** Small caps line above the title. */
  kicker?: string;
}) {
  return (
    <main
      className={`mx-auto w-full ${wide ? "max-w-[1200px]" : "max-w-[720px]"} px-4 sm:px-6 py-6 sm:py-10 flex flex-col gap-6`}
      data-testid={testId}
    >
      <div className="flex items-center gap-4">
        <Link to={back} className="btn zk-square" aria-label="Back">
          <Icon name="back" size={20} />
        </Link>
        <div>
          {kicker !== undefined && <div className="label text-fog">{kicker}</div>}
          <h1
            className="font-black text-white"
            style={{ fontSize: "clamp(30px, 5vw, 44px)", lineHeight: 1.05, fontStretch: "112%" }}
          >
            {title}
          </h1>
        </div>
      </div>
      {children}
    </main>
  );
}
