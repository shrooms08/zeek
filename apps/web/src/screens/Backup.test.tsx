import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const words = Array.from({ length: 24 }, (_, i) => `word${i + 1}`).join(" ");
const dispatched = vi.hoisted(() => ({ events: [] as { type: string }[] }));
vi.mock("../api/PocketProvider", () => ({
  usePocket: () => ({
    pocket: () =>
      Promise.resolve({
        create: () => Promise.resolve({ address: "utest1x", birthdayHeight: 1 }),
        exportSeed: () => Promise.resolve(words),
      }),
    dispatch: (e: { type: string }) => dispatched.events.push(e),
  }),
}));

const { CreateCard } = await import("./WalletScreen");

describe("the backup step", () => {
  afterEach(cleanup);

  it("tapping away while the words show tells the wallet they are gone", async () => {
    const view = render(<CreateCard />);
    fireEvent.change(screen.getByTestId("create-pass"), {
      target: { value: "horse battery staple" },
    });
    fireEvent.change(screen.getByLabelText("Again"), { target: { value: "horse battery staple" } });
    fireEvent.submit(screen.getByTestId("create-pass").closest("form") as HTMLFormElement);
    await act(async () => {
      for (let i = 0; i < 4; i += 1) await Promise.resolve();
    });
    expect(screen.getByTestId("wallet-backup")).toBeTruthy();
    view.unmount();
    expect(dispatched.events.map((e) => e.type)).toEqual(["create", "created", "backup-left"]);
  });
});
