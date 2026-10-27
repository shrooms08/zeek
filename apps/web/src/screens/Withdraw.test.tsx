import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { PocketError } from "@zeek/pocket";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const send = vi.hoisted(() => ({ fn: vi.fn() }));
vi.mock("../api/PocketProvider", () => ({
  usePocket: () => ({ pocket: () => Promise.resolve({ send: send.fn }) }),
}));

const { WithdrawCard } = await import("./WalletScreen");

const TEXT =
  "We couldn’t confirm it reached the chain. Check History in a minute before trying again.";

function fill() {
  fireEvent.change(screen.getByPlaceholderText("utest1…"), {
    target: { value: `utest1${"q".repeat(60)}` },
  });
  fireEvent.change(screen.getByLabelText(/Amount/), { target: { value: "0.01" } });
  fireEvent.change(screen.getByLabelText(/Passphrase/), { target: { value: "horse battery" } });
}

const withdraw = () => screen.getByRole("button", { name: /Withdraw/ }) as HTMLButtonElement;

describe("a withdrawal whose broadcast may have been sent", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("says so and holds Withdraw for 2 minutes", async () => {
    send.fn.mockRejectedValue(new PocketError("broadcast_unknown", TEXT));
    render(<WithdrawCard spendable={100_000_000n} onDone={() => undefined} />);
    fill();
    fireEvent.click(withdraw());
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole("status").textContent).toBe(TEXT);
    // The passphrase is cleared after any attempt; type it again.
    fireEvent.change(screen.getByLabelText(/Passphrase/), { target: { value: "horse battery" } });
    expect(withdraw().disabled).toBe(true);
    act(() => {
      vi.advanceTimersByTime(119_000);
    });
    expect(withdraw().disabled).toBe(true);
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(withdraw().disabled).toBe(false);
  });
});
