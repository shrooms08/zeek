/**
 * Sound: short effects, loops (waiting for a reveal, the campfire) and
 * background music, from the files in public/sfx/. No dependencies:
 * HTMLAudioElement, with a small pool per effect so rapid tile picks
 * overlap cleanly.
 *
 * Nothing plays before `unlock()` (the first gesture), which also preloads
 * the effects and the waiting loop. On the field that gesture is the load
 * screen's tap, which plays the enter sound (`playIntro`) and fades the
 * music in after it. Music loads when it is first started and
 * the campfire loop the first time the campfire opens. Every `play()`
 * swallows a rejected promise: a browser that refuses to play is never an
 * error. Effects (loops other than music included) and music are muted
 * separately, remembered in this browser ("zeek.sfxMuted",
 * "zeek.musicMuted"). The music ducks during a reveal (from the rumble
 * until the result card) and while the campfire is open; the lower duck
 * wins. Sound is never needed to understand anything.
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
/**
 * Music under something else: the reveal (from the rumble until the result
 * card) and the open campfire. With both, the lower wins.
 */
export type Duck = "reveal" | "campfire";
const DUCK_LEVEL: Record<Duck, number> = { reveal: 0.05, campfire: 0.06 };
const UNDUCK_MS = 600;
/** The win sound follows the coin by this much. */
const WIN_AFTER_COIN_MS = 400;
/** The music's fade-in after the load screen's enter sound. */
const MUSIC_IN_MS = 1500;
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
  private ducks = new Set<Duck>();
  private listeners = new Set<() => void>();
  /** The load screen's enter sound is playing: the music waits for it. */
  private introPlaying = false;
  /** Until then (performance.now()), the load screen covers the app. */
  private introUntil = 0;
  /** A reveal's result reached before sound was unlocked. */
  private pendingResult: "win" | "miss" | null = null;
  private resultTimers = new Set<number>();

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
    // After this gesture's own handlers (the load screen's tap sets the
    // intro first), so a waiting result knows when the app is visible.
    if (this.pendingResult !== null) {
      window.setTimeout(() => {
        this.flushResult();
      }, 0);
    }
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
    if (name === "rumble") this.duckMusic("reveal", 0);
    audio.currentTime = 0;
    audio.volume = VOLUME[name];
    attempt(audio);
  }

  /** Starts a loop (a second call while it plays does nothing). */
  startLoop(name: Loop): void {
    // Playing already (and not fading out): leave its level alone.
    if (this.wanted.has(name) && this.loops.get(name)?.paused === false) return;
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

  /**
   * The load screen's tap (call after `unlock()`, in the same gesture):
   * plays `name`, then fades the music in when it ends. With effects muted,
   * or if the browser refuses it, the music fades in `fallbackMs` after the
   * tap instead (as the load screen leaves).
   */
  playIntro(name: Effect, fallbackMs: number): void {
    if (!this.unlocked || this.introPlaying) return;
    this.wanted.add("music");
    this.introPlaying = true;
    const tapped = performance.now();
    this.introUntil = tapped + fallbackMs;
    let started = false;
    const start = () => {
      if (started) return;
      started = true;
      this.introPlaying = false;
      this.fadeInMusic();
    };
    const fallback = () => {
      window.setTimeout(start, Math.max(0, tapped + fallbackMs - performance.now()));
    };
    const audio = this.pools.get(name)?.[0];
    if (this.sfxOff || audio === undefined) {
      fallback();
      return;
    }
    audio.addEventListener("ended", start, { once: true });
    audio.addEventListener("error", fallback, { once: true });
    audio.currentTime = 0;
    audio.volume = VOLUME[name];
    try {
      void audio.play().catch(fallback);
    } catch {
      fallback();
    }
  }

  /**
   * A reveal's result: the coin, then the win 400 ms later, or the miss.
   * Reached before sound is unlocked (a reload landing on the result), it
   * plays at the unlock; after the load screen's tap, once the screen has
   * gone (`fallbackMs` of `playIntro`). Returns a cancel.
   */
  playResult(kind: "win" | "miss"): () => void {
    if (!this.unlocked) {
      this.pendingResult = kind;
    } else {
      this.scheduleResult(kind);
    }
    return () => {
      this.pendingResult = null;
      for (const id of this.resultTimers) window.clearTimeout(id);
      this.resultTimers.clear();
    };
  }

  private flushResult(): void {
    const kind = this.pendingResult;
    this.pendingResult = null;
    if (kind !== null) this.scheduleResult(kind);
  }

  private scheduleResult(kind: "win" | "miss"): void {
    const at = (ms: number, name: Effect) => {
      if (ms <= 0) {
        this.play(name);
        return;
      }
      const id = window.setTimeout(() => {
        this.resultTimers.delete(id);
        this.play(name);
      }, ms);
      this.resultTimers.add(id);
    };
    const wait = Math.max(0, this.introUntil - performance.now());
    if (kind === "miss") {
      at(wait, "miss");
    } else {
      at(wait, "coin");
      at(wait + WIN_AFTER_COIN_MS, "win");
    }
  }

  /** Lowers the music for `source` over `fadeMs` (the lowest duck wins). */
  duckMusic(source: Duck, fadeMs: number): void {
    this.ducks.add(source);
    this.levelMusic(fadeMs);
  }

  /** Ends `source`'s duck: back to the duck still active, or full. */
  releaseMusic(source: Duck, fadeMs = UNDUCK_MS): void {
    if (!this.ducks.delete(source)) return;
    this.levelMusic(fadeMs);
  }

  /** Brings the music back from the reveal's duck (the result card shows). */
  releaseDuck(): void {
    this.releaseMusic("reveal");
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
    if (name !== "music") return VOLUME[name];
    let level = VOLUME.music;
    for (const d of this.ducks) level = Math.min(level, DUCK_LEVEL[d]);
    return level;
  }

  /** Plays a wanted, unmuted loop that is not playing yet. */
  private resume(name: Loop): void {
    if (!this.unlocked || !this.wanted.has(name) || this.muted(name)) return;
    // The intro is playing: the music starts when it ends.
    if (name === "music" && this.introPlaying) return;
    const audio = this.loop(name);
    this.fades.get(name)?.();
    this.fades.delete(name);
    audio.volume = this.loopVolume(name);
    if (audio.paused) attempt(audio);
  }

  /** Starts the music (wanted, unmuted) from silence, up to its level over MUSIC_IN_MS. */
  private fadeInMusic(): void {
    if (!this.unlocked || !this.wanted.has("music") || this.musicOff) return;
    const audio = this.loop("music");
    if (!audio.paused) return;
    this.fades.get("music")?.();
    audio.volume = 0;
    attempt(audio);
    this.fades.set(
      "music",
      fade(audio, this.loopVolume("music"), MUSIC_IN_MS, () => {
        this.fades.delete("music");
      }),
    );
  }

  /** Moves the music to its current level over `fadeMs`. */
  private levelMusic(fadeMs: number): void {
    const music = this.loops.get("music");
    if (music === undefined) return;
    this.fades.get("music")?.();
    this.fades.set(
      "music",
      fade(music, this.loopVolume("music"), fadeMs, () => {
        this.fades.delete("music");
      }),
    );
  }
}

/** The app's one sound player. */
export const sound = new Sound();
