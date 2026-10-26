import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** A stand-in for HTMLAudioElement that records what the module asks of it. */
class FakeAudio {
  static made: FakeAudio[] = [];
  /** Whether the browser allows play() (autoplay policy). */
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

describe("the load screen's intro", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("allowed: enter plays with the logo and the music follows when it ends", async () => {
    FakeAudio.allow = true;
    const { sound, cues } = await freshSound();
    cues.unlockOnFirstGesture();
    sound.tryAutoplay("enter");
    await settle();
    const enter = named("enter")[0];
    expect(enter?.plays).toBe(1);
    expect(named("music")).toHaveLength(0);
    // A gesture during the intro does not start the music early.
    window.dispatchEvent(new Event("pointerdown"));
    expect(named("music").every((a) => a.paused)).toBe(true);
    enter?.end();
    expect(named("music")[0]?.paused).toBe(false);
    // Enter never plays again.
    expect(named("enter").reduce((n, a) => n + a.plays, 0)).toBe(1);
  });

  it("blocked: nothing plays until the first gesture, which starts music but not enter", async () => {
    FakeAudio.allow = false;
    const { sound, cues } = await freshSound();
    cues.unlockOnFirstGesture();
    sound.tryAutoplay("enter");
    await settle();
    expect(named("music")).toHaveLength(0);
    FakeAudio.allow = true;
    window.dispatchEvent(new Event("pointerdown"));
    expect(named("music")[0]?.paused).toBe(false);
    // Only the refused attempt: enter is not replayed by the gesture.
    expect(named("enter").reduce((n, a) => n + a.plays, 0)).toBe(1);
  });

  it("respects the mutes: no attempt with effects muted, no music after it with music muted", async () => {
    FakeAudio.allow = true;
    localStorage.setItem("zeek.sfxMuted", "1");
    let { sound } = await freshSound();
    sound.tryAutoplay("enter");
    await settle();
    expect(named("enter")).toHaveLength(0);

    localStorage.clear();
    localStorage.setItem("zeek.musicMuted", "1");
    ({ sound } = await freshSound());
    sound.tryAutoplay("enter");
    await settle();
    named("enter")[0]?.end();
    expect(named("music").every((a) => a.paused)).toBe(true);
  });

  it("an intro that starts only after the load screen has gone is stopped", async () => {
    FakeAudio.allow = true;
    const { sound } = await freshSound();
    sound.tryAutoplay("enter");
    sound.cancelAutoplay();
    await settle();
    expect(named("enter")[0]?.paused).toBe(true);
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
