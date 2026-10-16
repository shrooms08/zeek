/**
 * Zeek Pocket: the browser wallet's typed API. Every call runs in a Web
 * Worker that owns the wallet (see `worker.ts`); this class only forwards
 * calls and progress events.
 */

import { PocketError } from "./errors.js";
import type {
  DigOptions,
  DigRequestOptions,
  Methods,
  MethodName,
  RequestMessage,
  ResponseMessage,
  SendOptions,
  ShoutOptions,
  ShoutRequestOptions,
  VerifyOptions,
  WalletStatus,
} from "./protocol.js";
import { qrSvg } from "./qr.js";
import type {
  AnyWalletDig,
  AnyWalletShout,
  Balance,
  DigProgress,
  DigResult,
  HistoryEntry,
  PinnedVault,
  RoundVerification,
  SendResult,
  SyncProgress,
  VerifyProgress,
  WinProofResult,
  WinVerdict,
  OfflineReport,
  SplitResult,
} from "./types.js";

/** Progress callbacks of one call. */
interface Listeners {
  onProgress?: ((progress: SyncProgress) => void) | undefined;
  onDig?: ((progress: DigProgress) => void) | undefined;
  onVerify?: ((progress: VerifyProgress) => void) | undefined;
}

interface Pending extends Listeners {
  resolve: (value: unknown) => void;
  reject: (error: PocketError) => void;
}

export class Pocket {
  private readonly worker: Worker;
  private readonly pending = new Map<number, Pending>();
  private nextId = 1;

  private constructor(worker: Worker) {
    this.worker = worker;
    this.worker.onmessage = (event: MessageEvent<ResponseMessage>) => {
      this.receive(event.data);
    };
  }

  /**
   * Starts the wallet worker against the engine at `engineUrl` (its chain
   * relay and game API) for the vault `pinned`. The page must be
   * cross-origin isolated.
   */
  static async start(engineUrl: string, pinned: PinnedVault): Promise<Pocket> {
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    const pocket = new Pocket(worker);
    await pocket.call("init", { engineUrl, pinned });
    return pocket;
  }

  private receive(message: ResponseMessage): void {
    const pending = this.pending.get(message.id);
    if (pending === undefined) {
      return;
    }
    switch (message.type) {
      case "progress":
        pending.onProgress?.(message.progress);
        return;
      case "dig-progress":
        pending.onDig?.(message.progress);
        return;
      case "verify-progress":
        pending.onVerify?.(message.progress);
        return;
      case "result":
        this.pending.delete(message.id);
        pending.resolve(message.value);
        return;
      case "error":
        this.pending.delete(message.id);
        pending.reject(PocketError.fromData(message.error));
        return;
    }
  }

  private call<M extends MethodName>(
    method: M,
    arg: Methods[M][0],
    listeners: Listeners | ((progress: SyncProgress) => void) = {},
  ): Promise<Methods[M][1]> {
    const id = this.nextId;
    this.nextId += 1;
    const l: Listeners = typeof listeners === "function" ? { onProgress: listeners } : listeners;
    return new Promise<Methods[M][1]>((resolve, reject) => {
      this.pending.set(id, {
        resolve: (value) => {
          resolve(value as Methods[M][1]);
        },
        reject,
        ...l,
      });
      const message: RequestMessage<M> = { id, method, arg };
      this.worker.postMessage(message);
    });
  }

  status(): Promise<WalletStatus> {
    return this.call("status", undefined);
  }

  /** Creates a wallet with a fresh 24-word phrase; its birthday is the current tip. */
  create(passphrase: string): Promise<{ address: string; birthdayHeight: number }> {
    return this.call("create", { passphrase });
  }

  /** Restores from a 24-word phrase, scanning from `birthdayHeight`. */
  restore(
    phrase: string,
    birthdayHeight: number,
    passphrase: string,
  ): Promise<{ address: string; birthdayHeight: number }> {
    return this.call("restore", { phrase, birthdayHeight, passphrase });
  }

  unlock(passphrase: string): Promise<{ address: string }> {
    return this.call("unlock", { passphrase });
  }

  /** Saves the state and drops the keys from memory. */
  async lock(): Promise<void> {
    await this.call("lock", undefined);
  }

  /** Deletes the wallet from this browser. */
  async forget(): Promise<void> {
    await this.call("forget", undefined);
  }

  /** The receive address (diversifier index 0). */
  getAddress(): Promise<string> {
    return this.call("getAddress", undefined);
  }

  getBalance(): Promise<Balance> {
    return this.call("getBalance", undefined);
  }

  /** Scans to the chain tip and fetches memos; reports progress. */
  sync(onProgress?: (progress: SyncProgress) => void): Promise<Balance> {
    return this.call("sync", undefined, onProgress);
  }

