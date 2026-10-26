/**
 * When the app's sounds play (the files and volumes are in ./sound.ts).
 *
 * The dig sound marks the moment a dig of yours counts: the prize number
 * updates as it lands. A dig from the Pocket (or one auto-dig) marks itself
 * here when it is sent; the next time the prize grows, the field plays the
 * dig sound once. A mark not used within two minutes is dropped, so a later
 * dig by someone else never plays it.
 */

import { useEffect, useSyncExternalStore } from "react";
import { sound, type Loop } from "./sound";

const DIG_MARK_MS = 2 * 60_000;
let digMarkedAt: number | null = null;

/** A dig of this player's was just sent. */
export function markOwnDig(): void {
  digMarkedAt = Date.now();
}

/** Whether the prize growing now is this player's dig (once per mark). */
export function takeOwnDig(): boolean {
  const at = digMarkedAt;
  digMarkedAt = null;
  return at !== null && Date.now() - at <= DIG_MARK_MS;
}

/** Plays `name` while `active` (and this component is mounted). */
export function useLoop(name: Loop, active: boolean): void {
  useEffect(() => {
    if (!active) return;
    sound.startLoop(name);
    return () => {
      sound.stopLoop(name);
    };
  }, [name, active]);
}

/** The two mutes, kept in step with every toggle. */
export function useSoundSettings(): { sfxMuted: boolean; musicMuted: boolean } {
  const sfxMuted = useSyncExternalStore(subscribe, () => sound.sfxMuted);
  const musicMuted = useSyncExternalStore(subscribe, () => sound.musicMuted);
  return { sfxMuted, musicMuted };
}

function subscribe(listener: () => void): () => void {
  return sound.subscribe(listener);
}

/**
 * Sound needs a user gesture: the player's first pointerdown or keydown
 * anywhere unlocks it, plays the enter sound and starts the music (unless
 * muted). A tile picked by that first tap still plays its own sound (the
 * pick's click comes after the pointerdown).
 */
export function unlockOnFirstGesture(): void {
  const first = () => {
    window.removeEventListener("pointerdown", first, true);
    window.removeEventListener("keydown", first, true);
    sound.unlock();
    sound.play("enter");
    sound.startLoop("music");
  };
  window.addEventListener("pointerdown", first, true);
  window.addEventListener("keydown", first, true);
}
