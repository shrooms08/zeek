import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./sound/sound", () => ({
  sound: { unlock: vi.fn(), playIntro: vi.fn() },
}));

/** The load screen as index.html paints it, and onboarding's Next behind it. */
async function freshBoot() {
  vi.resetModules();
  document.body.innerHTML =
    '<div id="zeek-boot"><div class="zb-bar"></div></div><div id="root"><button id="next">Next</button></div>';
  return import("./boot");
}

describe("the load screen and the keyboard", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.stubGlobal("matchMedia", () => ({ matches: false }));
    vi.useFakeTimers({
      toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "performance"],
    });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });

  it("Enter starts the game even when something behind holds focus, and never reaches it", async () => {
    const boot = await freshBoot();
    const next = document.getElementById("next") as HTMLButtonElement;
    const reached: string[] = [];
    next.addEventListener("keydown", (e) => reached.push(e.key));
    boot.startBoot();
    boot.bootReady();
    vi.advanceTimersByTime(1_300);
    expect(document.getElementById("zeek-boot")?.getAttribute("role")).toBe("button");
    next.focus();
    next.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(document.getElementById("zeek-boot")?.classList.contains("zb-exit")).toBe(true);
    expect(reached).toEqual([]);
  });

  it("whenBootGone waits for the load screen to be removed", async () => {
    const boot = await freshBoot();
    let gone = 0;
    boot.whenBootGone(() => {
      gone += 1;
    });
    boot.startBoot();
    boot.bootReady();
    vi.advanceTimersByTime(1_300);
    document.getElementById("zeek-boot")?.dispatchEvent(new Event("pointerdown"));
    vi.advanceTimersByTime(3_000);
    expect(gone).toBe(0);
    vi.advanceTimersByTime(300);
    expect(document.getElementById("zeek-boot")).toBeNull();
    expect(gone).toBe(1);
    // With no load screen, at once.
    boot.whenBootGone(() => {
      gone += 1;
    });
    expect(gone).toBe(2);
  });
});
