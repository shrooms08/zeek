import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

const splitPlan = vi.fn();
vi.mock("../api/PocketProvider", () => ({
  usePocket: () => ({ pocket: () => Promise.resolve({ splitPlan, prepareCoins: vi.fn() }) }),
}));
vi.mock("../api/pocketQueries", () => ({
  usePocketBalance: () => ({ data: { all_ready_in_blocks: null } }),
}));
vi.mock("../api/queries", () => ({ useBlockSpacing: () => 25 }));

const { NotesShort } = await import("./DigPocketScreen");

function wrap(children: ReactNode) {
  return <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>;
}

describe("Check your dig: not enough coins to dig separately", () => {
  afterEach(cleanup);

  it("says so plainly and offers one payment", async () => {
    splitPlan.mockResolvedValue(null);
    const onOne = vi.fn();
    render(wrap(<NotesShort payments={2} notes={1} onOnePayment={onOne} />));
    expect(screen.getByTestId("notes-short-text").textContent).toContain(
      "sends 2 payments a few seconds apart",
    );
    expect(screen.getByTestId("notes-short-text").textContent).toContain(
      "Your Pocket has 1 coin ready.",
    );
    fireEvent.click(screen.getByTestId("notes-one-payment"));
    expect(onOne).toHaveBeenCalledOnce();
    // No split is possible (the plan is null): no split button.
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByTestId("notes-split")).toBeNull();
  });

  it("offers to split the coins first when a split is possible", async () => {
    splitPlan.mockResolvedValue({
      inputs: 1,
      totalZat: 6_000_000,
      parts: 2,
      partZat: 2_995_000,
      feeZat: 10_000,
    });
    render(wrap(<NotesShort payments={2} notes={1} onOnePayment={vi.fn()} />));
    expect((await screen.findByTestId("notes-split")).textContent).toBe(
      "Get my coins ready first (2 coins, fee 0.0001)",
    );
  });
});
