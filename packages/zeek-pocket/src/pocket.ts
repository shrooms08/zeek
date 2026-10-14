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
  WalletStatus,
} from "./protocol.js";
import { qrSvg } from "./qr.js";
import type {
  AnyWalletDig,
  Balance,
  DigResult,
  HistoryEntry,
  SendResult,
  SyncProgress,
} from "./types.js";

interface Pending {
  resolve: (value: unknown) => void;
  reject: (error: PocketError) => void;
  onProgress: ((progress: SyncProgress) => void) | undefined;
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
   * relay and game API). The page must be cross-origin isolated.
   */
  static async start(engineUrl: string): Promise<Pocket> {
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    const pocket = new Pocket(worker);
    await pocket.call("init", { engineUrl });
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
    onProgress?: (progress: SyncProgress) => void,
  ): Promise<Methods[M][1]> {
    const id = this.nextId;
    this.nextId += 1;
    return new Promise<Methods[M][1]>((resolve, reject) => {
      this.pending.set(id, {
        resolve: (value) => {
          resolve(value as Methods[M][1]);
        },
        reject,
        onProgress,
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

  /** Digs `tiles` in the current round, one sealed fresh payout address per tile. */
  dig(options: DigOptions, onProgress?: (progress: SyncProgress) => void): Promise<DigResult> {
    return this.call("dig", options, onProgress);
  }

  /** Sends to any address with an Orchard-protocol receiver. */
  send(options: SendOptions, onProgress?: (progress: SyncProgress) => void): Promise<SendResult> {
    return this.call("send", options, onProgress);
  }

  history(): Promise<HistoryEntry[]> {
    return this.call("history", undefined);
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
  options: DigRequestOptions,
): Promise<AnyWalletDig> {
  const pocket = await Pocket.start(engineUrl);
  try {
    return await pocket.buildDigRequest(options);
  } finally {
    pocket.terminate();
  }
}
