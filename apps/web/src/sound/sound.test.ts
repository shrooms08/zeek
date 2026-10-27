import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** A stand-in for HTMLAudioElement that records what the module asks of it. */
class FakeAudio {
  static made: FakeAudio[] = [];
  /** Whether the browser allows play(). */
  static allow = false;
  src: string;
  preload = "";
  volume = 1;
  loop = false;
  paused = true;
  currentTime = 0;
  duration = Number.NaN;
  plays = 0;
  loads = 0;
  private handlers = new Map<string, (() => void)[]>();
  constructor(src: string) {
    this.src = src;
    FakeAudio.made.push(this);
  }
  load() {
    this.loads += 1;
  }
  play() {
    this.plays += 1;
    if (!FakeAudio.allow) return Promise.reject(new Error("autoplay refused"));
    this.paused = false;
    return Promise.resolve();
  }
  pause() {
    this.paused = true;
  }
  addEventListener(type: string, fn: () => void) {
    this.handlers.set(type, [...(this.handlers.get(type) ?? []), fn]);
  }
  /** The clip finished. */
  end() {
    this.paused = true;
    for (const fn of this.handlers.get("ended") ?? []) fn();
  }
}

async function freshSound() {
  vi.resetModules();
  FakeAudio.made = [];
  vi.stubGlobal("Audio", FakeAudio);
  const sound = (await import("./sound")).sound;
  const cues = await import("./cues");
  return { sound, cues };
}

const named = (name: string) => FakeAudio.made.filter((a) => a.src === `/sfx/${name}.mp3`);
const settle = () => new Promise((r) => setTimeout(r, 0));

