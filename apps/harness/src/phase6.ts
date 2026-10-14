/** DEV HARNESS sections for Phase 6: Campfire, round verifier, win proofs. */

import {
  RelayClient,
  subscribeCampfire,
  toPocketError,
  type CampfireMessage,
  type CheckResult,
  type Pocket,
  type SecondSource,
} from "@zeek/pocket";
import { formatZec, parseZec } from "./amounts.js";
import { button, el, input, output } from "./dom.js";

type WalletGetter = () => Promise<Pocket>;

function show(target: HTMLElement, value: unknown): void {
  target.classList.remove("error");
  target.textContent = typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function fail(target: HTMLElement, error: unknown): void {
  const e = toPocketError(error);
  target.classList.add("error");
  const partial =
    e.partial === undefined
      ? ""
      : `\nsent: ${JSON.stringify(e.partial.sent)}\nnot sent: ${e.partial.unsent.join(",")}`;
  target.textContent = `${e.code}: ${e.message}${partial}`;
}

async function guarded(target: HTMLElement, action: () => Promise<void>): Promise<void> {
  try {
    await action();
  } catch (error) {
    fail(target, error);
    throw error;
  }
}

function messageRow(m: CampfireMessage, showHidden: boolean): HTMLLIElement {
  const who = m.nickname === undefined ? "anonymous" : `[${m.nickname}]`;
  const where = m.kind === "shout" ? "shout" : `tile ${(m.tiles ?? []).join(",")}`;
  const when = new Date(m.time * 1000).toISOString().slice(11, 19);
  const body = m.hidden
    ? `(hidden: ${m.reason ?? "filtered"})${showHidden && m.text !== undefined ? ` ${m.text}` : ""}`
    : (m.text ?? "");
  return el(
    "li",
    { "data-testid": m.hidden ? "campfire-hidden" : "campfire-message", "data-id": m.id },
    `${when} round ${m.round} ${where} ${formatZec(m.value_zat)} ZEC ${who}: ${body}`,
  );
}

export function campfireSection(engineUrl: string, wallet: WalletGetter): HTMLElement {
  const client = new RelayClient(engineUrl);
  const feed = el("ul", { "data-testid": "campfire-feed", class: "feed" });
  const status = output("campfire-status");
  const showHidden = el("input", { type: "checkbox", "data-testid": "campfire-show-hidden" });
  const text = input("shout-text", "message (280 bytes; start with [name] for a nickname)");
  const amount = input("shout-amount", "ZEC (at least 0.0005)");
  const result = output("shout-result");
  const qr = el("div", { class: "qr", "data-testid": "shout-qr" });
  const uri = el("textarea", { "data-testid": "shout-uri", rows: "4", readonly: "" });

  const load = async (): Promise<void> => {
    try {
      const page = await client.campfire({ limit: 50, includeHidden: showHidden.checked });
      feed.replaceChildren(...page.messages.map((m) => messageRow(m, showHidden.checked)));
      show(status, `${page.messages.length} messages (live updates over the engine's WebSocket)`);
    } catch (error) {
      fail(status, error);
    }
  };
  showHidden.addEventListener("change", () => void load());
  subscribeCampfire(engineUrl, () => void load());
  void load();
  setInterval(() => void load(), 30_000);

  return el(
    "section",
    { "data-testid": "campfire" },
    el("h2", {}, "Campfire"),
    el("label", {}, showHidden, " show hidden text"),
    status,
    feed,
    el("h2", {}, "Shout"),
    text,
    el(
      "div",
      { class: "row" },
      amount,
      button("shout", "Shout from Pocket", () =>
        guarded(result, async () => {
          show(result, "syncing, proving and broadcasting…");
          const sent = await (
            await wallet()
          ).shout({ message: text.value, amountZat: parseZec(amount.value) });
          show(result, sent);
        }),
      ),
      button("shout-request", "Any-wallet shout QR", () =>
        guarded(result, async () => {
          const built = await (
            await wallet()
          ).buildShoutRequest({
            message: text.value,
            amountZat: parseZec(amount.value),
          });
          uri.value = built.uri;
          qr.innerHTML = built.qrSvg;
          show(result, built.notice);
        }),
      ),
    ),
    result,
    qr,
    uri,
  );
}

function checkRow(c: CheckResult): HTMLLIElement {
  const mark = c.status === "pass" ? "PASS" : c.status === "fail" ? "FAIL" : "N/A";
  return el(
    "li",
    { "data-testid": `check-${c.id}`, "data-status": c.status },
    `[${mark}] ${c.title}: ${c.summary}`,
    el("pre", { class: "out" }, c.detail.join("\n")),
  );
}

export function verifierSection(wallet: WalletGetter): HTMLElement {
  const round = input("verify-round", "round");
  const second = input("verify-second", "second source relay URL (optional)");
  const progress = output("verify-progress");
  const checks = el("ol", { "data-testid": "verify-checks" });
  const cross = output("verify-crosscheck");
  return el(
    "section",
    { "data-testid": "verifier" },
    el("h2", {}, "Verify a round (in this browser)"),
    el("div", { class: "row" }, round, second),
    el(
      "div",
      { class: "row" },
      button("verify-run", "Verify", () =>
        guarded(progress, async () => {
          checks.replaceChildren();
          show(cross, "");
          const started = performance.now();
          const url = second.value.trim();
          const secondSource: SecondSource | undefined =
            url.length > 0 ? { kind: "relay", url } : undefined;
          const result = await (
            await wallet()
          ).verifyRound(
            secondSource === undefined
              ? { round: Number(round.value) }
              : { round: Number(round.value), secondSource },
            (p) => {
              show(progress, `${p.step}: ${p.detail}`);
            },
          );
          checks.replaceChildren(...result.report.checks.map(checkRow));
          const passed = result.report.checks.every((c) => c.status !== "fail");
          show(
            progress,
            `round ${result.report.round}: ${passed ? "every available check passed" : "VERIFICATION FAILED"} in ${((performance.now() - started) / 1000).toFixed(1)} s (source: ${result.source})`,
          );
          show(
            cross,
            `cross-check ${result.crossCheck.status.toUpperCase()} (${result.crossCheck.source}): ${result.crossCheck.detail}`,
          );
        }),
      ),
    ),
    progress,
    checks,
    cross,
  );
}

export function winProofSection(wallet: WalletGetter): HTMLElement {
  const round = input("win-round", "round you won");
  const created = output("win-created");
  const target = input("win-verify-input", "proof id or base64url proof");
  const verdict = output("win-verdict");
  const verify = async (value: string): Promise<void> => {
    show(verdict, "verifying…");
    const v = await (await wallet()).verifyWinProof(value);
    const lines = v.steps.map((s) => `[${s.passed ? "ok" : "FAIL"}] ${s.name}: ${s.detail}`);
    show(
      verdict,
      [v.valid ? `VALID: ${v.claim}` : `INVALID: ${v.reason ?? ""}`, ...lines].join("\n"),
    );
    verdict.dataset.valid = String(v.valid);
  };
  const fromUrl = new URLSearchParams(window.location.search).get("winproof");
  if (fromUrl !== null) {
    target.value = fromUrl;
    void guarded(verdict, () => verify(fromUrl));
  }
  return el(
    "section",
    { "data-testid": "win-proof" },
    el("h2", {}, "Prove a win"),
    el(
      "div",
      { class: "row" },
      round,
      button("win-create", "Create proof (Pocket)", () =>
        guarded(created, async () => {
          show(created, "proving in the worker…");
          const shareBase = `${window.location.origin}${window.location.pathname}`;
          const started = performance.now();
          const result = await (await wallet()).createWinProof(Number(round.value), shareBase);
          show(created, {
            id: result.id,
            stored: result.stored,
            shareUrl: result.shareUrl,
            seconds: Number(((performance.now() - started) / 1000).toFixed(1)),
            proofBytes: Math.floor((result.proof.length * 3) / 4),
            verdict: result.verdict.valid ? result.verdict.claim : result.verdict.reason,
          });
          target.value = result.id;
        }),
      ),
    ),
    created,
    el(
      "div",
      { class: "row" },
      target,
      button("win-verify", "Verify proof", () => guarded(verdict, () => verify(target.value))),
    ),
    verdict,
  );
}
