/**
 * Sound cues (design/mono Zeek Motion § 04), synthesized with the Web Audio
 * API: no audio files. Muted by default; one toggle in the menu, remembered
 * in this browser. Sound is never needed to understand anything.
 */

export type Cue = "thunk" | "pin" | "tick" | "crack" | "chime" | "rumble" | "crackle";

const KEY = "zeek.sound";

let ctx: AudioContext | null = null;

export function soundOn(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

export function setSoundOn(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? "1" : "0");
  } catch {
    // private window: the choice lasts this page only
  }
  if (on) context();
}

function context(): AudioContext {
  ctx ??= new AudioContext();
  if (ctx.state === "suspended") void ctx.resume();
  return ctx;
}

/** A decaying burst of filtered noise (soil, crackle, crack, rumble). */
function noise(
  c: AudioContext,
  at: number,
  length: number,
  freq: number,
  q: number,
  gain: number,
): void {
  const frames = Math.max(1, Math.floor(c.sampleRate * length));
  const buffer = c.createBuffer(1, frames, c.sampleRate);
  const data = buffer.getChannelData(0);
  for (let i = 0; i < frames; i++) data[i] = (Math.random() * 2 - 1) * (1 - i / frames) ** 2;
  const src = c.createBufferSource();
  src.buffer = buffer;
  const filter = c.createBiquadFilter();
  filter.type = "bandpass";
  filter.frequency.value = freq;
  filter.Q.value = q;
  const g = c.createGain();
  g.gain.value = gain;
  src.connect(filter).connect(g).connect(c.destination);
  src.start(at);
}

/** A short tone with an exponential decay. */
function tone(
  c: AudioContext,
  at: number,
  freq: number,
  length: number,
  gain: number,
  type: OscillatorType = "sine",
  glideTo?: number,
): void {
  const osc = c.createOscillator();
  osc.type = type;
  osc.frequency.setValueAtTime(freq, at);
  if (glideTo !== undefined) osc.frequency.exponentialRampToValueAtTime(glideTo, at + length);
  const g = c.createGain();
  g.gain.setValueAtTime(gain, at);
  g.gain.exponentialRampToValueAtTime(0.0001, at + length);
  osc.connect(g).connect(c.destination);
  osc.start(at);
  osc.stop(at + length + 0.02);
}

/** Plays a cue if sound is on. `quiet` is −12 dB (others' digs). */
export function play(cue: Cue, quiet = false): void {
  if (!soundOn()) return;
  const c = context();
  const t = c.currentTime + 0.01;
  const v = quiet ? 0.25 : 1;
  switch (cue) {
    case "thunk": // spade into packed soil
      tone(c, t, 110, 0.16, 0.5 * v, "sine", 55);
      noise(c, t, 0.12, 900, 0.8, 0.35 * v);
      return;
    case "pin": // a peg into the board
      tone(c, t, 1400, 0.05, 0.18 * v, "triangle", 900);
      return;
    case "tick":
      tone(c, t, 2000, 0.03, 0.12 * v, "square");
      return;
    case "crack":
      for (let k = 0; k < 4; k++) noise(c, t + k * 0.08, 0.09, 2400 - k * 300, 3, 0.4 * v);
      return;
    case "chime": // treasure: a rising triad
      [659.3, 830.6, 987.8, 1318.5].forEach((f, k) => {
        tone(c, t + k * 0.09, f, 0.9, 0.16 * v, "sine");
      });
      return;
    case "rumble":
      tone(c, t, 48, 1.1, 0.5 * v, "sawtooth", 32);
      noise(c, t, 1.0, 120, 0.7, 0.5 * v);
      return;
    case "crackle":
      for (let k = 0; k < 6; k++)
        noise(c, t + Math.random() * 0.4, 0.03, 3000 + Math.random() * 2000, 4, 0.25 * v);
      return;
  }
}
