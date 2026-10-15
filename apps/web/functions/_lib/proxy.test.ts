import { describe, expect, it } from "vitest";
import { allowed, subpath, validUpstream } from "./proxy";

describe("proxy", () => {
  it("forwards only the listed routes and methods", () => {
    expect(allowed("engine", "GET", "/v1/board")).toBeDefined();
    expect(allowed("engine", "GET", "/v1/rounds/205/proof")).toBeDefined();
    expect(allowed("engine", "GET", "/v1/stream")?.websocket).toBe(true);
    expect(allowed("engine", "POST", "/v1/proofs")?.maxBody).toBe(32 * 1024);
    expect(
      allowed("engine", "GET", "/share/win/dbd7adfbf2bdf06a966bac96953caac2.png"),
    ).toBeDefined();
    expect(allowed("engine", "POST", "/v1/board")).toBeUndefined();
    expect(allowed("engine", "GET", "/v1/rounds/abc")).toBeUndefined();
    expect(allowed("engine", "GET", "/v1/../../etc/passwd")).toBeUndefined();
    expect(allowed("engine", "DELETE", "/v1/proofs")).toBeUndefined();
    expect(allowed("relay", "POST", "/v1/tx/broadcast")).toBeUndefined();
    expect(allowed("relay", "GET", "/v1/chain/tip")).toBeDefined();
    // Each route may only answer with its own kind of content (review N3).
    expect(allowed("engine", "GET", "/v1/board")?.kind).toBe("json");
    expect(allowed("engine", "GET", "/v1/chain/blocks")?.kind).toBe("binary");
    // The engine's relay answers tree states as protobuf (the Pocket needs
    // them to create a wallet).
    expect(allowed("engine", "GET", "/v1/chain/tree-state")?.kind).toBe("binary");
    expect(allowed("engine", "GET", "/v1/chain/tip")?.kind).toBe("json");
    expect(allowed("engine", "GET", "/share/round/205")?.kind).toBe("share");
  });

  it("contacts only https quick-tunnel hosts", () => {
    const ok = { url: "https://word-word-word.trycloudflare.com", updated: 1 };
    expect(validUpstream(ok)).toEqual(ok);
    expect(validUpstream({ url: "http://word.trycloudflare.com", updated: 1 })).toBeNull();
    expect(validUpstream({ url: "https://evil.example", updated: 1 })).toBeNull();
    expect(
      validUpstream({ url: "https://a.trycloudflare.com.evil.example", updated: 1 }),
    ).toBeNull();
    expect(validUpstream({ url: "https://a.trycloudflare.com/path", updated: 1 })).toBeNull();
    expect(validUpstream(null)).toBeNull();
  });

  it("maps route parameters to upstream paths", () => {
    expect(subpath("", ["v1", "board"])).toBe("/v1/board");
    expect(subpath("/share", ["win", "x.png"])).toBe("/share/win/x.png");
    expect(subpath("", undefined)).toBe("");
  });
});
