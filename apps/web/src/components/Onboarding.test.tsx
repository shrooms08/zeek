import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const boot = vi.hoisted(() => ({ gone: [] as (() => void)[] }));
vi.mock("../boot", () => ({
  whenBootGone: (cb: () => void) => {
    boot.gone.push(cb);
    return () => undefined;
  },
}));

const { Onboarding } = await import("./Onboarding");

describe("onboarding on a first visit", () => {
  afterEach(cleanup);

  it("takes focus only once the load screen has gone", () => {
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }));
    render(<Onboarding onDone={() => undefined} />);
    const next = screen.getByTestId("onboarding-next");
    expect(document.activeElement).not.toBe(next);
    act(() => {
      for (const cb of boot.gone) cb();
    });
    expect(document.activeElement).toBe(next);
    vi.unstubAllGlobals();
  });
});
