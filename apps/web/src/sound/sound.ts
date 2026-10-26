/**
 * Sound: short effects, loops (waiting for a reveal, the campfire) and
 * background music, from the files in public/sfx/. No dependencies:
 * HTMLAudioElement, with a small pool per effect so rapid tile picks
 * overlap cleanly.
 *
 * Nothing plays before `unlock()` (the intro's Enter, a user gesture), which
 * also preloads the effects and the waiting loop. Music loads when it is
 * first started (Enter) and the campfire loop the first time the campfire
 * opens. Every `play()` swallows a rejected promise: a browser that refuses
 * to play is never an error. Effects (loops other than music included) and
 * music are muted separately, remembered in this browser ("zeek.sfxMuted",
 * "zeek.musicMuted"). During a reveal the music ducks from the rumble until
 * the result card shows. Sound is never needed to understand anything.
 */

export type Effect =
  "enter" | "tile-pick" | "dig" | "rumble" | "crack" | "draw" | "coin" | "win" | "miss";
export type Loop = "waiting-loop" | "campfire-loop" | "music";
type Name = Effect | Loop;

const VOLUME: Record<Name, number> = {
  "tile-pick": 1,
  enter: 1,
  dig: 1,
  rumble: 0.9,
  crack: 0.5,
  draw: 0.6,
  coin: 0.7,
  win: 0.6,
  miss: 0.5,
  "waiting-loop": 0.4,
  "campfire-loop": 0.25,
  music: 0.15,
};
/** Music's volume from the rumble until the reveal's result card. */
const DUCKED = 0.05;
const UNDUCK_MS = 600;
/** Copies per effect (tile picks come quickly). */
const POOL: Record<Effect, number> = {
  "tile-pick": 4,
  enter: 1,
  dig: 2,
  rumble: 1,
  crack: 1,
  draw: 1,
  coin: 1,
  win: 1,
  miss: 1,
};
const EFFECTS = Object.keys(POOL) as Effect[];
/** Loops preloaded with the effects; music and the campfire load on first use. */
const PRELOADED_LOOPS: readonly Loop[] = ["waiting-loop"];

const SFX_KEY = "zeek.sfxMuted";
const MUSIC_KEY = "zeek.musicMuted";

function readFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

function writeFlag(key: string, on: boolean): void {
  try {
    localStorage.setItem(key, on ? "1" : "0");
  } catch {
    // Storage unavailable (private window): the choice lasts this page only.
  }
}

function element(name: Name): HTMLAudioElement {
  const audio = new Audio(`/sfx/${name}.mp3`);
  audio.preload = "auto";
  audio.volume = VOLUME[name];
  return audio;
}

function attempt(audio: HTMLAudioElement): void {
  try {
    void audio.play().catch(() => undefined);
  } catch {
    // Some browsers throw instead of rejecting: silence either way.
  }
}

/** Moves `audio`'s volume to `to` over `ms`, then calls `done`; returns a cancel. */
function fade(audio: HTMLAudioElement, to: number, ms: number, done?: () => void): () => void {
  const from = audio.volume;
  if (ms <= 0) {
    audio.volume = to;
    done?.();
    return () => undefined;
  }
  const start = performance.now();
  const id = window.setInterval(() => {
    const t = Math.min(1, (performance.now() - start) / ms);
    audio.volume = Math.max(0, Math.min(1, from + (to - from) * t));
    if (t >= 1) {
      window.clearInterval(id);
      done?.();
    }
  }, 30);
  return () => {
    window.clearInterval(id);
  };
}

class Sound {
  private unlocked = false;
  private sfxOff = readFlag(SFX_KEY);
  private musicOff = readFlag(MUSIC_KEY);
  private pools = new Map<Effect, HTMLAudioElement[]>();
  private next = new Map<Effect, number>();
  private loops = new Map<Loop, HTMLAudioElement>();
  /** Loops asked for; a muted one resumes when unmuted. */
  private wanted = new Set<Loop>();
  private fades = new Map<Loop, () => void>();
  private ducked = false;
  private listeners = new Set<() => void>();

