/**
 * The wallet worker: owns the WebAssembly wallet core, the relay client and
 * IndexedDB. The seed and keys exist only inside this worker's wasm memory
 * while unlocked; nothing secret is posted back except the recovery phrase on
 * an explicit, passphrase-checked export.
 */

import init, {
  blockHashes,
  buildDigRequest as coreBuildDigRequest,
  buildShoutRequest as coreBuildShoutRequest,
  decodeWinProof,
  initThreadPool,
  PocketCore,
  verifyPlan,
  verifyRound as coreVerifyRound,
  verifyScan,
  verifyWinProof as coreVerifyWinProof,
} from "../wasm/zeek_pocket.js";
import {
  CHANGE_POLL_MS,
  CHANGE_WAIT_MS,
  describePartial,
  planBatches,
  privacyGapMs,
} from "./batching.js";
import { blockchairHash, compareHashes, sourceName, type BlockHash } from "./crosscheck.js";
import { cryptoRandom, planTransactionFetches, type TxRef } from "./decoys.js";
import { PocketError, toPocketError } from "./errors.js";
import type { Methods, MethodName, RequestMessage, ResponseMessage } from "./protocol.js";
import { hexToBytes, MAX_BLOCK_RANGE, RelayClient } from "./relay.js";
import { WalletStorage } from "./storage.js";
import type {
  AnyWalletDig,
  AnyWalletShout,
  Balance,
  BuiltTransaction,
  CrossCheck,
  DigBatch,
  DigProgress,
  HistoryEntry,
  RoundReport,
  ScanReport,
  SyncProgress,
  VerifyProgress,
  WinVerdict,
} from "./types.js";

/** Blocks fetched per relay request while syncing. */
const SYNC_BATCH = 200;
/** Decoy transactions fetched per wanted one. */
const DECOYS_PER_WANTED = 3;
/** Rewinds attempted on one sync before giving up. */
const MAX_REWINDS = 20;
/** Times a dig retries a failed read (sync, config, board) per batch. */
const READ_RETRIES = 4;
/** Wait between those retries. */
const READ_RETRY_MS = 15_000;
/** Syncs attempted before building, when blocks keep arriving meanwhile. */
const MAX_CATCH_UP = 5;

const scope = self as unknown as DedicatedWorkerGlobalScope;
const storage = new WalletStorage();
let relay: RelayClient | undefined;
let core: PocketCore | undefined;
let ready: Promise<number> | undefined;
/** The wasm module's linear memory (shared with the rayon threads). */
let wasmMemory: WebAssembly.Memory | undefined;
let queue: Promise<unknown> = Promise.resolve();

function post(message: ResponseMessage): void {
  scope.postMessage(message);
}

async function loadWasm(): Promise<number> {
  if (!globalThis.crossOriginIsolated) {
    throw new PocketError(
      "not_isolated",
      "the page is not cross-origin isolated (COOP/COEP headers), so the wallet cannot use threads",
    );
  }
  const output = await init();
  wasmMemory = output.memory;
  const threads = Math.max(1, Math.min(navigator.hardwareConcurrency || 4, 8));
  await initThreadPool(threads);
  return threads;
}

function wasm(): Promise<number> {
  ready ??= loadWasm();
  return ready;
}

function needRelay(): RelayClient {
  if (relay === undefined) {
    throw new PocketError("internal", "the worker was not initialised");
  }
  return relay;
}

function needCore(): PocketCore {
  if (core === undefined) {
    throw new PocketError("locked", "the wallet is locked; unlock it first");
  }
  return core;
}

async function persist(): Promise<void> {
  await storage.saveState(needCore().sealedState());
}

async function openFromTreeState(opened: {
  seedEnvelope: string;
  takeCore(): PocketCore;
}): Promise<{ address: string; birthdayHeight: number }> {
  core?.free();
  const c = opened.takeCore();
  core = c;
  await storage.saveAll({ seedEnvelope: opened.seedEnvelope, state: c.sealedState() });
  return { address: c.receiveAddress(), birthdayHeight: c.birthdayHeight() };
}

