/**
 * DEV HARNESS. A plain functional page to exercise Zeek Pocket, the
 * any-wallet dig request and the live board against a running engine. Not
 * the product UI.
 */

import "./style.css";
import {
  Pocket,
  RelayClient,
  toPocketError,
  type BoardState,
  type HistoryEntry,
  type PendingRound,
  type SyncProgress,
} from "@zeek/pocket";
import { formatZec, parseTiles, parseZec } from "./amounts.js";
import { button, el, input, output } from "./dom.js";
import { campfireSection, verifierSection, winProofSection } from "./phase6.js";

const engineUrl: string = import.meta.env.VITE_ZEEK_ENGINE_URL ?? "http://127.0.0.1:8787";

const app = document.getElementById("app");
if (app === null) {
  throw new Error("missing #app");
}

function show(target: HTMLElement, value: unknown): void {
  target.classList.remove("error");
  target.textContent = typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

function fail(target: HTMLElement, error: unknown): void {
  const e = toPocketError(error);
  target.classList.add("error");
  target.textContent = `${e.code}: ${e.message}`;
}

async function guarded(target: HTMLElement, action: () => Promise<void>): Promise<void> {
  try {
    await action();
  } catch (error) {
    fail(target, error);
    throw error;
  }
}

let pocket: Pocket | undefined;
async function wallet(): Promise<Pocket> {
  pocket ??= await Pocket.start(engineUrl, __ZEEK_PINNED_VAULT__);
  return pocket;
}

function progressText(p: SyncProgress): string {
  return `${p.phase}: block ${p.scannedHeight} of ${p.tipHeight} (${Math.round(p.fraction * 100)}%)`;
}

function historyRow(entry: HistoryEntry): HTMLTableRowElement {
  if (entry.type === "received") {
    return el(
      "tr",
      { "data-testid": "history-received" },
      el("td", {}, "received"),
      el("td", {}, String(entry.height)),
      el("td", {}, formatZec(entry.value_zat)),
      el("td", { "data-testid": "history-memo" }, entry.memo ?? "(memo not fetched yet)"),
      el("td", {}, entry.txid),
    );
  }
  const what =
    "dig" in entry.kind
      ? `dig round ${entry.kind.dig.round} tiles ${entry.kind.dig.tiles.join(",")} x ${formatZec(entry.kind.dig.amount_each_zat)}`
      : "shout" in entry.kind
        ? `shout ${formatZec(entry.kind.shout.amount_zat)}: ${entry.kind.shout.message}`
        : `send ${formatZec(entry.kind.send.amount_zat)} to ${entry.kind.send.to.slice(0, 18)}…`;
  return el(
    "tr",
    { "data-testid": "history-sent" },
    el("td", {}, `sent (${entry.status})`),
    el("td", {}, String(entry.mined_height ?? entry.target_height)),
    el("td", {}, `fee ${formatZec(entry.fee_zat)}`),
    el("td", {}, what),
    el("td", {}, entry.txid),
  );
}

function pocketSection(): HTMLElement {
  const status = output("pocket-status");
  const passphrase = input("passphrase", "passphrase (8+ characters)", "password");
  const phrase = el("textarea", {
    "data-testid": "restore-phrase",
    rows: "2",
    placeholder: "24-word phrase (restore only)",
  });
  const birthday = input("restore-birthday", "birthday height (restore only)");
  const address = output("pocket-address");
  const progress = output("sync-progress");
  const balance = output("pocket-balance");
  const tiles = input("dig-tiles", "tiles, e.g. 3,17 or 0-24");
  const amount = input("dig-amount", "ZEC per tile, e.g. 0.001");
  const digResult = output("dig-result");
  const digMessage = input("dig-message", "Campfire message (optional, 275 bytes)");
  const maxPerTx = input("dig-max-per-tx", "tiles per transaction (default 6)");
  const separate = el("input", { type: "checkbox", "data-testid": "dig-separate" });
  const sendTo = input("send-to", "destination address");
  const sendAmount = input("send-amount", "ZEC");
  const sendMemo = input("send-memo", "memo (optional)");
  const sendResult = output("send-result");
  const exported = output("seed-export");
  const historyBody = el("tbody", { "data-testid": "history" });

  const refresh = async (): Promise<void> => {
    const p = await wallet();
    const s = await p.status();
    show(
      status,
      s.unlocked
        ? `unlocked, scanned to ${String(s.scannedHeight)}`
        : s.exists
          ? "locked"
          : "no wallet yet",
    );
    if (s.unlocked) {
      show(address, await p.getAddress());
      const b = await p.getBalance();
      show(
        balance,
        `spendable ${formatZec(b.spendable_zat)} ZEC, pending ${formatZec(b.pending_zat)}, total ${formatZec(b.total_zat)} (scanned ${b.scanned_height})`,
      );
      historyBody.replaceChildren(...(await p.history()).map(historyRow));
    }
  };

  return el(
    "section",
    { "data-testid": "pocket" },
    el("h2", {}, "Pocket (browser wallet)"),
    status,
    el("div", { class: "row" }, passphrase),
    el(
      "div",
      { class: "row" },
      button("create", "Create", () =>
        guarded(status, async () => {
          const created = await (await wallet()).create(passphrase.value);
          show(status, `created, birthday ${created.birthdayHeight}`);
          await refresh();
        }),
      ),
      button("unlock", "Unlock", () =>
        guarded(status, async () => {
          await (await wallet()).unlock(passphrase.value);
          await refresh();
        }),
      ),
      button("lock", "Lock", () =>
        guarded(status, async () => {
          await (await wallet()).lock();
          await refresh();
        }),
      ),
      button("forget", "Forget wallet", () =>
        guarded(status, async () => {
          await (await wallet()).forget();
          await refresh();
        }),
      ),
    ),
    phrase,
    el(
      "div",
      { class: "row" },
      birthday,
      button("restore", "Restore", () =>
        guarded(status, async () => {
          await (await wallet()).restore(phrase.value, Number(birthday.value), passphrase.value);
          await refresh();
        }),
      ),
    ),
    el("div", {}, "Receive address:"),
    address,
    el(
      "div",
      { class: "row" },
      button("sync", "Sync", () =>
        guarded(progress, async () => {
          await (
            await wallet()
          ).sync((p) => {
            show(progress, progressText(p));
          });
          await refresh();
        }),
      ),
      button("refresh", "Refresh", () => guarded(status, refresh)),
    ),
    progress,
    balance,
    el("h2", {}, "Dig"),
    el("div", { class: "row" }, tiles, amount),
    el("div", { class: "row" }, digMessage, maxPerTx, el("label", {}, separate, " dig separately")),
    el(
      "div",
      { class: "row" },
      button("dig", "Dig", () =>
        guarded(digResult, async () => {
          show(digResult, "syncing, proving and broadcasting…");
          const message = digMessage.value.trim();
          const perTx = maxPerTx.value.trim();
          const started = performance.now();
          const log: string[] = [];
          try {
            const result = await (
              await wallet()
            ).dig(
              {
                tiles: parseTiles(tiles.value),
                amountEachZat: parseZec(amount.value),
                ...(message.length > 0 ? { message } : {}),
                ...(perTx.length > 0 ? { maxTilesPerTx: Number(perTx) } : {}),
                separate: separate.checked,
              },
              (b) => {
                log.push(
                  `${((performance.now() - started) / 1000).toFixed(1)}s batch ${b.batch}/${b.batches} [${b.tiles.join(",")}] ${b.phase}${b.txid === undefined ? "" : ` ${b.txid}`}`,
                );
                show(digResult, log.join("\n"));
              },
              (p) => {
                show(digResult, [...log, progressText(p)].join("\n"));
              },
            );
            show(digResult, {
              elapsedSeconds: Math.round((performance.now() - started) / 1000),
              log,
              result,
            });
          } finally {
            await refresh();
          }
        }),
      ),
    ),
    digResult,
    el("h2", {}, "Send"),
    el("div", { class: "row" }, sendTo, sendAmount, sendMemo),
    el(
      "div",
      { class: "row" },
      button("send", "Send", () =>
        guarded(sendResult, async () => {
          show(sendResult, "syncing, proving and broadcasting…");
          const memo = sendMemo.value.trim();
          const result = await (
            await wallet()
          ).send(
            memo.length > 0
              ? { to: sendTo.value.trim(), amountZat: parseZec(sendAmount.value), memo }
              : { to: sendTo.value.trim(), amountZat: parseZec(sendAmount.value) },
          );
          show(sendResult, result);
          await refresh();
        }),
      ),
    ),
    sendResult,
    el("h2", {}, "History"),
    el(
      "table",
      {},
      el(
        "thead",
        {},
        el(
          "tr",
          {},
          ...["what", "height", "amount", "memo / detail", "txid"].map((h) => el("th", {}, h)),
        ),
      ),
      historyBody,
    ),
    el(
      "div",
      { class: "row" },
      button("export-seed", "Export phrase (checks passphrase)", () =>
        guarded(exported, async () => {
          show(exported, await (await wallet()).exportSeed(passphrase.value));
        }),
      ),
    ),
    exported,
  );
}

function anyWalletSection(): HTMLElement {
  const payout = input("payout-address", "your fresh shielded receive address (utest1…)");
  const tiles = input("aw-tiles", "tiles, e.g. 4 or 2,9");
  const amount = input("aw-amount", "ZEC per tile, e.g. 0.001");
  const round = input("aw-round", "round (empty = current)");
  const awMessage = input("aw-message", "Campfire message (optional)");
  const result = output("aw-result");
  const uri = el("textarea", { "data-testid": "aw-uri", rows: "6", readonly: "" });
  const qr = el("div", { class: "qr", "data-testid": "aw-qr" });
  return el(
    "section",
    { "data-testid": "any-wallet" },
    el("h2", {}, "Any wallet (ZIP 321 QR)"),
    el(
      "p",
      {},
      "Paste a fresh receive address from your own wallet as the payout address; it is sealed so only the vault can read it.",
    ),
    payout,
    el("div", { class: "row" }, tiles, amount, round),
    awMessage,
    el(
      "div",
      { class: "row" },
      button("aw-build", "Build request", () =>
        guarded(result, async () => {
          const roundText = round.value.trim();
          const message = awMessage.value.trim();
          const options = {
            payoutAddress: payout.value.trim(),
            tiles: parseTiles(tiles.value),
            amountEachZat: parseZec(amount.value),
            ...(message.length > 0 ? { message } : {}),
          };
          const built = await (
            await wallet()
          ).buildDigRequest(
            roundText.length > 0 ? { ...options, round: Number(roundText) } : options,
          );
          uri.value = built.uri;
          qr.innerHTML = built.qrSvg;
          show(
            result,
            `round ${built.round}, tiles ${built.tiles.join(",")}, total ${formatZec(built.totalZat)} ZEC\n${built.notice}`,
          );
        }),
      ),
    ),
    result,
    qr,
    uri,
  );
}

function boardSection(): HTMLElement {
  const summary = output("board-summary");
  const grid = el("div", { class: "grid", "data-testid": "board-grid" });
  const pending = output("board-pending");
  const client = new RelayClient(engineUrl);
  const render = (board: BoardState, rounds: PendingRound[]): void => {
    show(
      summary,
      [
        `round ${board.round} (${board.status}, rules v${board.rules_version}), ${board.blocks_remaining} blocks left, tip ${board.tip_height}`,
        `prize pool ${formatZec(board.prize_pool_zat)} ZEC (gross dug ${formatZec(board.gross_dug_zat)}, before dig fees and rake)`,
        `Deep Cache ${formatZec(board.cache)} ZEC, ${board.digs} digs`,
      ].join("\n"),
    );
    grid.replaceChildren(
      ...board.depths.map((depth, tile) => {
        const fill = el("div", { class: "fill", style: `height:${depth}%` });
        return el(
          "div",
          { class: "tile", "data-testid": `tile-${tile}` },
          fill,
          el("span", {}, `${tile}: ${depth}`),
        );
      }),
    );
    show(
      pending,
      rounds.length === 0
        ? "no rounds waiting for a reveal"
        : rounds
            .map(
              (r) =>
                `round ${r.round} ${r.status}, beacon ${r.drand_round} at ${new Date(r.beacon_time * 1000).toISOString()}`,
            )
            .join("\n"),
    );
  };
  const poll = async (): Promise<void> => {
    try {
      const [board, rounds] = await Promise.all([client.board(), client.pendingRounds()]);
      render(board, rounds);
    } catch (error) {
      fail(summary, error);
    }
  };
  void poll();
  setInterval(() => void poll(), 10_000);
  return el(
    "section",
    { "data-testid": "board" },
    el("h2", {}, "Live board"),
    summary,
    grid,
    el("div", {}, "Pending reveals:"),
    pending,
  );
}

app.append(
  el(
    "div",
    { class: "banner", "data-testid": "banner" },
    `DEV HARNESS (not the product UI). Engine: ${engineUrl}. Cross-origin isolated: ${String(globalThis.crossOriginIsolated)}`,
  ),
  el(
    "main",
    {},
    pocketSection(),
    anyWalletSection(),
    boardSection(),
    campfireSection(engineUrl, wallet),
    verifierSection(wallet),
    winProofSection(wallet),
  ),
);