  /**
   * Digs `tiles` in the current round, one sealed fresh payout address per
   * tile. Selections larger than `maxTilesPerTx` (default 6) are split into
   * sequential transactions, each proven and broadcast in turn; `onBatch`
   * reports each step. If a later batch fails the error has code
   * `partial_dig` and `partial` lists exactly what was sent (never resent)
   * and which tiles were not.
   */
  dig(
    options: DigOptions,
    onBatch?: (progress: DigProgress) => void,
    onProgress?: (progress: SyncProgress) => void,
  ): Promise<DigResult> {
    return this.call("dig", options, { onDig: onBatch, onProgress });
  }

  /** Shouts a Campfire message (rules v4); the amount goes to the Deep Cache. */
  shout(options: ShoutOptions, onProgress?: (progress: SyncProgress) => void): Promise<SendResult> {
    return this.call("shout", options, onProgress);
  }

  /**
   * Proves (in the worker) that a treasure payout of `round` reached this
   * wallet, submits the proof to the engine (which verifies it before
   * storing) and returns it with a share URL built on `shareBase`.
   */
  async createWinProof(
    round: number,
    shareBase: string,
  ): Promise<WinProofResult & { shareUrl: string }> {
    const result = await this.call("createWinProof", { round });
    const url = new URL(shareBase);
    url.searchParams.set("winproof", result.id);
    return { ...result, shareUrl: url.toString() };
  }

  /** Runs the five round checks in the browser. */
  verifyRound(
    options: VerifyOptions,
    onVerify?: (progress: VerifyProgress) => void,
  ): Promise<RoundVerification> {
    return this.call("verifyRound", options, { onVerify });
  }

  /**
   * The parts of the round checks that need no chain data, for when the
   * engine is off (Replay mode): `bundle` and `config` come from the site's
   * saved snapshot; the beacon is compared with drand's public relays.
   * Needs no engine; chain-dependent parts come back `needs_chain`.
   */
  verifyOffline(bundle: string, config: string): Promise<OfflineReport> {
    return this.call("verifyOffline", { bundle, config });
  }

  /** Verifies a win proof given its engine id or the base64url proof itself. */
  verifyWinProof(
    idOrProof: string,
  ): Promise<WinVerdict & { id: string; round: number; payoutIndex: number }> {
    return this.call("verifyWinProof", { idOrProof });
  }

  /** The any-wallet shout request (ZIP 321 with a text memo) and its QR code. */
  async buildShoutRequest(options: ShoutRequestOptions): Promise<AnyWalletShout> {
    const built = await this.call("buildShoutRequest", options);
    return { ...built, qrSvg: await qrSvg(built.uri) };
  }

  /** Sends to any address with an Orchard-protocol receiver. */
  send(options: SendOptions, onProgress?: (progress: SyncProgress) => void): Promise<SendResult> {
    return this.call("send", options, onProgress);
  }

  history(): Promise<HistoryEntry[]> {
    return this.call("history", undefined);
  }

  /**
   * Checks the passphrase against the stored wallet (the wallet must be
   * unlocked); resolves `true` or rejects with `bad_passphrase`. Nothing
   * secret comes back.
   */
  confirmPassphrase(passphrase: string): Promise<true> {
    return this.call("confirmPassphrase", { passphrase });
  }

  /**
   * Gets the coins ready for several digs in a round: when the balance sits
   * in fewer than two notes of at least 0.02 and splitting costs under 1 %,
   * splits it with a shielded self-send into 2 to 4 notes (see
   * `crates/zeek-pocket/src/tx.rs`, `plan_split`). Resolves null when
   * nothing is worth splitting. Never called inside a dig, send or auto-dig.
   */
  prepareCoins(onProgress?: (progress: SyncProgress) => void): Promise<SplitResult | null> {
    return this.call("prepareCoins", undefined, onProgress);
  }

  /** Returns the 24-word phrase after checking the passphrase. */
  exportSeed(passphrase: string): Promise<string> {
    return this.call("exportSeed", { passphrase });
  }

  /** The any-wallet ZIP 321 request (see {@link buildDigRequest}). */
  async buildDigRequest(options: DigRequestOptions): Promise<AnyWalletDig> {
    const built = await this.call("buildDigRequest", options);
    return { ...built, qrSvg: await qrSvg(built.uri) };
  }

  terminate(): void {
    this.worker.terminate();
  }
}

/**
 * The any-wallet path without a Pocket wallet: validates the payout address
 * (shielded Orchard-protocol receiver only), seals it into one `zeek1:` text
 * memo per tile and returns the ZIP 321 URI and a QR code. Any Zcash wallet
 * can pay it.
 */
export async function buildDigRequest(
  engineUrl: string,
  pinned: PinnedVault,
  options: DigRequestOptions,
): Promise<AnyWalletDig> {
  const pocket = await Pocket.start(engineUrl, pinned);
  try {
    return await pocket.buildDigRequest(options);
  } finally {
    pocket.terminate();
  }
}
