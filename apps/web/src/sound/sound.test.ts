import { beforeEach, describe, expect, it, vi } from "vitest";

/** A stand-in for HTMLAudioElement that records what the module asks of it. */
class FakeAudio {
  static made: FakeAudio[] = [];
  src: string;
  preload = "";
  volume = 1;
  loop = false;
  paused = true;
  currentTime = 0;
  duration = Number.NaN;
  plays = 0;
  loads = 0;
  constructor(src: string) {
    this.src = src;
    FakeAudio.made.push(this);
  }
  load() {
    this.loads += 1;
  }
  play() {
    this.plays += 1;
    this.paused = false;
    return Promise.reject(new Error("autoplay refused"));
  }
  pause() {
    this.paused = true;
  }
  addEventListener() {}
}

async function freshSound() {
  vi.resetModules();
  FakeAudio.made = [];
  vi.stubGlobal("Audio", FakeAudio);
  return (await import("./sound")).sound;
}

const named = (name: string) => FakeAudio.made.filter((a) => a.src === `/sfx/${name}.mp3`);

describe("sound", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("plays nothing and loads nothing before unlock()", async () => {
    const sound = await freshSound();
    sound.play("tile-pick");
    sound.startLoop("music");
    expect(FakeAudio.made).toHaveLength(0);
  });

  it("preloads the effects at unlock, but not the music or the campfire loop", async () => {
    const sound = await freshSound();
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
    const sound = await freshSound();
    sound.unlock();
    expect(() => {
      sound.play("crack");
    }).not.toThrow();
    expect(named("crack")[0]?.volume).toBe(0.5);
    expect(named("crack")[0]?.plays).toBe(1);
  });

  it("rapid tile picks use different copies", async () => {
    const sound = await freshSound();
    sound.unlock();
    sound.play("tile-pick");
    sound.play("tile-pick");
    expect(named("tile-pick").filter((a) => a.plays === 1)).toHaveLength(2);
  });

  it("remembers both mutes; muted effects do not play", async () => {
    let sound = await freshSound();
    sound.sfxMuted = true;
    sound.musicMuted = true;
    expect(localStorage.getItem("zeek.sfxMuted")).toBe("1");
    expect(localStorage.getItem("zeek.musicMuted")).toBe("1");
    sound = await freshSound();
    expect(sound.sfxMuted).toBe(true);
    expect(sound.musicMuted).toBe(true);
    sound.unlock();
    sound.play("dig");
    expect(named("dig").every((a) => a.plays === 0)).toBe(true);
  });

  it("muting music pauses its loop; unmuting resumes it", async () => {
    const sound = await freshSound();
    sound.unlock();
    sound.startLoop("music");
    const music = named("music")[0];
    expect(music?.paused).toBe(false);
    sound.musicMuted = true;
    expect(music?.paused).toBe(true);
    sound.musicMuted = false;
    expect(music?.paused).toBe(false);
  });

  it("the rumble ducks the music until the result card releases it", async () => {
    vi.useFakeTimers();
    const sound = await freshSound();
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
});

describe("the first gesture", () => {
  it("unlocks sound, plays enter and starts the music (not when music is muted)", async () => {
    let sound = await freshSound();
    const { unlockOnFirstGesture } = await import("./cues");
    unlockOnFirstGesture();
    expect(FakeAudio.made).toHaveLength(0);
    window.dispatchEvent(new Event("pointerdown"));
    expect(named("enter")[0]?.plays).toBe(1);
    expect(named("music")[0]?.paused).toBe(false);
    // Once only.
    window.dispatchEvent(new Event("keydown"));
    expect(named("enter")[0]?.plays).toBe(1);

    localStorage.setItem("zeek.musicMuted", "1");
    sound = await freshSound();
    const again = await import("./cues");
    again.unlockOnFirstGesture();
    window.dispatchEvent(new Event("keydown"));
    expect(sound.musicMuted).toBe(true);
    expect(named("enter")[0]?.plays).toBe(1);
    expect(named("music").every((a) => a.paused)).toBe(true);
  });
});
