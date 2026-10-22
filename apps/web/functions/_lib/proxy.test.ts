import { afterEach, describe, expect, it, vi } from "vitest";
import {
  type EdgeCache,
  type Env,
  allowed,
  edgeCacheable,
  fixedUpstream,
  forward,
  subpath,
  validUpstream,
} from "./proxy";

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

  it("accepts a fixed hosted origin only on Railway's https service domains", () => {
    expect(fixedUpstream("https://engine-production.up.railway.app")).toEqual({
      url: "https://engine-production.up.railway.app",
      updated: 0,
    });
    expect(fixedUpstream("http://engine-production.up.railway.app")).toBeNull();
    expect(fixedUpstream("https://engine.up.railway.app.evil.example")).toBeNull();
    expect(fixedUpstream("https://engine.up.railway.app/path")).toBeNull();
    expect(fixedUpstream("https://evil.example")).toBeNull();
  });

  it("uses the fixed origin instead of KV when one is configured", async () => {
    const kv = vi.fn(() => Promise.resolve(null));
    const targets: string[] = [];
    const fetchMock = vi.fn((target: string) => {
      targets.push(target);
      return Promise.resolve(
        new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
      );
    });
    vi.stubGlobal("fetch", fetchMock);
    const env = {
      ZEEK: { get: kv },
      ZEEK_PROXY_SECRET: "s",
      ZEEK_ENGINE_ORIGIN: "https://engine-production.up.railway.app",
    } as unknown as Env;
    const request = new Request("https://playzeek.pages.dev/api/v1/board", {
      headers: { "CF-Connecting-IP": "203.0.113.7" },
    });
    const response = await forward(request, env, "engine", "/v1/board");
    expect(response.status).toBe(200);
    expect(kv).not.toHaveBeenCalled();
    expect(targets).toEqual(["https://engine-production.up.railway.app/v1/board"]);
    vi.unstubAllGlobals();
  });

  it("maps route parameters to upstream paths", () => {
    expect(subpath("", ["v1", "board"])).toBe("/v1/board");
    expect(subpath("/share", ["win", "x.png"])).toBe("/share/win/x.png");
    expect(subpath("", undefined)).toBe("");
  });

  describe("a tunnel that has stopped", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("reports the service offline instead of Cloudflare's own error page", async () => {
      const kv = vi.fn(() =>
        Promise.resolve({ url: "https://gone.trycloudflare.com", updated: 1 }),
      );
      const env = { ZEEK: { get: kv }, ZEEK_PROXY_SECRET: "s" } as unknown as Env;
      vi.stubGlobal(
        "fetch",
        vi.fn(() =>
          Promise.resolve(
            new Response("error code: 1033", {
              status: 530,
              headers: { "content-type": "text/plain; charset=UTF-8" },
            }),
          ),
        ),
      );
      const request = new Request("https://playzeek.pages.dev/share/round/1", {
        headers: { "CF-Connecting-IP": "203.0.113.7" },
      });

      const first = await forward(request, env, "engine", "/share/round/1");
      expect(first.status).toBe(503);
      expect(await first.json()).toMatchObject({ offline: true });
      // The stale address is forgotten: the next request reads KV again.
      await forward(request, env, "engine", "/share/round/1");
      expect(kv).toHaveBeenCalledTimes(2);
    });
  });

  describe("the edge cache", () => {
    afterEach(() => vi.unstubAllGlobals());

    function fakeEdge(): EdgeCache & { store: Map<string, Response>; pending: Promise<unknown>[] } {
      const store = new Map<string, Response>();
      const pending: Promise<unknown>[] = [];
      return {
        store,
        pending,
        cache: {
          match: (key: string) => Promise.resolve(store.get(key)?.clone()),
          put: (key: string, value: Response) => {
            store.set(key, value);
            return Promise.resolve();
          },
        } as unknown as Cache,
        waitUntil: (p) => {
          pending.push(p);
        },
      };
    }
    const env = (url: string | null) =>
      ({
        ZEEK: { get: vi.fn(() => Promise.resolve(url === null ? null : { url, updated: 1 })) },
        ZEEK_PROXY_SECRET: "s",
      }) as unknown as Env;
    const request = (path: string) =>
      new Request(`https://playzeek.pages.dev${path}`, {
        headers: { "CF-Connecting-IP": "203.0.113.9" },
      });
    const upstream = (body: string, type: string, cacheControl: string) =>
      vi.fn(() =>
        Promise.resolve(
          new Response(body, {
            status: 200,
            headers: { "content-type": type, "cache-control": cacheControl },
          }),
        ),
      );

    it("keeps what the engine marks public, and serves it without the tunnel", async () => {
      const fetchMock = upstream("<html>card</html>", "text/html", "public, max-age=86400");
      vi.stubGlobal("fetch", fetchMock);
      const edge = fakeEdge();
      const e = env("https://a-b-c.trycloudflare.com");
      const first = await forward(
        request("/share/round/251"),
        e,
        "engine",
        "/share/round/251",
        edge,
      );
      expect(await first.text()).toBe("<html>card</html>");
      await Promise.all(edge.pending);
      expect(edge.store.size).toBe(1);
      const second = await forward(
        request("/share/round/251"),
        e,
        "engine",
        "/share/round/251",
        edge,
      );
      expect(await second.text()).toBe("<html>card</html>");
      expect(second.headers.get("cross-origin-embedder-policy")).toBe("require-corp");
      expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    it("still serves a kept share page while the engine is offline", async () => {
      vi.stubGlobal("fetch", upstream("<html>card</html>", "text/html", "public, max-age=86400"));
      const edge = fakeEdge();
      await forward(
        request("/share/win/78ea3288c9ef4fea6c07ec9eeb26c9e0"),
        env("https://a-b-c.trycloudflare.com"),
        "engine",
        "/share/win/78ea3288c9ef4fea6c07ec9eeb26c9e0",
        edge,
      );
      await Promise.all(edge.pending);
      const offline = await forward(
        request("/share/win/78ea3288c9ef4fea6c07ec9eeb26c9e0"),
        env(null),
        "engine",
        "/share/win/78ea3288c9ef4fea6c07ec9eeb26c9e0",
        edge,
      );
      expect(offline.status).toBe(200);
    });

    it("never keeps a bundle that may still change, or live data", async () => {
      vi.stubGlobal("fetch", upstream("{}", "application/json", "no-store"));
      const edge = fakeEdge();
      const e = env("https://a-b-c.trycloudflare.com");
      await forward(request("/api/v1/rounds/252/proof"), e, "engine", "/v1/rounds/252/proof", edge);
      vi.stubGlobal("fetch", upstream("{}", "application/json", "public, max-age=60"));
      await forward(request("/api/v1/board"), e, "engine", "/v1/board", edge);
      await Promise.all(edge.pending);
      expect(edge.store.size).toBe(0);
    });

    it("reads Cache-Control strictly", () => {
      expect(edgeCacheable("public, max-age=31536000, immutable")).toBe(true);
      expect(edgeCacheable("public, max-age=0")).toBe(false);
      expect(edgeCacheable("no-store")).toBe(false);
      expect(edgeCacheable("private, max-age=60")).toBe(false);
      expect(edgeCacheable(null)).toBe(false);
    });
  });
});
