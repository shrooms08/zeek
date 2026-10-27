import { describe, expect, it, vi } from "vitest";
import { backoffMs, LiveClient, streamUrl } from "./live";
import type { LiveEvent } from "./types";

class FakeSocket {
  onopen: (() => void) | null = null;
  onmessage: ((m: MessageEvent<unknown>) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  close(): void {
    this.onclose?.();
  }
}

describe("LiveClient", () => {
  it("backs off exponentially with jitter: 30 s, then minutes while the engine is away", () => {
    expect(backoffMs(0, () => 0.5)).toBe(1_000);
    expect(backoffMs(3, () => 0.5)).toBe(8_000);
    expect(backoffMs(5, () => 0.5)).toBe(30_000);
    expect(backoffMs(6, () => 0.5)).toBe(60_000);
    expect(backoffMs(7, () => 0.5)).toBe(120_000);
    expect(backoffMs(8, () => 0.5)).toBe(240_000);
    expect(backoffMs(20, () => 0.5)).toBe(300_000);
    expect(backoffMs(0, () => 0)).toBe(750);
  });

  it("an engine away for an hour costs about 20 connection attempts, not 120", () => {
    let t = 0;
    let n = 0;
    while (t < 3_600_000) {
      t += backoffMs(n, () => 0.5);
      n += 1;
    }
    expect(n).toBeLessThanOrEqual(22);
  });

  it("pauses without reconnecting, and resumes at once", () => {
    vi.useFakeTimers();
    const sockets: FakeSocket[] = [];
    const states: string[] = [];
    const client = new LiveClient({
      url: "ws://x/v1/stream",
      onEvent: () => undefined,
      onState: (s) => states.push(s),
      createSocket: () => {
        const s = new FakeSocket();
        sockets.push(s);
        return s as unknown as WebSocket;
      },
      random: () => 0.5,
    });
    client.start();
    sockets[0]?.onopen?.();
    client.pause();
    vi.advanceTimersByTime(600_000);
    expect(sockets).toHaveLength(1);
    expect(states.at(-1)).toBe("paused");
    client.resume();
    expect(sockets).toHaveLength(2);
    client.stop();
    vi.useRealTimers();
  });

  it("reports state, parses events and reconnects", () => {
    vi.useFakeTimers();
    const sockets: FakeSocket[] = [];
    const states: string[] = [];
    const events: LiveEvent[] = [];
    const client = new LiveClient({
      url: "ws://x/v1/stream",
      onEvent: (e) => events.push(e),
      onState: (s) => states.push(s),
      createSocket: () => {
        const s = new FakeSocket();
        sockets.push(s);
        return s as unknown as WebSocket;
      },
      random: () => 0.5,
    });
    client.start();
    sockets[0]?.onopen?.();
    sockets[0]?.onmessage?.({
      data: JSON.stringify({ event: "paid", round: 3, txid: "ab" }),
    } as MessageEvent<unknown>);
    sockets[0]?.onmessage?.({ data: "not json" } as MessageEvent<unknown>);
    sockets[0]?.close();
    expect(states).toEqual(["connecting", "open", "reconnecting"]);
    vi.advanceTimersByTime(1_000);
    expect(sockets).toHaveLength(2);
    expect(events).toEqual([{ event: "paid", round: 3, txid: "ab" }]);
    client.stop();
    vi.useRealTimers();
  });

  it("announces every reconnect, but not the first connection", () => {
    vi.useFakeTimers();
    const sockets: FakeSocket[] = [];
    let reconnects = 0;
    const client = new LiveClient({
      url: "ws://x/v1/stream",
      onEvent: () => undefined,
      onState: () => undefined,
      onReconnect: () => {
        reconnects += 1;
      },
      createSocket: () => {
        const s = new FakeSocket();
        sockets.push(s);
        return s as unknown as WebSocket;
      },
      random: () => 0.5,
    });
    client.start();
    sockets[0]?.onopen?.();
    expect(reconnects).toBe(0);
    // The engine restarts: the stream drops and comes back.
    sockets[0]?.close();
    vi.advanceTimersByTime(1_000);
    sockets[1]?.onopen?.();
    expect(reconnects).toBe(1);
    // The tab was hidden and comes back.
    client.pause();
    client.resume();
    sockets[2]?.onopen?.();
    expect(reconnects).toBe(2);
    client.stop();
    vi.useRealTimers();
  });

  it("derives the stream URL", () => {
    expect(streamUrl("http://127.0.0.1:8787")).toBe("ws://127.0.0.1:8787/v1/stream");
    expect(streamUrl("https://zeek.example")).toBe("wss://zeek.example/v1/stream");
    expect(streamUrl("https://playzeek.pages.dev/api/")).toBe(
      "wss://playzeek.pages.dev/api/v1/stream",
    );
  });
});
