import { describe, expect, it } from "vitest";
import { PocketError } from "./errors.js";
import { hexToBytes, RelayClient } from "./relay.js";

function fakeFetch(
  routes: Record<string, () => Response>,
  seen: { url: string; init: RequestInit | undefined }[],
) {
  return (url: string, init?: RequestInit): Promise<Response> => {
    seen.push({ url, init });
    const route = routes[url.replace("http://engine", "")];
    return Promise.resolve(route ? route() : new Response("{}", { status: 404 }));
  };
}

describe("RelayClient", () => {
  it("reads tip, blocks and transactions, and posts broadcasts", async () => {
    const seen: { url: string; init: RequestInit | undefined }[] = [];
    const client = new RelayClient(
      "http://engine/",
      fakeFetch(
        {
          "/v1/chain/tip": () => Response.json({ height: 7, hash: "ab" }),
          "/v1/chain/blocks?start=5&end=6": () => new Response(new Uint8Array([1, 2, 3])),
          [`/v1/tx/${"0".repeat(64)}`]: () => new Response(new Uint8Array([9])),
          "/v1/tx/broadcast": () => Response.json({ ok: true }),
        },
        seen,
      ),
    );
    expect(await client.tip()).toEqual({ height: 7, hash: "ab" });
    expect(await client.blocks(5, 6)).toEqual(new Uint8Array([1, 2, 3]));
    expect(await client.transaction("0".repeat(64))).toEqual(new Uint8Array([9]));
    await client.broadcast(new Uint8Array([4]));
    const post = seen.at(-1);
    expect(post?.url).toBe("http://engine/v1/tx/broadcast");
    expect(post?.init?.method).toBe("POST");
  });

  it("refuses oversized ranges and bad txids before any request", async () => {
    const seen: { url: string; init: RequestInit | undefined }[] = [];
    const client = new RelayClient("http://engine", fakeFetch({}, seen));
    await expect(client.blocks(0, 500)).rejects.toMatchObject({ code: "invalid_request" });
    await expect(client.transaction("xyz")).rejects.toMatchObject({ code: "invalid_request" });
    expect(seen).toHaveLength(0);
  });

  it("maps HTTP and network failures to typed errors", async () => {
    const client = new RelayClient(
      "http://engine",
      fakeFetch(
        {
          "/v1/tx/broadcast": () =>
            Response.json({ error: "the node rejected the transaction: dup" }, { status: 422 }),
          "/v1/chain/tip": () => Response.json({ error: "rate limit exceeded" }, { status: 429 }),
        },
        [],
      ),
    );
    await expect(client.broadcast(new Uint8Array([1]))).rejects.toMatchObject({
      code: "broadcast_rejected",
      message: "the node rejected the transaction: dup",
    });
    await expect(client.tip()).rejects.toMatchObject({ code: "relay" });
    const offline = new RelayClient("http://engine", () =>
      Promise.reject(new TypeError("Failed to fetch")),
    );
    const error: unknown = await offline.tip().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(PocketError);
    expect((error as PocketError).code).toBe("network");
  });

  it("decodes hex", () => {
    expect(hexToBytes("00ff10")).toEqual(new Uint8Array([0, 255, 16]));
    expect(() => hexToBytes("abc")).toThrow(PocketError);
  });

  it("refuses an engine config that names another vault (security review H3)", async () => {
    const pinned = {
      network: "test",
      vault_address: "utest1vault",
      sealing_public_key: "aa".repeat(32),
      uivk: "uivktest1key",
      params: { network: "test", min_dig_zat: 1, blocks_per_round: 24, genesis_height: 1 },
    };
    const config = (overrides: Record<string, unknown>) => () =>
      Response.json({
        params: { network: "test", min_dig_zat: 1, blocks_per_round: 24, genesis_height: 1 },
        vault_address: pinned.vault_address,
        sealing_public_key: pinned.sealing_public_key,
        uivk: pinned.uivk,
        genesis_height: 1,
        ...overrides,
      });
    const honest = new RelayClient(
      "http://engine",
      fakeFetch({ "/v1/config": config({}) }, []),
      pinned,
    );
    expect((await honest.config()).config.vault_address).toBe("utest1vault");
    for (const [field, value] of [
      ["vault_address", "utest1attacker"],
      ["sealing_public_key", "bb".repeat(32)],
      ["uivk", "uivktest1other"],
    ]) {
      const swapped = new RelayClient(
        "http://engine",
        fakeFetch({ "/v1/config": config({ [field as string]: value }) }, []),
        pinned,
      );
      await expect(swapped.config()).rejects.toMatchObject({ code: "config_mismatch" });
    }
    const mainnet = new RelayClient(
      "http://engine",
      fakeFetch(
        {
          "/v1/config": config({
            params: { network: "main", min_dig_zat: 1, blocks_per_round: 24, genesis_height: 1 },
          }),
        },
        [],
      ),
      pinned,
    );
    await expect(mainnet.config()).rejects.toMatchObject({ code: "config_mismatch" });
    const cheaper = new RelayClient(
      "http://engine",
      fakeFetch(
        {
          "/v1/config": config({
            params: { network: "test", min_dig_zat: 2, blocks_per_round: 24, genesis_height: 1 },
          }),
        },
        [],
      ),
      pinned,
    );
    await expect(cheaper.config()).rejects.toMatchObject({ code: "config_mismatch" });
  });
});
