import { describe, expect, it } from "vitest";
import { blockchairHash, compareHashes } from "./crosscheck.js";

describe("compareHashes", () => {
  const ours = [
    { height: 10, hash: "AA" },
    { height: 11, hash: "bb" },
  ];

  it("passes when every hash matches (case-insensitively)", () => {
    expect(compareHashes(ours, [...ours].reverse(), "s").status).toBe("pass");
    expect(
      compareHashes(
        ours,
        [
          { height: 10, hash: "aa" },
          { height: 11, hash: "BB" },
        ],
        "s",
      ).status,
    ).toBe("pass");
  });

  it("fails on any difference and is unavailable when heights are missing", () => {
    const fail = compareHashes(
      ours,
      [
        { height: 10, hash: "aa" },
        { height: 11, hash: "cc" },
      ],
      "s",
    );
    expect(fail.status).toBe("fail");
    expect(fail.detail).toContain("11");
    expect(compareHashes(ours, [{ height: 10, hash: "aa" }], "s").status).toBe("unavailable");
    expect(compareHashes([], [], "s").status).toBe("unavailable");
  });
});

describe("blockchairHash", () => {
  it("reads the documented dashboard shape", async () => {
    const fetchImpl = (url: string) => {
      expect(url).toBe("https://api.blockchair.com/zcash/dashboards/block/3000000");
      return Promise.resolve(
        Response.json({ data: { "3000000": { block: { id: 3000000, hash: "00ab" } } } }),
      );
    };
    expect(await blockchairHash(3_000_000, fetchImpl)).toEqual({ height: 3_000_000, hash: "00ab" });
    const missing = () => Promise.resolve(new Response("{}", { status: 404 }));
    expect(await blockchairHash(1, missing)).toBeUndefined();
  });
});
