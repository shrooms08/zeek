/** When the app's sounds play (the files and volumes are in ./sound.ts). */

import { useEffect, useSyncExternalStore } from "react";
import { sound, type Loop } from "./sound";

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
 * Sound needs a user gesture (unless the load screen's intro was allowed to
 * autoplay): the player's first pointerdown or keydown anywhere unlocks it
 * and starts the music (unless muted). It never plays the enter sound: that
 * belongs to the load screen. A tile picked or a DIG pressed by that first
 * tap still plays its own sound (their handlers run after this one).
 */
export function unlockOnFirstGesture(): void {
  const first = () => {
    window.removeEventListener("pointerdown", first, true);
    window.removeEventListener("keydown", first, true);
    sound.unlock();
    sound.startLoop("music");
  };
  window.addEventListener("pointerdown", first, true);
  window.addEventListener("keydown", first, true);
}
