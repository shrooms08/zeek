/**
 * Live check: a payout co-signed by the keyholders, without the faucet.
 * The suite's Pocket sends 0.004 to the test player bob; bob digs 0.002 naming
 * a round far ahead (a wrong-round dig), which the engine must refund; the
 * refund is a payout transaction that two keyholders co-sign. The test waits
 * until it is broadcast and mined, and records the txid and signers.
 *
 *   ZEEK_E2E_APP=https://playzeek.pages.dev pnpm exec playwright test --project=live e2e/live/10
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { expect, test } from "@playwright/test";
import { ENGINE, REPO, engine, withdrawFromPublicPocket, zeekDig } from "./support";

const OUT = path.join(REPO, "docs", "examples", process.env.ZEEK_E2E_EXAMPLES ?? "phase9a");
const log: Record<string, unknown> = { started: new Date().toISOString() };

interface Board {
  round: number;
  status: string;
  blocks_remaining: number;
}
interface Payout {
  state: string;
  txid: string;
  height: number | null;
  signers: number[];
}
interface Round {
  status: string;
  settled?: {
    payouts: { net: number; reason: { reason: string } }[];
    payout: Payout | null;
  };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function spendable(player: string): Promise<number> {
  await zeekDig("sync", player);
  const out = await zeekDig("balance", player);
  return Number(/spendable ([0-9.]+) ZEC/.exec(out)?.[1] ?? "0");
}

test("a wrong-round dig is refunded in a payout co-signed by two keyholders", async () => {
  test.setTimeout(120 * 60_000);
  const address = /utest1[0-9a-z]+/.exec(await zeekDig("address", "bob"))?.[0];
  if (address === undefined) throw new Error("bob has no address");
  log.fundingTxid = await withdrawFromPublicPocket(address, "0.004");

  for (let i = 0; (await spendable("bob")) < 0.0025; i += 1) {
    if (i > 60) throw new Error("bob's coins did not become spendable");
    await sleep(30_000);
  }

  // Dig with time left in the round, naming a round far ahead.
  let board = await engine<Board>("/v1/board");
  while (board.status !== "open" || board.blocks_remaining < 6) {
    await sleep(20_000);
    board = await engine<Board>("/v1/board");
  }
  const wrong = board.round + 100;
  const out = await zeekDig(
    "dig",
    "bob",
    "--tile",
    "5",
    "--amount",
    "0.002",
    "--round",
    String(wrong),
    "--engine",
    ENGINE,
  );
  log.dig = { sentInRound: board.round, memoRound: wrong, txid: /[0-9a-f]{64}/.exec(out)?.[0] };

  // The refund lands in the round the dig is mined in: wait for its payout.
  const deadline = Date.now() + 100 * 60_000;
  for (;;) {
    for (const r of [board.round, board.round + 1, board.round + 2]) {
      const round = await engine<Round>(`/v1/rounds/${r}`).catch(() => undefined);
      const refund = round?.settled?.payouts.find((p) => p.reason.reason === "refund");
      const paid = round?.settled?.payout;
      if (refund !== undefined && paid != null && paid.height !== null) {
        log.round = r;
        log.refundNet = refund.net;
        log.payout = paid;
        log.done = new Date().toISOString();
        mkdirSync(OUT, { recursive: true });
        writeFileSync(
          path.join(OUT, "live-refund-cosign.json"),
          JSON.stringify(log, null, 2) + "\n",
        );
        expect(paid.signers.length).toBeGreaterThanOrEqual(2);
        expect(paid.txid).toMatch(/^[0-9a-f]{64}$/);
        return;
      }
    }
    if (Date.now() > deadline) throw new Error("no refund payout yet");
    await sleep(30_000);
  }
});
