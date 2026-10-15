/** Messages between the `Pocket` facade and its Web Worker. */

import type { PocketErrorData } from "./errors.js";
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
} from "./types.js";

export interface WalletStatus {
  exists: boolean;
  unlocked: boolean;
  address: string | null;
  scannedHeight: number | null;
  birthdayHeight: number | null;
}

export interface DigOptions {
  tiles: number[];
  amountEachZat: number;
  /** Campfire message for every dig (rules v4), at most 275 bytes. */
  message?: string;
  /** Tiles per transaction (default 6): larger selections are split into
   * sequential transactions. */
  maxTilesPerTx?: number;
  /** One tile per transaction. */
  separate?: boolean;
}

export interface ShoutOptions {
  message: string;
  amountZat: number;
}

export interface ShoutRequestOptions {
  message: string;
  amountZat: number;
}

/** An independent source of block hashes to cross-check the relay with. */
export type SecondSource = { kind: "relay"; url: string } | { kind: "blockchair"; network: "main" };

export interface VerifyOptions {
  round: number;
  secondSource?: SecondSource;
}

export interface SendOptions {
  to: string;
  amountZat: number;
  memo?: string;
  /** The wallet's passphrase, asked again before funds leave the game. */
  passphrase: string;
}

export interface DigRequestOptions {
  payoutAddress: string;
  tiles: number[];
  amountEachZat: number;
  /** The round to dig; defaults to the board's current round. */
  round?: number;
  /** Campfire message for every dig (rules v4). */
  message?: string;
}

/** Method name to [argument, result]. */
export interface Methods {
  init: [
    { engineUrl: string; pinned: PinnedVault },
    { threads: number; crossOriginIsolated: boolean },
  ];
  status: [undefined, WalletStatus];
  create: [{ passphrase: string }, { address: string; birthdayHeight: number }];
  restore: [
    { phrase: string; birthdayHeight: number; passphrase: string },
    { address: string; birthdayHeight: number },
  ];
  unlock: [{ passphrase: string }, { address: string }];
  lock: [undefined, null];
  forget: [undefined, null];
  getAddress: [undefined, string];
  getBalance: [undefined, Balance];
  sync: [undefined, Balance];
  dig: [DigOptions, DigResult];
  send: [SendOptions, SendResult];
  history: [undefined, HistoryEntry[]];
  exportSeed: [{ passphrase: string }, string];
  confirmPassphrase: [{ passphrase: string }, true];
  buildDigRequest: [DigRequestOptions, Omit<AnyWalletDig, "qrSvg">];
  shout: [ShoutOptions, SendResult];
  buildShoutRequest: [ShoutRequestOptions, Omit<AnyWalletShout, "qrSvg">];
  createWinProof: [{ round: number }, WinProofResult];
  verifyRound: [VerifyOptions, RoundVerification];
  verifyOffline: [{ bundle: string; config: string }, OfflineReport];
  verifyWinProof: [
    { idOrProof: string },
    WinVerdict & { id: string; round: number; payoutIndex: number },
  ];
}

export type MethodName = keyof Methods;

export interface RequestMessage<M extends MethodName = MethodName> {
  id: number;
  method: M;
  arg: Methods[M][0];
}

export type ResponseMessage =
  | { id: number; type: "result"; value: unknown }
  | { id: number; type: "error"; error: PocketErrorData }
  | { id: number; type: "progress"; progress: SyncProgress }
  | { id: number; type: "dig-progress"; progress: DigProgress }
  | { id: number; type: "verify-progress"; progress: VerifyProgress };
