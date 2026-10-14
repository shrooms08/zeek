import "fake-indexeddb/auto";
import { describe, expect, it } from "vitest";
import { WalletStorage } from "./storage.js";

describe("WalletStorage", () => {
  it("stores the seed envelope and encrypted state, and clears them", async () => {
    const storage = new WalletStorage();
    expect(await storage.exists()).toBe(false);
    await storage.saveAll({ seedEnvelope: '{"version":1}', state: new Uint8Array([1, 2]) });
    expect(await storage.load()).toEqual({
      seedEnvelope: '{"version":1}',
      state: new Uint8Array([1, 2]),
    });
    await storage.saveState(new Uint8Array([3]));
    expect((await storage.load())?.state).toEqual(new Uint8Array([3]));
    await storage.clear();
    expect(await storage.exists()).toBe(false);
  });
});
