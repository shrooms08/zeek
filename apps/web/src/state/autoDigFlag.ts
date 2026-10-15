/**
 * Whether a confirmed auto-dig is running. The Pocket's idle auto-lock
 * waits while it runs: the person asked for digs in rounds to come, with
 * their passphrase, and the run stops on its own (src/lib/autoDig.ts).
 */

let running = false;

export function setAutoDigRunning(value: boolean): void {
  running = value;
}

export function autoDigRunning(): boolean {
  return running;
}
