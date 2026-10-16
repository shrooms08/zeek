import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { FaucetCard } from "./FaucetCard";

const sync = () => Promise.resolve();

describe("the faucet card follows the coins", () => {
  afterEach(cleanup);
  beforeEach(() => {
    localStorage.clear();
    // The person opened the faucet with an empty Pocket.
    localStorage.setItem("zeek-awaiting-coins", JSON.stringify({ from: "0", since: 1 }));
  });

  it("waits, then says 'on the way' while confirming, and 'dig now' only once spendable", () => {
    const props = { address: "utest1x", readyInBlocks: null, spacing: 25, sync };
    const { rerender } = render(<FaucetCard {...props} spendable={0n} pending={0n} />);
    expect(screen.getByTestId("faucet-waiting")).toBeTruthy();

    // 0.125 arrived, 4 of 10 confirmations: 6 more blocks of ~30 s.
    rerender(
      <FaucetCard {...props} spendable={0n} pending={12_500_000n} readyInBlocks={6} spacing={30} />,
    );
    expect(screen.getByTestId("faucet-coming").textContent).toContain("Test coins on the way");
    expect(screen.getByTestId("faucet-ready-in").textContent).toBe(
      "Ready in about 3 min (6 more blocks).",
    );
    expect(screen.queryByText("You can dig now.")).toBeNull();

    rerender(<FaucetCard {...props} spendable={12_500_000n} pending={0n} />);
    expect(screen.getByTestId("faucet-arrived").textContent).toContain("You can dig now.");
  });

  it("never says 'dig now' while the balance is 0", () => {
    const props = { address: "utest1x", spacing: 25, sync };
    const { rerender } = render(
      <FaucetCard {...props} spendable={0n} pending={0n} readyInBlocks={null} />,
    );
    rerender(<FaucetCard {...props} spendable={0n} pending={1n} readyInBlocks={1} />);
    expect(screen.queryByTestId("faucet-arrived")).toBeNull();
    expect(screen.getByTestId("faucet-ready-in").textContent).toBe(
      "Ready in under a minute (1 more block).",
    );
  });
});
