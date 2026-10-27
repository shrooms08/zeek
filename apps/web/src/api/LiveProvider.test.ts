import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { refreshMissed } from "./LiveProvider";

describe("after a gap in the stream", () => {
  it("refreshes the round a reveal is waiting on, not the board the stream pushes", async () => {
    const client = new QueryClient();
    client.setQueryData(["round", 2035], { round: 2035, settled: null });
    client.setQueryData(["pending"], []);
    client.setQueryData(["board"], { round: 2037 });
    await refreshMissed(client);
    expect(client.getQueryState(["round", 2035])?.isInvalidated).toBe(true);
    expect(client.getQueryState(["pending"])?.isInvalidated).toBe(true);
    expect(client.getQueryState(["board"])?.isInvalidated).toBe(false);
  });
});
