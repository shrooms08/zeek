import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { PocketError } from "@zeek/pocket";
import { afterEach, describe, expect, it, vi } from "vitest";

const pocket = vi.hoisted(() => ({ forget: vi.fn(), restore: vi.fn() }));
vi.mock("../api/PocketProvider", () => ({
  usePocket: () => ({
    pocket: () => Promise.resolve(pocket),
    dispatch: vi.fn(),
    refresh: () => Promise.resolve(),
  }),
}));

const { RestoreCard } = await import("./WalletScreen");

describe("Restore with 24 words", () => {
  afterEach(cleanup);

  it("a wrong word never removes the Pocket already in this browser", async () => {
    pocket.restore.mockRejectedValue(
      new PocketError("invalid_mnemonic", "word 7 is not a BIP 39 word"),
    );
    render(<RestoreCard onCancel={() => undefined} />);
    fireEvent.change(screen.getByLabelText("24 words"), {
      target: { value: Array(24).fill("abandon").join(" ") },
    });
    fireEvent.change(screen.getByLabelText("Birthday height"), { target: { value: "4500000" } });
    fireEvent.change(screen.getByLabelText("New passphrase"), {
      target: { value: "horse battery staple" },
    });
    fireEvent.submit(screen.getByLabelText("24 words").closest("form") as HTMLFormElement);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(pocket.forget).not.toHaveBeenCalled();
    expect(pocket.restore).toHaveBeenCalledWith(
      Array(24).fill("abandon").join(" "),
      4_500_000,
      "horse battery staple",
      true,
    );
    expect(screen.getByRole("alert").textContent).toContain("word 7 is not a BIP 39 word");
  });
});