async function sync(id: number): Promise<Balance> {
  const c = needCore();
  const client = needRelay();
  const tip = (await client.tip()).height;
  const start = c.scannedHeight();
  const progress = (phase: SyncProgress["phase"]): void => {
    const span = Math.max(1, tip - start);
    post({
      id,
      type: "progress",
      progress: {
        scannedHeight: c.scannedHeight(),
        tipHeight: tip,
        birthdayHeight: c.birthdayHeight(),
        fraction: Math.min(1, Math.max(0, (c.scannedHeight() - start) / span)),
        phase,
      },
    });
  };
  let rewinds = 0;
  while (c.scannedHeight() < tip) {
    const from = c.scannedHeight() + 1;
    const to = Math.min(from + Math.min(SYNC_BATCH, MAX_BLOCK_RANGE) - 1, tip);
    const blocks = await client.blocks(from, to);
    try {
      c.scan(blocks) as ScanReport;
    } catch (error) {
      const e = toPocketError(error);
      if (e.code !== "reorg" || e.height === undefined || rewinds >= MAX_REWINDS) {
        throw e;
      }
      rewinds += 1;
      // The block at `height` does not extend ours: our block below it is
      // stale. Step back one more block and fetch again.
      c.rewind(Math.max(c.birthdayHeight(), e.height - 2));
    }
    await persist();
    progress("blocks");
  }
  await fetchMemos(c, client);
  await persist();
  progress("done");
  return c.balance() as Balance;
}

/**
 * Syncs, then reads the chain tip again and syncs once more if a block
 * arrived meanwhile, so a transaction is built against the tip the wallet
 * has scanned. Returns that tip.
 */
async function syncToTip(id: number): Promise<number> {
  const c = needCore();
  for (let attempt = 0; attempt < MAX_CATCH_UP; attempt += 1) {
    await sync(id);
    const tip = (await needRelay().tip()).height;
    if (c.scannedHeight() >= tip) {
      return tip;
    }
  }
  throw new PocketError("not_synced", "the chain kept advancing while syncing; try again");
}

async function fetchMemos(c: PocketCore, client: RelayClient): Promise<void> {
  const requests = c.memoRequests() as { wanted: TxRef[]; decoys: TxRef[] };
  if (requests.wanted.length === 0) {
    return;
  }
  const plan = planTransactionFetches(
    requests.wanted,
    requests.decoys,
    DECOYS_PER_WANTED,
    cryptoRandom,
  );
  for (const fetchItem of plan) {
    const bytes = await client.transaction(fetchItem.txid);
    if (fetchItem.wanted) {
      c.attachRelayTransaction(bytes);
    }
  }
}

async function broadcastBuilt(c: PocketCore, built: BuiltTransaction): Promise<void> {
  // The state (spent notes reserved, fresh diversifier indices consumed) is
  // saved before the transaction leaves, so an index is never reused.
  await persist();
  try {
    await needRelay().broadcast(hexToBytes(built.rawHex));
  } catch (error) {
    c.abandon(built.txid);
    await persist();
    throw error;
  }
  c.markBroadcast(built.txid);
  await persist();
}

function digProgress(id: number, progress: DigProgress): void {
  post({ id, type: "dig-progress", progress });
}

function verifyProgress(id: number, step: VerifyProgress["step"], detail: string): void {
  post({ id, type: "verify-progress", progress: { step, detail } });
}

