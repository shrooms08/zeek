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
  it("backs off exponentially with jitter, capped at 30 s", () => {
    expect(backoffMs(0, () => 0.5)).toBe(1_000);
    expect(backoffMs(3, () => 0.5)).toBe(8_000);
    expect(backoffMs(20, () => 0.5)).toBe(30_000);
    expect(backoffMs(0, () => 0)).toBe(750);
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

  it("derives the stream URL", () => {
    expect(streamUrl("http://127.0.0.1:8787")).toBe("ws://127.0.0.1:8787/v1/stream");
    expect(streamUrl("https://zeek.example")).toBe("wss://zeek.example/v1/stream");
  });
});