describe("sound", () => {
  beforeEach(() => {
    localStorage.clear();
    FakeAudio.allow = true;
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("plays nothing and loads nothing before unlock()", async () => {
    const { sound } = await freshSound();
    sound.play("tile-pick");
    sound.startLoop("music");
    expect(FakeAudio.made).toHaveLength(0);
  });

  it("preloads the effects at unlock, but not the music or the campfire loop", async () => {
    const { sound } = await freshSound();
    sound.unlock();
    expect(named("tile-pick").length).toBeGreaterThan(1);
    expect(named("waiting-loop")).toHaveLength(1);
    expect(named("music")).toHaveLength(0);
    expect(named("campfire-loop")).toHaveLength(0);
    sound.startLoop("music");
    expect(named("music")[0]?.loop).toBe(true);
    sound.startLoop("campfire-loop");
    expect(named("campfire-loop")).toHaveLength(1);
  });

  it("swallows a refused play and uses each effect's volume", async () => {
    FakeAudio.allow = false;
    const { sound } = await freshSound();
    sound.unlock();
    expect(() => {
      sound.play("crack");
    }).not.toThrow();
    await settle();
    expect(named("crack")[0]?.volume).toBe(0.5);
    expect(named("crack")[0]?.plays).toBe(1);
  });

  it("rapid tile picks use different copies", async () => {
    const { sound } = await freshSound();
    sound.unlock();
    sound.play("tile-pick");
    sound.play("tile-pick");
    expect(named("tile-pick").filter((a) => a.plays === 1)).toHaveLength(2);
  });

  it("remembers both mutes; muted effects do not play", async () => {
    let { sound } = await freshSound();
    sound.sfxMuted = true;
    sound.musicMuted = true;
    expect(localStorage.getItem("zeek.sfxMuted")).toBe("1");
    expect(localStorage.getItem("zeek.musicMuted")).toBe("1");
    ({ sound } = await freshSound());
    expect(sound.sfxMuted).toBe(true);
    expect(sound.musicMuted).toBe(true);
    sound.unlock();
    sound.play("dig");
    expect(named("dig").every((a) => a.plays === 0)).toBe(true);
  });

  it("muting music pauses its loop; unmuting resumes it", async () => {
    const { sound } = await freshSound();
    sound.unlock();
    sound.startLoop("music");
    const music = named("music")[0];
    expect(music?.paused).toBe(false);
    sound.musicMuted = true;
    expect(music?.paused).toBe(true);
    sound.musicMuted = false;
    expect(music?.paused).toBe(false);
  });
});

describe("the load screen's tap", () => {
  beforeEach(() => {
    localStorage.clear();
    FakeAudio.allow = true;
  });
  afterEach(() => {
    vi.useRealTimers();
    document.getElementById("zeek-boot")?.remove();
  });

  const plays = (name: string) => named(name).reduce((n, a) => n + a.plays, 0);

  it("plays enter once, then fades the music in from silence when it ends", async () => {
    const { sound, cues } = await freshSound();
    cues.unlockOnFirstGesture();
    sound.unlock();
    vi.useFakeTimers();
    sound.playIntro("enter", 3200);
    await vi.advanceTimersByTimeAsync(4000);
    expect(plays("enter")).toBe(1);
    // Another gesture during the intro does not start the music early.
    window.dispatchEvent(new Event("pointerdown"));
    expect(plays("music")).toBe(0);
    named("enter")[0]?.end();
    const music = named("music")[0];
    expect(music?.paused).toBe(false);
    expect(music?.volume).toBe(0);
    vi.advanceTimersByTime(750);
    expect(music?.volume).toBeGreaterThan(0.05);
    expect(music?.volume).toBeLessThan(0.1);
    // A gesture during the fade-in does not jump it to full.
    window.dispatchEvent(new Event("keydown"));
    expect(music?.volume).toBeLessThan(0.1);
    vi.advanceTimersByTime(800);
    expect(music?.volume).toBeCloseTo(0.15, 5);
    expect(plays("enter")).toBe(1);
    expect(plays("music")).toBe(1);
  });

  it("a refused enter: the music fades in at the fallback time", async () => {
    FakeAudio.allow = false;
    const { sound } = await freshSound();
    sound.unlock();
    vi.useFakeTimers();
    sound.playIntro("enter", 3200);
    await vi.advanceTimersByTimeAsync(3100);
    expect(plays("enter")).toBe(1);
    expect(plays("music")).toBe(0);
    await vi.advanceTimersByTimeAsync(200);
    expect(plays("music")).toBe(1);
    expect(named("music")[0]?.volume).toBeLessThan(0.15);
  });

  it("respects the mutes: effects muted, music at the fallback time; music muted, none", async () => {
    localStorage.setItem("zeek.sfxMuted", "1");
    let { sound } = await freshSound();
    sound.unlock();
    vi.useFakeTimers();
    sound.playIntro("enter", 3200);
    vi.advanceTimersByTime(3100);
    expect(plays("music")).toBe(0);
    vi.advanceTimersByTime(1700);
    expect(plays("enter")).toBe(0);
    expect(named("music")[0]?.paused).toBe(false);
    expect(named("music")[0]?.volume).toBeCloseTo(0.15, 5);
    vi.useRealTimers();

    localStorage.clear();
    localStorage.setItem("zeek.musicMuted", "1");
    ({ sound } = await freshSound());
    sound.unlock();
    sound.playIntro("enter", 3200);
    await settle();
    expect(plays("enter")).toBe(1);
    named("enter")[0]?.end();
    expect(plays("music")).toBe(0);
  });

  it("while the load screen waits for its tap, other gestures leave sound to it", async () => {
    const boot = document.createElement("div");
    boot.id = "zeek-boot";
    document.body.append(boot);
    const { cues } = await freshSound();
    cues.unlockOnFirstGesture();
    window.dispatchEvent(new Event("pointerdown"));
    expect(FakeAudio.made).toHaveLength(0);
    // Gone (a deep link, or after the tap): the next gesture starts the music.
    boot.remove();
    window.dispatchEvent(new Event("keydown"));
    expect(named("music")[0]?.paused).toBe(false);
    expect(plays("enter")).toBe(0);
  });
});

describe("a reveal's result", () => {
  beforeEach(() => {
    localStorage.clear();
    FakeAudio.allow = true;
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const plays = (name: string) => named(name).reduce((n, a) => n + a.plays, 0);

  it("a win: coin at once, win 400 ms later; a miss: miss", async () => {
    const { sound } = await freshSound();
    sound.unlock();
    vi.useFakeTimers();
    sound.playResult("win");
    expect(plays("coin")).toBe(1);
    expect(plays("win")).toBe(0);
    vi.advanceTimersByTime(400);
    expect(plays("win")).toBe(1);
    sound.playResult("miss");
    expect(plays("miss")).toBe(1);
  });

  it("reached before sound is unlocked, it plays once at the first gesture", async () => {
    const { sound, cues } = await freshSound();
    cues.unlockOnFirstGesture();
    vi.useFakeTimers();
    sound.playResult("miss");
    expect(FakeAudio.made).toHaveLength(0);
    window.dispatchEvent(new Event("pointerdown"));
    vi.advanceTimersByTime(0);
    expect(plays("miss")).toBe(1);
    window.dispatchEvent(new Event("keydown"));
    vi.advanceTimersByTime(5000);
    expect(plays("miss")).toBe(1);
  });

  it("behind the load screen, it plays when the screen has gone", async () => {
    const { sound } = await freshSound();
    vi.useFakeTimers();
    sound.playResult("win");
    // The tap: unlock, then the intro.
    sound.unlock();
    sound.playIntro("enter", 3200);
    vi.advanceTimersByTime(3100);
    expect(plays("coin")).toBe(0);
    vi.advanceTimersByTime(100);
    expect(plays("coin")).toBe(1);
    vi.advanceTimersByTime(400);
    expect(plays("win")).toBe(1);
  });

  it("leaving first cancels it", async () => {
    const { sound } = await freshSound();
    vi.useFakeTimers();
    const cancel = sound.playResult("miss");
    cancel();
    sound.unlock();
    vi.advanceTimersByTime(5000);
    expect(plays("miss")).toBe(0);
  });
});

describe("music ducking", () => {
  beforeEach(() => {
    localStorage.clear();
    FakeAudio.allow = true;
  });

  it("the rumble ducks the music until the result card releases it", async () => {
    vi.useFakeTimers();
    const { sound } = await freshSound();
    sound.unlock();
    sound.startLoop("music");
    const music = named("music")[0];
    expect(music?.volume).toBe(0.15);
    sound.play("rumble");
    expect(music?.volume).toBe(0.05);
    sound.releaseDuck();
    vi.advanceTimersByTime(700);
    expect(music?.volume).toBeCloseTo(0.15, 5);
    vi.useRealTimers();
  });

  it("the campfire lowers it to 0.06; with a reveal too the lower wins; each release returns to the other", async () => {
    vi.useFakeTimers();
    const { sound } = await freshSound();
    sound.unlock();
    sound.startLoop("music");
    const music = named("music")[0];
    sound.duckMusic("campfire", 400);
    vi.advanceTimersByTime(200);
    expect(music?.volume).toBeGreaterThan(0.06);
    vi.advanceTimersByTime(300);
    expect(music?.volume).toBeCloseTo(0.06, 5);
    sound.play("rumble");
    expect(music?.volume).toBeCloseTo(0.05, 5);
    sound.releaseDuck();
    vi.advanceTimersByTime(700);
    expect(music?.volume).toBeCloseTo(0.06, 5);
    sound.releaseMusic("campfire", 600);
    vi.advanceTimersByTime(700);
    expect(music?.volume).toBeCloseTo(0.15, 5);
    vi.useRealTimers();
  });
});
