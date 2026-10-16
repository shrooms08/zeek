/**
 * An optional browser notification when a reveal lands. Permission is only
 * ever asked from a click ("Tell me when it lands"); nothing here throws,
 * and nothing is sent anywhere (the notification is local to this browser).
 */

export type NotifyState = "unsupported" | "default" | "granted" | "denied";

export function notifyState(): NotifyState {
  if (typeof window === "undefined" || !("Notification" in window)) return "unsupported";
  return Notification.permission;
}

/** Asks for permission (call from a click only). */
export async function askToNotify(): Promise<NotifyState> {
  if (notifyState() === "unsupported") return "unsupported";
  try {
    return await Notification.requestPermission();
  } catch {
    return notifyState();
  }
}

/** Shows `title` if allowed and this tab is not in front (best effort). */
export function notify(title: string, body: string): void {
  if (notifyState() !== "granted" || document.visibilityState === "visible") return;
  try {
    new Notification(title, { body, tag: "zeek-reveal" });
  } catch {
    // Some browsers only allow notifications from a service worker.
  }
}
