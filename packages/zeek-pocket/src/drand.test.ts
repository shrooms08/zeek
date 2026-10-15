import { describe, expect, it } from "vitest";
import { fetchPublicBeacon } from "./drand.js";

const CHAIN = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971";
const SIG =
  "958ff30dbe3e49b6cbb0f7b62d99e1500a9a67562f38d08bad748490f607e16e118a733f2f4043bafd61fffeaede1f48";

function fake(answers: (Response | Error)[]): { fetch: typeof fetch; urls: string[] } {
  const urls: string[] = [];
  let k = 0;
  return {
    urls,
    fetch: (input) => {
      urls.push(typeof input === "string" ? input : input instanceof URL ? input.href : input.url);
      const a = answers[k++] ?? new Error("no more answers");
      return a instanceof Error ? Promise.reject(a) : Promise.resolve(a);
    },
  };
}

describe("drand public beacon", () => {
  it("takes the first relay that answers with the right round", async () => {
    const f = fake([
      new Error("network down"),
      new Response(JSON.stringify({ round: 33093153, signature: SIG, randomness: "f26d" })),
    ]);
    const out = await fetchPublicBeacon(CHAIN, 33093153, f.fetch);
    expect(out).toEqual({
      kind: "fetched",
      beacon: { round: 33093153, signature: SIG },
      source: "api2.drand.sh",
    });
    expect(f.urls[0]).toBe(`https://api.drand.sh/${CHAIN}/public/33093153`);
  });

  it("refuses an answer for another round", async () => {
    const wrong = () => new Response(JSON.stringify({ round: 1, signature: SIG }));
    const f = fake([wrong(), wrong(), wrong(), wrong()]);
    const out = await fetchPublicBeacon(CHAIN, 33093153, f.fetch);
    expect(out.kind).toBe("unreachable");
  });

  it("says why when no relay answers", async () => {
    const f = fake([
      new Response("", { status: 500 }),
      new Error("timeout"),
      new Error("timeout"),
      new Error("timeout"),
    ]);
    const out = await fetchPublicBeacon(CHAIN, 33093153, f.fetch);
    expect(out.kind).toBe("unreachable");
    if (out.kind === "unreachable") expect(out.reason).toContain("api.drand.sh: HTTP 500");
  });

  it("never fetches for a malformed chain or round", async () => {
    const f = fake([]);
    expect((await fetchPublicBeacon("nope", 5, f.fetch)).kind).toBe("unreachable");
    expect(f.urls).toHaveLength(0);
  });
});
