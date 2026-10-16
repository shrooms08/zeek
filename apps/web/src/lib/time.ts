/** Time display. */

/** `m:ss` (or `h:mm:ss`) for a non-negative number of seconds. */
export function formatClock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const rest = (s % 60).toString().padStart(2, "0");
  return h > 0 ? `${h}:${m.toString().padStart(2, "0")}:${rest}` : `${m}:${rest}`;
}

/** `21:04` local time of a Unix timestamp. */
export function formatTimeOfDay(unix: number): string {
  const d = new Date(unix * 1000);
  return `${d.getHours().toString().padStart(2, "0")}:${d.getMinutes().toString().padStart(2, "0")}`;
}

/** "2 min ago", "3 h ago", for statuses. */
export function formatAgo(unix: number, now: number): string {
  const seconds = Math.max(0, now - unix);
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86_400)} d ago`;
}

/** Percent of the wait from the round's close to its beacon, 0 to 100. */
export function revealProgress(beaconTime: number, delay: number, now: number): number {
  if (delay <= 0) return 100;
  const done = (now - (beaconTime - delay)) / delay;
  return Math.round(Math.min(1, Math.max(0, done)) * 100);
}