/** Fetches relay-encoded compact blocks `start..=end` in chunks. */
async function fetchBlocks(client: RelayClient, start: number, end: number): Promise<Uint8Array> {
  const parts: Uint8Array[] = [];
  for (let from = start; from <= end; from += MAX_BLOCK_RANGE) {
    parts.push(await client.blocks(from, Math.min(end, from + MAX_BLOCK_RANGE - 1)));
  }
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

async function crossCheck(
  blocks: Uint8Array,
  options: {
    secondSource?: { kind: "relay"; url: string } | { kind: "blockchair"; network: "main" };
  },
): Promise<CrossCheck> {
  const ours = blockHashes(blocks) as BlockHash[];
  const source = options.secondSource;
  if (source === undefined) {
    return {
      status: "unavailable",
      source: "none",
      detail:
        "no second source configured: block hashes were not cross-checked (no browser-reachable public source exists for testnet; run `zeek-verify relay` against any lightwalletd and configure it)",
    };
  }
  try {
    let theirs: BlockHash[] = [];
    if (source.kind === "relay") {
      const second = new RelayClient(source.url);
      const heights = ours.map((b) => b.height);
      const lo = Math.min(...heights);
      const hi = Math.max(...heights);
      // The payout block may be far from the window: fetch both ranges.
      const windowEnd = heights
        .filter((h) => h - lo < MAX_BLOCK_RANGE)
        .reduce((m, h) => Math.max(m, h), lo);
      let bytes = await fetchBlocks(second, lo, windowEnd);
      if (hi > windowEnd) {
        bytes = concat(bytes, await fetchBlocks(second, hi, hi));
      }
      theirs = blockHashes(bytes) as BlockHash[];
    } else {
      for (const block of ours) {
        const hash = await blockchairHash(block.height);
        if (hash !== undefined) theirs.push(hash);
      }
    }
    return compareHashes(ours, theirs, sourceName(source));
  } catch (error) {
    return {
      status: "unavailable",
      source: sourceName(source),
      detail: `the second source could not be reached: ${toPocketError(error).message}`,
    };
  }
}

type Handler<M extends MethodName> = (arg: Methods[M][0], id: number) => Promise<Methods[M][1]>;

const handlers: { [M in MethodName]: Handler<M> } = {
  async init({ engineUrl }) {
    relay = new RelayClient(engineUrl);
    const threads = await wasm();
    return { threads, crossOriginIsolated: globalThis.crossOriginIsolated };
  },
  async status() {
    const exists = await storage.exists();
    return {
      exists,
      unlocked: core !== undefined,
      address: core?.receiveAddress() ?? null,
      scannedHeight: core?.scannedHeight() ?? null,
      birthdayHeight: core?.birthdayHeight() ?? null,
    };
  },
  async create({ passphrase }) {
    await wasm();
    if (await storage.exists()) {
      throw new PocketError("wallet_exists", "a wallet already exists in this browser");
    }
    const client = needRelay();
    const tip = (await client.tip()).height;
    const treeState = await client.treeState(tip);
    return openFromTreeState(PocketCore.create(passphrase, treeState));
  },
  async restore({ phrase, birthdayHeight, passphrase }) {
    await wasm();
    if (await storage.exists()) {
      throw new PocketError("wallet_exists", "a wallet already exists in this browser");
    }
    const treeState = await needRelay().treeState(Math.max(1, birthdayHeight - 1));
    return openFromTreeState(PocketCore.restore(phrase, passphrase, treeState));
  },
  async unlock({ passphrase }) {
    await wasm();
    const stored = await storage.load();
    if (stored === undefined) {
      throw new PocketError("no_wallet", "no wallet in this browser; create or restore one");
    }
    core?.free();
    core = PocketCore.unlock(stored.seedEnvelope, passphrase, stored.state);
    return { address: core.receiveAddress() };
  },
  async lock() {
    if (core !== undefined) {
      await persist();
      core.free();
      core = undefined;
    }
    return null;
  },
  async forget() {
    core?.free();
    core = undefined;
    await storage.clear();
    return null;
  },
  getAddress() {
    return Promise.resolve(needCore().receiveAddress());
  },
  getBalance() {
    return Promise.resolve(needCore().balance() as Balance);
  },
  async sync(_arg, id) {
    return sync(id);
  },
  async dig({ tiles, amountEachZat, message, maxTilesPerTx, separate }, id) {
    const c = needCore();
    const client = needRelay();
    const batches = planBatches(tiles, maxTilesPerTx, separate === true);
    const sent: DigBatch[] = [];
    let round = -1;
    for (const [index, batchTiles] of batches.entries()) {
      const progress = (phase: DigProgress["phase"], txid?: string): void => {
        digProgress(id, {
          phase,
          batch: index + 1,
          batches: batches.length,
          tiles: batchTiles,
          ...(txid === undefined ? {} : { txid }),
        });
      };
      try {
        if (index > 0 && separate === true) {
          // Separate payments go out at random intervals so their timing
          // does not tie them together.
          const gap = privacyGapMs(cryptoRandom);
          digProgress(id, {
            phase: "waiting",
            batch: index + 1,
            batches: batches.length,
            tiles: batchTiles,
            waitingFor: "privacy-gap",
            seconds: Math.round(gap / 1000),
          });
          await new Promise((resolve) => setTimeout(resolve, gap));
        }
        const started = Date.now();
        let built: BuiltTransaction | undefined;
        let board: { round: number; blocks_remaining: number } = { round: -1, blocks_remaining: 0 };
        let readFailures = 0;
        while (built === undefined) {
          progress("sync");
          let chainTip: number;
          let json: string;
          try {
            chainTip = await syncToTip(id);
            const [config, current] = await Promise.all([client.config(), client.board()]);
            json = config.json;
            board = current;
          } catch (error) {
            // These steps only read, so a dropped upstream connection (the
            // relay's public lightwalletd does that) is retried rather than
            // stopping a dig part-way. Broadcasts are never retried.
            const e = toPocketError(error);
            readFailures += 1;
            if ((e.code !== "relay" && e.code !== "network") || readFailures > READ_RETRIES)
              throw e;
            await new Promise((resolve) => setTimeout(resolve, READ_RETRY_MS));
            continue;
          }
          if (round === -1) {
            round = board.round;
          } else if (board.round !== round) {
            throw new PocketError(
              "closing_too_soon",
              `round ${round} closed before batch ${index + 1}; the rest would land in round ${board.round}`,
            );
          }
          try {
            progress("proving");
            built = c.buildDig(
              json,
              JSON.stringify({
                tiles: batchTiles,
                amountEachZat,
                round: board.round,
                blocksRemaining: board.blocks_remaining,
                chainTip,
                message: message ?? null,
              }),
            ) as BuiltTransaction;
          } catch (error) {
            // A later batch may need the change of an earlier one: wait for
            // it to confirm (3 blocks) rather than give up.
            const e = toPocketError(error);
            const balance = c.balance() as Balance;
            const waited = Date.now() - started;
            if (
              e.code !== "insufficient_funds" ||
              index === 0 ||
              balance.pending_zat === 0 ||
              waited > CHANGE_WAIT_MS
            ) {
              throw e;
            }
            digProgress(id, {
              phase: "waiting",
              batch: index + 1,
              batches: batches.length,
              tiles: batchTiles,
              waitingFor: "change",
              seconds: Math.round(waited / 1000),
            });
            await new Promise((resolve) => setTimeout(resolve, CHANGE_POLL_MS));
          }
        }
        progress("broadcast", built.txid);
        await broadcastBuilt(c, built);
        sent.push({
          txid: built.txid,
          tiles: batchTiles,
          feeZat: built.feeZat,
          provingMs: built.provingMs,
          wasmMemoryBytes: wasmMemory?.buffer.byteLength ?? 0,
        });
        progress("sent", built.txid);
      } catch (error) {
        if (sent.length === 0) {
          throw error;
        }
        const unsent = batches.slice(index).flat();
        throw new PocketError(
          "partial_dig",
          describePartial(sent, unsent, toPocketError(error).message),
          undefined,
          { sent: sent.map((b) => ({ txid: b.txid, tiles: b.tiles })), unsent },
        );
      }
    }
    return { round, tiles, amountEachZat, batches: sent };
  },
  async shout({ message, amountZat }, id) {
    const c = needCore();
    const chainTip = await syncToTip(id);
    const { json } = await needRelay().config();
    const built = c.buildShout(
      json,
      JSON.stringify({ message, amountZat, chainTip }),
    ) as BuiltTransaction;
    await broadcastBuilt(c, built);
    return { txid: built.txid, amountZat, feeZat: built.feeZat, provingMs: built.provingMs };
  },
  async buildShoutRequest({ message, amountZat }) {
    await wasm();
    const client = needRelay();
    const [{ config }, tip] = await Promise.all([client.config(), client.tip()]);
    return coreBuildShoutRequest(
      JSON.stringify({ config, message, amountZat, chainTip: tip.height }),
    ) as Omit<AnyWalletShout, "qrSvg">;
  },
  async createWinProof({ round }) {
    const c = needCore();
    const client = needRelay();
    const bundle = await client.roundBundle(round);
    const parsed = JSON.parse(bundle) as {
      payout_openings?: { outputs: { payout_index: number }[] };
    };
    const outputs = parsed.payout_openings?.outputs ?? [];
    if (outputs.length === 0) {
      throw new PocketError(
        "invalid_request",
        `round ${round} has no payout openings yet (no payout, or not mined yet)`,
      );
    }
    let lastError: PocketError | undefined;
    for (const output of outputs) {
      try {
        const made = c.createWinProof(bundle, output.payout_index) as {
          id: string;
          proof: string;
          round: number;
          payoutIndex: number;
        };
        const submitted = await client.submitProof(made.proof);
        return {
          id: made.id,
          round: made.round,
          payoutIndex: made.payoutIndex,
          proof: made.proof,
          verdict: submitted.body.verdict,
          stored: submitted.stored,
        };
      } catch (error) {
        lastError = toPocketError(error);
        if (lastError.code !== "invalid_request") {
          throw lastError;
        }
      }
    }
    throw (
      lastError ??
      new PocketError("invalid_request", `no payout of round ${round} went to this wallet`)
    );
  },
  async verifyRound({ round, secondSource }, id) {
    await wasm();
    const client = needRelay();
    verifyProgress(id, "bundle", `fetching round ${round}'s proof bundle and the public config`);
    const [bundle, { json: config }] = await Promise.all([
      client.roundBundle(round),
      client.config(),
    ]);
    const plan = verifyPlan(bundle) as { start: number; end: number; payoutHeight: number | null };
    verifyProgress(id, "blocks", `fetching compact blocks ${plan.start}..${plan.end}`);
    let blocks = await fetchBlocks(client, plan.start, plan.end);
    if (
      plan.payoutHeight !== null &&
      (plan.payoutHeight < plan.start || plan.payoutHeight > plan.end)
    ) {
      blocks = concat(blocks, await fetchBlocks(client, plan.payoutHeight, plan.payoutHeight));
    }
    verifyProgress(id, "scan", "trial-decrypting the window with the public viewing key");
    const txids = verifyScan(bundle, config, blocks) as string[];
    verifyProgress(id, "transactions", `fetching ${txids.length} transactions`);
    const txs: Uint8Array[] = [];
    for (const txid of txids) {
      txs.push(await client.transaction(txid));
    }
    verifyProgress(
      id,
      "checks",
      "running the five checks (proof verification takes a few seconds)",
    );
    const report = coreVerifyRound(bundle, config, blocks, txs) as RoundReport;
    verifyProgress(id, "cross-check", "comparing block hashes with the second source");
    const check = await crossCheck(blocks, secondSource === undefined ? {} : { secondSource });
    verifyProgress(id, "done", "done");
    return { report, crossCheck: check, source: `Zeek relay ${client.baseUrl}` };
  },
  async verifyWinProof({ idOrProof }) {
    await wasm();
    const client = needRelay();
    const trimmed = idOrProof.trim();
    const encoded = /^[0-9a-f]{32}$/.test(trimmed) ? (await client.proof(trimmed)).proof : trimmed;
    const summary = decodeWinProof(encoded) as {
      id: string;
      round: number;
      payoutIndex: number;
      payoutTxid: string;
    };
    const bundle = await client.roundBundle(summary.round);
    const parsed = JSON.parse(bundle) as { payout_openings?: { mined_height: number } };
    const height = parsed.payout_openings?.mined_height;
    if (height === undefined) {
      throw new PocketError("invalid_request", `round ${summary.round} has no payout openings`);
    }
    const [tx, block] = await Promise.all([
      client.transaction(summary.payoutTxid),
      client.blocks(height, height),
    ]);
    const verdict = coreVerifyWinProof(encoded, bundle, tx, block) as WinVerdict;
    return { ...verdict, id: summary.id, round: summary.round, payoutIndex: summary.payoutIndex };
  },
  async send({ to, amountZat, memo }, id) {
    const c = needCore();
    const chainTip = await syncToTip(id);
    const built = c.buildSend(
      JSON.stringify({ to, amountZat, memo: memo ?? null, chainTip }),
    ) as BuiltTransaction;
    await broadcastBuilt(c, built);
    return { txid: built.txid, amountZat, feeZat: built.feeZat, provingMs: built.provingMs };
  },
  history() {
    return Promise.resolve(needCore().history() as HistoryEntry[]);
  },
  async exportSeed({ passphrase }) {
    const stored = await storage.load();
    if (stored === undefined) {
      throw new PocketError("no_wallet", "no wallet in this browser");
    }
    return needCore().exportPhrase(stored.seedEnvelope, passphrase);
  },
  async buildDigRequest({ payoutAddress, tiles, amountEachZat, round, message }) {
    await wasm();
    const client = needRelay();
    const [{ config }, board, tip] = await Promise.all([
      client.config(),
      client.board(),
      client.tip(),
    ]);
    const built = coreBuildDigRequest(
      JSON.stringify({
        config,
        payoutAddress,
        tiles,
        amountEachZat,
        round: round ?? board.round,
        chainTip: tip.height,
        message: message ?? null,
      }),
    ) as Omit<AnyWalletDig, "qrSvg">;
    return built;
  },
};

async function handle(message: RequestMessage): Promise<void> {
  const handler = handlers[message.method] as Handler<MethodName>;
  try {
    const value = await handler(message.arg, message.id);
    post({ id: message.id, type: "result", value });
  } catch (error) {
    post({ id: message.id, type: "error", error: toPocketError(error).toData() });
  }
}

scope.onmessage = (event: MessageEvent<RequestMessage>) => {
  // One operation at a time: a dig never interleaves with a sync.
  queue = queue.then(() => handle(event.data));
};