  /** Allows sound and preloads the effects; call from a user gesture. */
  unlock(): void {
    if (this.unlocked) return;
    this.unlocked = true;
    for (const name of EFFECTS) {
      const pool = Array.from({ length: POOL[name] }, () => element(name));
      for (const audio of pool) audio.load();
      this.pools.set(name, pool);
    }
    for (const name of PRELOADED_LOOPS) this.loop(name).load();
    for (const name of this.wanted) this.resume(name);
  }

  /** Plays an effect (overlapping copies of quick ones). The rumble ducks the music. */
  play(name: Effect): void {
    if (!this.unlocked || this.sfxOff) return;
    const pool = this.pools.get(name);
    if (pool === undefined) return;
    const i = (this.next.get(name) ?? 0) % pool.length;
    this.next.set(name, i + 1);
    const audio = pool[i];
    if (audio === undefined) return;
    if (name === "rumble") this.duck();
    audio.currentTime = 0;
    audio.volume = VOLUME[name];
    attempt(audio);
  }

  /** Starts a loop (a second call while it plays does nothing). */
  startLoop(name: Loop): void {
    this.wanted.add(name);
    this.resume(name);
  }

  /** Fades a loop out over `fadeMs`, then pauses it. */
  stopLoop(name: Loop, fadeMs = 400): void {
    this.wanted.delete(name);
    const audio = this.loops.get(name);
    if (audio === undefined || audio.paused) return;
    this.fades.get(name)?.();
    this.fades.set(
      name,
      fade(audio, 0, fadeMs, () => {
        this.fades.delete(name);
        audio.volume = this.loopVolume(name);
        // Asked for again during the fade: it keeps playing.
        if (!this.wanted.has(name)) audio.pause();
      }),
    );
  }

  /** Brings the music back from the reveal's duck (the result card shows). */
  releaseDuck(): void {
    if (!this.ducked) return;
    this.ducked = false;
    const music = this.loops.get("music");
    if (music === undefined) return;
    this.fades.get("music")?.();
    this.fades.set(
      "music",
      fade(music, VOLUME.music, UNDUCK_MS, () => {
        this.fades.delete("music");
      }),
    );
  }

  get sfxMuted(): boolean {
    return this.sfxOff;
  }

  set sfxMuted(on: boolean) {
    this.sfxOff = on;
    writeFlag(SFX_KEY, on);
    for (const name of ["waiting-loop", "campfire-loop"] as const) {
      if (on) this.loops.get(name)?.pause();
      else this.resume(name);
    }
    this.changed();
  }

  get musicMuted(): boolean {
    return this.musicOff;
  }

  set musicMuted(on: boolean) {
    this.musicOff = on;
    writeFlag(MUSIC_KEY, on);
    if (on) this.loops.get("music")?.pause();
    else this.resume("music");
    this.changed();
  }

  /** Calls `listener` whenever a mute changes; returns the unsubscribe. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private changed(): void {
    for (const l of this.listeners) l();
  }

  /** The loop's element, created (and so loaded) on first use. */
  private loop(name: Loop): HTMLAudioElement {
    let audio = this.loops.get(name);
    if (audio === undefined) {
      audio = element(name);
      audio.loop = true;
      this.loops.set(name, audio);
    }
    return audio;
  }

  private muted(name: Loop): boolean {
    return name === "music" ? this.musicOff : this.sfxOff;
  }

  private loopVolume(name: Loop): number {
    return name === "music" && this.ducked ? DUCKED : VOLUME[name];
  }

  /** Plays a wanted, unmuted loop that is not playing yet. */
  private resume(name: Loop): void {
    if (!this.unlocked || !this.wanted.has(name) || this.muted(name)) return;
    const audio = this.loop(name);
    this.fades.get(name)?.();
    this.fades.delete(name);
    audio.volume = this.loopVolume(name);
    if (audio.paused) attempt(audio);
  }

  private duck(): void {
    this.ducked = true;
    const music = this.loops.get("music");
    if (music === undefined) return;
    this.fades.get("music")?.();
    this.fades.delete("music");
    music.volume = DUCKED;
  }
}

/** The app's one sound player. */
export const sound = new Sound();
