/**
 * The engine the app under test talks to. playwright.config.ts sets
 * VITE_ZEEK_ENGINE_URL to the hosted engine when .zeek-deploy names one
 * (Railway: the app calls it directly). Without one, the local preview
 * uses an engine on this Mac and a deployed site (ZEEK_E2E_APP) its /api.
 */

const APP = process.env.ZEEK_E2E_APP;
const CONFIGURED = process.env.VITE_ZEEK_ENGINE_URL;

export const APP_ENGINE = (
  CONFIGURED ?? (APP === undefined ? "http://127.0.0.1:8787" : `${APP}/api`)
).replace(/\/+$/, "");

function escape(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
}

/** Matches requests to that engine; with `path`, only that path (exact). */
export function engineRequests(path?: string): RegExp {
  const base = escape(APP_ENGINE);
  return new RegExp(path === undefined ? `^${base}/` : `^${base}${escape(path)}$`);
}
