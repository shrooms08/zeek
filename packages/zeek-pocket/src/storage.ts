/**
 * IndexedDB persistence. One wallet per origin, in database `zeek-pocket`,
 * object store `wallet`:
 *
 * - `seedEnvelope`: JSON from the wallet core (Argon2id + XChaCha20-Poly1305);
 *   the seed is never stored in clear and the passphrase not at all.
 * - `state`: the encrypted wallet state (bytes), readable only with the seed.
 */

import { openDB, type IDBPDatabase } from "idb";

const DB_NAME = "zeek-pocket";
const STORE = "wallet";

export interface StoredWallet {
  seedEnvelope: string;
  state: Uint8Array;
}

export class WalletStorage {
  private db: Promise<IDBPDatabase> | undefined;

  private open(): Promise<IDBPDatabase> {
    this.db ??= openDB(DB_NAME, 1, {
      upgrade(db) {
        db.createObjectStore(STORE);
      },
    });
    return this.db;
  }

  async load(): Promise<StoredWallet | undefined> {
    const db = await this.open();
    const seedEnvelope = (await db.get(STORE, "seedEnvelope")) as string | undefined;
    const state = (await db.get(STORE, "state")) as Uint8Array | undefined;
    if (seedEnvelope === undefined || state === undefined) {
      return undefined;
    }
    return { seedEnvelope, state };
  }

  async exists(): Promise<boolean> {
    return (await this.load()) !== undefined;
  }

  async saveAll(wallet: StoredWallet): Promise<void> {
    const db = await this.open();
    const tx = db.transaction(STORE, "readwrite");
    await Promise.all([
      tx.store.put(wallet.seedEnvelope, "seedEnvelope"),
      tx.store.put(wallet.state, "state"),
      tx.done,
    ]);
  }

  async saveState(state: Uint8Array): Promise<void> {
    const db = await this.open();
    await db.put(STORE, state, "state");
  }

  async clear(): Promise<void> {
    const db = await this.open();
    await db.clear(STORE);
  }
}
