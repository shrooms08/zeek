/**
 * The wallet worker: owns the WebAssembly wallet core, the relay client and
 * IndexedDB. The seed and keys exist only inside this worker's wasm memory
 * while unlocked; nothing secret is posted back except the recovery phrase on
 * an explicit, passphrase-checked export.
 */

import init, {
  buildDigRequest as coreBuildDigRequest,
  initThreadPool,
  PocketCore,
} from "../wasm/zeek_pocket.js";
import { cryptoRandom, planTransactionFetches, type TxRef } from "./decoys.js";
import { PocketError, toPocketError } from "./errors.js";
import type { Methods, MethodName, RequestMessage, ResponseMessage } from "./protocol.js";
import { hexToBytes, MAX_BLOCK_RANGE, RelayClient } from "./relay.js";
import { WalletStorage } from "./storage.js";
import type {
  AnyWalletDig,
  Balance,
  BuiltTransaction,
  HistoryEntry,
  ScanReport,
  SyncProgress,
} from "./types.js";

/** Blocks fetched per relay request while syncing. */
const SYNC_BATCH = 200;
/** Decoy transactions fetched per wanted one. */
const DECOYS_PER_WANTED = 3;
/** Rewinds attempted on one sync before giving up. */
const MAX_REWINDS = 20;
/** Syncs attempted before building, when blocks keep arriving meanwhile. */
const MAX_CATCH_UP = 5;

const scope = self as unknown as DedicatedWorkerGlobalScope;
const storage = new WalletStorage();
let relay: RelayClient | undefined;
let core: PocketCore | undefined;
let ready: Promise<number> | undefined;
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
  await init();
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
  async dig({ tiles, amountEachZat }, id) {
    const c = needCore();
    const client = needRelay();
    const chainTip = await syncToTip(id);
    const [{ json }, board] = await Promise.all([client.config(), client.board()]);
    const built = c.buildDig(
      json,
      JSON.stringify({
        tiles,
        amountEachZat,
        round: board.round,
        blocksRemaining: board.blocks_remaining,
        chainTip,
      }),
    ) as BuiltTransaction;
    await broadcastBuilt(c, built);
    return {
      txid: built.txid,
      round: board.round,
      tiles,
      amountEachZat,
      feeZat: built.feeZat,
      provingMs: built.provingMs,
    };
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
  async buildDigRequest({ payoutAddress, tiles, amountEachZat, round }) {
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
