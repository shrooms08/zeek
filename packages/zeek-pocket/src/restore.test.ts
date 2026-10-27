import "fake-indexeddb/auto";
import { beforeEach, describe, expect, it } from "vitest";
import { PocketError } from "./errors.js";
import { restoreOver } from "./restore.js";
import { WalletStorage } from "./storage.js";

const OLD = { seedEnvelope: '{"old":true}', state: new Uint8Array([1]) };
const NEW = { seedEnvelope: '{"new":true}', state: new Uint8Array([2]) };

describe("restoring over the Pocket in this browser", () => {
  let storage: WalletStorage;
  beforeEach(async () => {
    storage = new WalletStorage();
    await storage.saveAll(OLD);
  });

  const commit = (w: typeof NEW) => storage.saveAll(w);

  it("a wrong word leaves the old Pocket untouched", async () => {
    await expect(
      restoreOver({
        exists: () => storage.exists(),
        birthdayHeight: 4_500_000,
        replace: true,
        build: () =>
          Promise.reject(new PocketError("invalid_mnemonic", "word 7 is not a BIP 39 word")),
        commit,
      }),
    ).rejects.toMatchObject({ code: "invalid_mnemonic" });
    expect(await storage.load()).toEqual(OLD);
  });

  for (const birthdayHeight of [0, Number.NaN, 4_500_000.5, -3]) {
    it(`a birthday of ${String(birthdayHeight)} is refused before anything is fetched or saved`, async () => {
      let built = false;
      await expect(
        restoreOver({
          exists: () => storage.exists(),
          birthdayHeight,
          replace: true,
          build: () => {
            built = true;
            return Promise.resolve(NEW);
          },
          commit,
        }),
      ).rejects.toMatchObject({ code: "invalid_request" });
      expect(built).toBe(false);
      expect(await storage.load()).toEqual(OLD);
    });
  }

  it("replaces the old Pocket only once the restored one is built, scanning from the block before", async () => {
    let from = 0;
    await restoreOver({
      exists: () => storage.exists(),
      birthdayHeight: 4_500_000,
      replace: true,
      build: (fromHeight) => {
        from = fromHeight;
        return Promise.resolve(NEW);
      },
      commit,
    });
    expect(from).toBe(4_499_999);
    expect(await storage.load()).toEqual(NEW);
  });

  it("without replace, an existing Pocket is never overwritten", async () => {
    await expect(
      restoreOver({
        exists: () => storage.exists(),
        birthdayHeight: 4_500_000,
        replace: false,
        build: () => Promise.resolve(NEW),
        commit,
      }),
    ).rejects.toMatchObject({ code: "wallet_exists" });
    expect(await storage.load()).toEqual(OLD);
  });
});
