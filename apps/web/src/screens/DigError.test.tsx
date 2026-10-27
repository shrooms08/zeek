import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api/PocketProvider", () => ({ usePocket: () => ({}) }));
vi.mock("../api/pocketQueries", () => ({ usePocketBalance: () => ({ data: undefined }) }));
vi.mock("../api/queries", () => ({ useBlockSpacing: () => 25 }));

const { DigErrorModal } = await import("./DigPocketScreen");

const wrap = (children: ReactNode) => <MemoryRouter>{children}</MemoryRouter>;

describe("a dig that failed", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("maybe sent: says so, never 'Nothing was sent', and holds Try again for 2 minutes", () => {
    const onRetry = vi.fn();
    render(
      wrap(
        <DigErrorModal
          code="broadcast_unknown"
          message="We couldn’t confirm it reached the chain. Check History in a minute before trying again."
          onRetry={onRetry}
          onClose={() => undefined}
        />,
      ),
    );
    const modal = screen.getByTestId("dig-error");
    expect(modal.textContent).toContain(
      "We couldn’t confirm it reached the chain. Check History in a minute before trying again.",
    );
    expect(modal.textContent).not.toContain("Nothing was sent");
    const retry = screen.getByRole("button", { name: /Try again/ });
    expect((retry as HTMLButtonElement).disabled).toBe(true);
    act(() => {
      vi.advanceTimersByTime(119_000);
    });
    expect(screen.getByRole<HTMLButtonElement>("button", { name: /Try again/ }).disabled).toBe(
      true,
    );
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    const ready = screen.getByRole("button", { name: /Try again/ });
    expect((ready as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(ready);
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("refused: nothing was sent, and Try again works at once", () => {
    const onRetry = vi.fn();
    render(
      wrap(
        <DigErrorModal
          code="broadcast_rejected"
          message="tx-expiring-soon"
          onRetry={onRetry}
          onClose={() => undefined}
        />,
      ),
    );
    expect(screen.getByTestId("dig-error").textContent).toContain("Nothing was sent.");
    fireEvent.click(screen.getByRole("button", { name: /Try again/ }));
    expect(onRetry).toHaveBeenCalledOnce();
  });
});
