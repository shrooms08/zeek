/**
 * The one WebSocket to the engine (`/v1/stream`), with reconnection and
 * exponential backoff (1 s doubling to 30 s, then 1, 2 and 5 minutes while
 * the engine stays away, with jitter). Events are handed to a listener
 * (which merges them into the query cache); connection state is published
 * so the UI can say "Reconnecting…" instead of showing stale numbers
 * silently. Every connection attempt is one request on the public site's
 * free daily allowance, so a hidden tab pauses the stream (`pause`) and
 * picks it up again on return (`resume`).
 */

import type { LiveEvent } from "./types";

export type LiveState = "connecting" | "open" | "reconnecting" | "paused";

export interface LiveOptions {
  url: string;
  onEvent: (event: LiveEvent) => void;
  onState: (state: LiveState) => void;
  /** Every open after the first (a drop, a hidden tab): events may have been missed. */
  onReconnect?: () => void;
  /** Injected for tests. */
  createSocket?: (url: string) => WebSocket;
  random?: () => number;
}

/** Delay before reconnect attempt `attempt` (0-based), in ms. */
export function backoffMs(attempt: number, random: () => number): number {
  const base =
    attempt < 6
      ? Math.min(30_000, 1_000 * 2 ** attempt)
      : Math.min(300_000, 60_000 * 2 ** Math.min(attempt - 6, 3));
  return Math.round(base * (0.75 + random() * 0.5));
}

function parse(data: unknown): LiveEvent | undefined {
  if (typeof data !== "string") return undefined;
  try {
    const value = JSON.parse(data) as { event?: unknown };
    return typeof value.event === "string" ? (value as LiveEvent) : undefined;
  } catch {
    return undefined;
  }
}

export class LiveClient {
  private socket: WebSocket | undefined;
  private attempt = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private stopped = false;
  private everOpened = false;
  private readonly options: LiveOptions;

  constructor(options: LiveOptions) {
    this.options = options;
  }

  start(): void {
    this.stopped = false;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.socket?.close();
  }

  /** Closes the stream without reconnecting (the tab is hidden). */
  pause(): void {
    this.stop();
    this.options.onState("paused");
  }

  /** Reconnects now, with the backoff reset (tab visible again, engine back). */
  resume(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    const open = this.socket?.readyState === 1 && !this.stopped;
    if (open) return;
    this.socket?.close();
    this.attempt = 0;
    this.stopped = false;
    this.connect();
  }

  private connect(): void {
    const create = this.options.createSocket ?? ((url: string) => new WebSocket(url));
    this.options.onState(this.attempt === 0 ? "connecting" : "reconnecting");
    const socket = create(this.options.url);
    this.socket = socket;
    socket.onopen = () => {
      this.attempt = 0;
      this.options.onState("open");
      if (this.everOpened) this.options.onReconnect?.();
      this.everOpened = true;
    };
    socket.onmessage = (message: MessageEvent<unknown>) => {
      const event = parse(message.data);
      if (event !== undefined) this.options.onEvent(event);
    };
    socket.onclose = () => {
      if (this.stopped) return;
      this.options.onState("reconnecting");
      const delay = backoffMs(this.attempt, this.options.random ?? Math.random);
      this.attempt += 1;
      this.timer = setTimeout(() => {
        this.connect();
      }, delay);
    };
    socket.onerror = () => {
      socket.close();
    };
  }
}

/** `ws://…/v1/stream` for an engine base URL (which may carry a path, such as `/api`). */
export function streamUrl(engineUrl: string): string {
  const url = new URL(`${engineUrl.replace(/\/+$/, "")}/v1/stream`);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}
