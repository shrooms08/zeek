/** Campfire: anonymous messages that ride on digs or shouts. */

import { toPocketError, type AnyWalletShout } from "@zeek/pocket";
import { useQuery } from "@tanstack/react-query";
import { motion, useReducedMotion } from "framer-motion";
import { useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { engine } from "../api/engine";
import { usePocket } from "../api/PocketProvider";
import { useBoard, useCampfire, useConfig } from "../api/queries";
import type { CampfireMessage } from "../api/types";
import { MAX_DIG_MESSAGE_BYTES, messageBytes } from "../components/DigPanel";
import { Campfire } from "../art/Characters";
import { Icon } from "../components/Icon";
import { Empty, Loading, Offline, QrCard } from "../components/ui";
import { CountUp } from "../motion/CountUp";
import { EASE } from "../motion/tokens";
import { useLoop } from "../sound/cues";
import { formatZec, toNumber, zat } from "../lib/money";
import { formatTimeOfDay } from "../lib/time";
import { useDraft } from "../state/draft";

const MAX_SHOUT_BYTES = 280;
const NICK = /^[A-Za-z0-9_-]{1,16}$/;

/** <MessageRow round tile time text state />: unfolds from the top when it arrives. */
export function MessageRow({
  m,
  shown,
  onShow,
  fresh = false,
}: {
  m: CampfireMessage;
  shown?: string | undefined;
  onShow?: () => void;
  fresh?: boolean;
}) {
  const reduce = useReducedMotion() === true;
  const where =
    m.kind === "shout"
      ? "SHOUT"
      : m.tiles !== undefined && m.tiles.length > 0
        ? `TILE ${m.tiles.map((t) => t + 1).join(" ")}`
        : "DIG";
  return (
    <motion.li
      className="zk-message flex flex-col gap-1 px-5 py-3"
      data-testid="campfire-message"
      style={{ transformOrigin: "top center" }}
      initial={fresh ? (reduce ? { opacity: 0 } : { opacity: 0, scaleY: 0.15 }) : false}
      animate={fresh && !reduce ? { opacity: 1, scaleY: [0.15, 1.04, 1] } : { opacity: 1 }}
      transition={{ duration: reduce ? 0.15 : 0.26, ease: EASE.out }}
    >
      <div className="flex gap-3 font-mono text-xs text-fog">
        <span>R{m.round}</span>
        <span className="text-white font-bold">{where}</span>
        <span>{formatTimeOfDay(m.time)}</span>
        {m.nickname !== undefined && <span className="text-mist">· {m.nickname}</span>}
      </div>
      {m.hidden && shown === undefined ? (
        <button
          type="button"
          className="flex items-center gap-2 text-left text-fog self-start"
          onClick={onShow}
          data-testid="campfire-hidden"
        >
          <Icon name="eye" size={18} /> Hidden. Tap to show
        </button>
      ) : (
        <p className="text-white break-words">{shown ?? m.text}</p>
      )}
    </motion.li>
  );
}

interface Pending {
  text: string;
  at: number;
}

export default function CampfireScreen() {
  // The campfire crackles while this panel is open.
  useLoop("campfire-loop", true);
  const feed = useCampfire(false);
  const config = useConfig();
  const navigate = useNavigate();
  const draft = useDraft();
  const { pocket, wallet } = usePocket();
  const [reveal, setReveal] = useState<Set<string>>(new Set());
  const [filterOn, setFilterOn] = useState(true);
  const hidden = useQuery({
    queryKey: ["campfire", true],
    queryFn: () => engine.campfire(true),
    enabled: reveal.size > 0 || !filterOn,
  });
  const [text, setText] = useState("");
  const [nick, setNick] = useState("");
  const [mode, setMode] = useState<"dig" | "shout">("dig");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<Pending[]>([]);
  const [request, setRequest] = useState<AnyWalletShout | null>(null);
  const board = useBoard();
  const reduce = useReducedMotion() === true;
  const [poked, setPoked] = useState(0);
  const [spark, setSpark] = useState<{ x: number; y: number; dx: number; dy: number } | null>(null);
  const [sparkN, setSparkN] = useState(0);
  const sendRef = useRef<HTMLButtonElement>(null);
  const cacheRef = useRef<HTMLDivElement>(null);
  // Messages present on first load don't unfold; later arrivals do.
  const firstIds = useRef<Set<string>>(new Set());
  const seeded = useRef(false);
  if (!seeded.current && feed.data !== undefined) {
    seeded.current = true;
    firstIds.current = new Set(feed.data.messages.map((m) => m.id));
  }
  /** A shout was sent: an ink spark arcs from Send into the Deep Cache counter. */
  const launchSpark = () => {
    const from = sendRef.current?.getBoundingClientRect();
    const to = cacheRef.current?.getBoundingClientRect();
    setSparkN((n) => n + 1);
    if (from === undefined || to === undefined || reduce) return;
    setSpark({
      x: from.left + from.width / 2 - 11,
      y: from.top - 11,
      dx: to.left + to.width / 2 - (from.left + from.width / 2),
      dy: to.top + to.height / 2 - from.top,
    });
  };

  const minShout = zat(config.data?.params.campfire?.min_shout_zat ?? 50_000);
  const nickOk = nick === "" || NICK.test(nick);
  const message = nick === "" ? text.trim() : `[${nick}] ${text.trim()}`;
  const limit = mode === "dig" ? MAX_DIG_MESSAGE_BYTES : MAX_SHOUT_BYTES;
  const bytes = messageBytes(message);
  const ok = text.trim().length > 0 && nickOk && bytes <= limit;
  const unlocked = wallet.kind === "unlocked" || wallet.kind === "syncing";
  const messages = feed.data?.messages ?? [];
  const stillPending = pending.filter(
    (p) =>
      !messages.some(
        (m) =>
          m.text !== undefined &&
          (p.text === m.text || p.text === `[${m.nickname ?? ""}] ${m.text}`),
      ),
  );

  const send = (viaPocket: boolean) => {
    setError(null);
    if (mode === "dig") {
      draft.setMessage(message);
      void navigate("/");
      return;
    }
    setBusy(true);
    void (async () => {
      try {
        const p = await pocket();
        if (viaPocket) {
          await p.shout({ message, amountZat: toNumber(minShout) });
          setPending((list) => [...list, { text: message, at: Date.now() }]);
          launchSpark();
          setText("");
        } else {
          setRequest(await p.buildShoutRequest({ message, amountZat: toNumber(minShout) }));
        }
      } catch (err) {
        setError(toPocketError(err).message);
      } finally {
        setBusy(false);
      }
    })();
  };

  const cache = zat(board.data?.cache ?? 0);
  const feedView = feed.isPending ? (
    <Loading text="Gathering round the fire…" />
  ) : feed.isError ? (
    <Offline onRetry={() => void feed.refetch()} />
  ) : messages.length === 0 && stillPending.length === 0 ? (
    <Empty title="Quiet tonight" text="Be the first to say something." />
  ) : (
    <ul className="zk-rows" aria-live="polite" data-testid="campfire-feed">
      {stillPending.map((p) => (
        <li key={p.at} className="zk-message px-5 py-3" data-testid="campfire-pending">
          <div className="flex gap-3 font-mono text-xs text-fog">
            <span>R{board.data?.round ?? "…"}</span>
            <span className="text-white font-bold">SHOUT</span>
            <span>now</span>
          </div>
          <p className="text-mist">{p.text}</p>
          <p className="text-fog text-sm flex items-center gap-2">
            <span className="zk-hourglass inline-flex">
              <Icon name="hourglass" size={14} />
            </span>
            On its way · about a minute
          </p>
        </li>
      ))}
      {messages.map((m) => (
        <MessageRow
          key={m.id}
          m={m}
          fresh={!firstIds.current.has(m.id)}
          shown={
            reveal.has(m.id) || !filterOn
              ? (hidden.data?.messages.find((h) => h.id === m.id)?.text ??
                (m.hidden ? "…" : undefined))
              : undefined
          }
          onShow={() => {
            setReveal(new Set(reveal).add(m.id));
          }}
        />
      ))}
    </ul>
  );

  return (
    <main
      className="mx-auto w-full max-w-[1200px] px-4 sm:px-6 py-6 sm:py-10 flex flex-col gap-6"
      data-testid="campfire-screen"
    >
      <div className="flex items-center gap-4 flex-wrap">
        <Link to="/" className="btn zk-square" aria-label="Back">
          <Icon name="back" size={20} />
        </Link>
        <h1
          className="font-black text-white flex items-center gap-3"
          style={{ fontSize: "clamp(30px, 5vw, 44px)", fontStretch: "112%" }}
        >
          <Icon name="fire" size={34} /> Campfire
        </h1>
        <div className="flex-1" />
        <div ref={cacheRef} className="zk-badge" aria-label={`Deep Cache ${formatZec(cache)} ZEC`}>
          <div className="label text-fog flex items-center gap-1.5" style={{ fontSize: 11 }}>
            <Icon name="mine" size={14} /> Deep Cache
          </div>
          <motion.div
            key={sparkN}
            className="numeral"
            style={{ fontSize: 20 }}
            {...(sparkN > 0 && !reduce
              ? { animate: { scale: [1, 1.12, 1] }, transition: { duration: 0.42, delay: 0.7 } }
              : {})}
          >
            <CountUp value={cache} />
          </motion.div>
          {stillPending.length > 0 && (
            <div className="text-fog text-xs">+{formatZec(minShout)} on its way</div>
          )}
        </div>
        <label className="flex items-center gap-2 text-white font-bold cursor-pointer">
          <input
            type="checkbox"
            role="switch"
            className="zk-switch"
            checked={filterOn}
            onChange={(e) => {
              setFilterOn(e.target.checked);
            }}
          />
          Filter
        </label>
      </div>
      <p className="text-fog text-sm">
        Each message shows only its round, tile (or Shout), time and text. A nickname appears only
        if the sender typed one. The filter hides abuse and scams; it never deletes anything.
      </p>
      <div className="grid gap-8 lg:grid-cols-[1fr_1.1fr] items-start">
        <div className="flex flex-col items-center gap-2">
          <Campfire
            width="100%"
            sparks={poked}
            onPoke={() => {
              setPoked((n) => n + 1);
            }}
          />
        </div>
        <div className="flex flex-col gap-5">
          {feedView}
          <form
            className="panel p-5 flex flex-col gap-3"
            aria-label="Say something"
            onSubmit={(e) => {
              e.preventDefault();
              if (ok) send(unlocked);
            }}
          >
            <label className="flex flex-col gap-1">
              <span className="sr-only">Message</span>
              <textarea
                className="field-input"
                rows={2}
                value={text}
                onChange={(e) => {
                  setText(e.target.value);
                }}
                placeholder="Say something…"
                data-testid="campfire-text"
              />
              <span className={`text-xs ${bytes > limit ? "text-white" : "text-fog"}`}>
                {bytes} / {limit} bytes · public, anonymous, permanent
              </span>
            </label>
            <fieldset className="grid grid-cols-2 gap-2">
              <legend className="sr-only">How to send it</legend>
              <button
                type="button"
                className={`btn flex items-center justify-center gap-2 ${mode === "dig" ? "btn-primary" : ""}`}
                aria-pressed={mode === "dig"}
                onClick={() => {
                  setMode("dig");
                }}
              >
                <Icon name="shovel" size={18} /> With my next dig
              </button>
              <button
                type="button"
                className={`btn flex items-center justify-center gap-2 ${mode === "shout" ? "btn-primary" : ""}`}
                aria-pressed={mode === "shout"}
                onClick={() => {
                  setMode("shout");
                }}
                data-testid="campfire-shout-mode"
              >
                <Icon name="shout" size={18} /> Shout · {formatZec(minShout)}
              </button>
            </fieldset>
            <div className="flex gap-2">
              <label className="flex-1">
                <span className="sr-only">Nickname (optional)</span>
                <input
                  className="field-input"
                  value={nick}
                  onChange={(e) => {
                    setNick(e.target.value);
                  }}
                  placeholder="Nickname (optional)"
                  autoComplete="off"
                />
              </label>
              <button
                ref={sendRef}
                type="submit"
                className="btn btn-primary flex items-center gap-2"
                disabled={!ok || busy}
                data-testid="campfire-send"
              >
                <Icon name="send" size={18} />
                {mode === "dig" ? "Add to my dig" : busy ? "Sending…" : unlocked ? "Shout" : "Send"}
              </button>
            </div>
            {!nickOk && (
              <span className="text-white text-sm">Up to 16 letters, digits, _ or -.</span>
            )}
            {mode === "shout" && (
              <p className="text-fog text-sm">
                A shout sends {formatZec(minShout)} ZEC to the Deep Cache, plus the network fee.
              </p>
            )}
            {mode === "shout" && unlocked && (
              <button
                type="button"
                className="underline text-white text-sm self-start"
                disabled={!ok || busy}
                onClick={() => {
                  send(false);
                }}
              >
                Shout from another wallet instead
              </button>
            )}
            {error !== null && (
              <p className="text-white" role="alert">
                {error}
              </p>
            )}
          </form>
          {request !== null && (
            <QrCard
              uri={request.uri}
              qrSvg={request.qrSvg}
              amount={formatZec(zat(request.amountZat), { decimals: 8 })}
              index={0}
              total={1}
              tiles={[]}
              status={request.notice}
            />
          )}
        </div>
      </div>
      {spark !== null && (
        <motion.span
          key={sparkN}
          className="fixed z-50 pointer-events-none text-white"
          style={{ left: spark.x, top: spark.y }}
          initial={{ opacity: 0, x: 0, y: 0, scale: 0.6 }}
          animate={{
            opacity: [0, 1, 0],
            x: [0, spark.dx * 0.4, spark.dx],
            y: [0, spark.dy * 0.5 - 60, spark.dy],
            scale: [0.6, 1.25, 0.5],
          }}
          transition={{ duration: 0.7, ease: EASE.out }}
          onAnimationComplete={() => {
            setSpark(null);
          }}
          aria-hidden="true"
        >
          <Icon name="sparkle" size={22} />
        </motion.span>
      )}
    </main>
  );
}
