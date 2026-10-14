/** Live Campfire messages from the engine's WebSocket. */

import type { CampfireMessage } from "./types.js";

function isCampfire(value: unknown): value is { event: "campfire" } & CampfireMessage {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as { event?: unknown }).event === "campfire" &&
    typeof (value as { id?: unknown }).id === "string"
  );
}

/**
 * Calls `onMessage` for every Campfire message the engine announces (mined
 * after the engine started) until the returned function is called.
 */
export function subscribeCampfire(
  engineUrl: string,
  onMessage: (message: CampfireMessage) => void,
): () => void {
  const url = new URL("/v1/stream", engineUrl);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(url);
  socket.onmessage = (event: MessageEvent<string>) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(event.data);
    } catch {
      return;
    }
    if (isCampfire(parsed)) {
      onMessage(parsed);
    }
  };
  return () => {
    socket.close();
  };
}
