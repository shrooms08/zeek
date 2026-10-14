/** Messages between the `Pocket` facade and its Web Worker. */

import type { PocketErrorData } from "./errors.js";
import type {
  AnyWalletDig,
  Balance,
  DigResult,
  HistoryEntry,
  SendResult,
  SyncProgress,
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
}

export interface SendOptions {
  to: string;
  amountZat: number;
  memo?: string;
}

export interface DigRequestOptions {
  payoutAddress: string;
  tiles: number[];
  amountEachZat: number;
  /** The round to dig; defaults to the board's current round. */
  round?: number;
}

/** Method name to [argument, result]. */
export interface Methods {
  init: [{ engineUrl: string }, { threads: number; crossOriginIsolated: boolean }];
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
  buildDigRequest: [DigRequestOptions, Omit<AnyWalletDig, "qrSvg">];
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
  | { id: number; type: "progress"; progress: SyncProgress };
