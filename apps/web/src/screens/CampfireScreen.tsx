/** Campfire: anonymous messages that ride on digs or shouts. */

import { toPocketError, type AnyWalletShout } from "@zeek/pocket";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { engine } from "../api/engine";
import { usePocket } from "../api/PocketProvider";
import { useCampfire, useConfig } from "../api/queries";
import type { CampfireMessage } from "../api/types";
import { MAX_DIG_MESSAGE_BYTES, messageBytes } from "../components/DigPanel";
import { CopyButton, Empty, Loading, Offline, Screen } from "../components/ui";
import { formatZec, toNumber, zat } from "../lib/money";
import { formatTimeOfDay } from "../lib/time";
import { useDraft } from "../state/draft";

const MAX_SHOUT_BYTES = 280;
const NICK = /^[A-Za-z0-9_-]{1,16}$/;

export function MessageRow({
  m,
  shown,
  onShow,
}: {
  m: CampfireMessage;
  shown?: string | undefined;
  onShow?: () => void;
}) {
  const where =
    m.kind === "shout"
      ? "SHOUT"
      : m.tiles !== undefined && m.tiles.length > 0
        ? `TILE ${m.tiles.map((t) => t + 1).join(" ")}`
        : "DIG";
  return (
    <li className="border-b hair py-3 flex flex-col gap-1" data-testid="campfire-message">
      <div className="flex gap-3 label text-fog">
        <span>R{m.round}</span>
        <span>{where}</span>
        <span>{formatTimeOfDay(m.time)}</span>
        {m.nickname !== undefined && (
          <span className="text-mist normal-case tracking-normal">· {m.nickname}</span>
        )}
      </div>
      {m.hidden && shown === undefined ? (
        <button
          type="button"
          className="text-left text-fog border-2 border-dashed border-ash px-2 py-1 self-start"
          onClick={onShow}
          data-testid="campfire-hidden"
        >
          Hidden. Tap to show
        </button>
      ) : (
        <p className="text-white break-words">{shown ?? m.text}</p>
      )}
    </li>
  );
}

interface Pending {
  text: string;
  at: number;
}

export default function CampfireScreen() {
  const feed = useCampfire(false);
  const config = useConfig();
  const navigate = useNavigate();
  const draft = useDraft();
  const { pocket, wallet } = usePocket();
  const [reveal, setReveal] = useState<Set<string>>(new Set());
  const hidden = useQuery({
    queryKey: ["campfire", true],
    queryFn: () => engine.campfire(true),
    enabled: reveal.size > 0,
  });
  const [text, setText] = useState("");
  const [nick, setNick] = useState("");
  const [mode, setMode] = useState<"dig" | "shout">("dig");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<Pending[]>([]);
  const [request, setRequest] = useState<AnyWalletShout | null>(null);

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

  return (
    <Screen title="Campfire" testId="campfire-screen">
      <p className="text-fog text-sm">
        Each message shows only its round, tile (or Shout), time and text. A nickname appears only
        if the sender typed one.
      </p>
      {feed.isPending ? (
        <Loading text="Gathering round the fire…" />
      ) : feed.isError ? (
        <Offline onRetry={() => void feed.refetch()} />
      ) : messages.length === 0 && stillPending.length === 0 ? (
        <Empty title="Quiet tonight" text="Be the first to say something." />
      ) : (
        <ul aria-live="polite" data-testid="campfire-feed">
          {stillPending.map((p) => (
            <li key={p.at} className="border-b hair py-3 text-fog" data-testid="campfire-pending">
              <div className="label">SHOUT · now</div>
              <p className="text-mist">{p.text}</p>
              <p className="text-sm">On its way · about a minute</p>
            </li>
          ))}
          {messages.map((m) => (
            <MessageRow
              key={m.id}
              m={m}
              shown={
                reveal.has(m.id)
                  ? (hidden.data?.messages.find((h) => h.id === m.id)?.text ?? "…")
                  : undefined
              }
              onShow={() => {
                setReveal(new Set(reveal).add(m.id));
              }}
            />
          ))}
        </ul>
      )}
      <form
        className="panel p-4 flex flex-col gap-3"
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
          <span className={`text-sm ${bytes > limit ? "text-white" : "text-fog"}`}>
            {bytes} / {limit} bytes · public, anonymous, permanent
          </span>
        </label>
        <fieldset className="flex gap-2 flex-wrap">
          <legend className="sr-only">How to send it</legend>
          <button
            type="button"
            className={`btn ${mode === "dig" ? "btn-primary" : ""}`}
            aria-pressed={mode === "dig"}
            onClick={() => {
              setMode("dig");
            }}
          >
            With my next dig
          </button>
          <button
            type="button"
            className={`btn ${mode === "shout" ? "btn-primary" : ""}`}
            aria-pressed={mode === "shout"}
            onClick={() => {
              setMode("shout");
            }}
            data-testid="campfire-shout-mode"
          >
            Shout · {formatZec(minShout)}
          </button>
        </fieldset>
        <label className="flex flex-col gap-1">
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
          {!nickOk && <span className="text-white text-sm">Up to 16 letters, digits, _ or -.</span>}
        </label>
        {mode === "shout" && (
          <p className="text-fog text-sm">
            A shout sends {formatZec(minShout)} ZEC to the Deep Cache, plus the network fee.
          </p>
        )}
        {error !== null && (
          <p className="text-white" role="alert">
            {error}
          </p>
        )}
        <div className="flex gap-2 flex-wrap">
          <button
            type="submit"
            className="btn btn-primary"
            disabled={!ok || busy}
            data-testid="campfire-send"
          >
            {mode === "dig"
              ? "Add to my dig"
              : busy
                ? "Sending…"
                : unlocked
                  ? "Shout with Pocket"
                  : "Send"}
          </button>
          {mode === "shout" && unlocked && (
            <button
              type="button"
              className="btn"
              disabled={!ok || busy}
              onClick={() => {
                send(false);
              }}
            >
              Other wallet
            </button>
          )}
        </div>
      </form>
      {request !== null && (
        <div className="panel p-4 flex flex-col gap-2" data-testid="campfire-qr">
          <div className="label">Shout · {formatZec(zat(request.amountZat))} ZEC</div>
          <div
            className="bg-white p-2 max-w-[220px]"
            role="img"
            aria-label="QR code of the shout payment"
            dangerouslySetInnerHTML={{ __html: request.qrSvg }}
          />
          <p className="text-fog text-sm">{request.notice}</p>
          <CopyButton text={request.uri} label="Copy payment link" />
        </div>
      )}
    </Screen>
  );
}
