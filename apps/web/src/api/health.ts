/**
 * While the engine is offline (or the site is busy), one light probe of
 * `/v1/health` decides when live play comes back: every 15 s at first,
 * doubling to 2 minutes, paused while the tab is hidden. When it answers,
 * every engine query is refetched and the field goes live again.
 */

import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { onVisibilityChange, isHidden } from "../lib/visibility";
import { engine } from "./engine";

export const PROBE_FIRST_MS = 15_000;
export const PROBE_MAX_MS = 120_000;

/** Delay before probe `attempt` (0-based). */
export function probeDelay(attempt: number): number {
  return Math.min(PROBE_MAX_MS, PROBE_FIRST_MS * 2 ** Math.min(attempt, 4));
}

export function useEngineComeback(): { nextAt: number | null; checkNow: () => void } {
  const client = useQueryClient();
  const [nextAt, setNextAt] = useState<number | null>(null);
  const attempt = useRef(0);
  const timer = useRef<number | undefined>(undefined);

  const probe = useCallback(async () => {
    try {
      const h = await engine.health();
      if (h.ok) {
        await client.invalidateQueries();
        return true;
      }
    } catch {
      // still offline or busy
    }
    return false;
  }, [client]);

  const schedule = useCallback(() => {
    window.clearTimeout(timer.current);
    if (isHidden()) {
      setNextAt(null);
      return;
    }
    const delay = probeDelay(attempt.current);
    setNextAt(Date.now() + delay);
    timer.current = window.setTimeout(() => {
      void probe().then((back) => {
        if (back) return;
        attempt.current += 1;
        schedule();
      });
    }, delay);
  }, [probe]);

  useEffect(() => {
    schedule();
    const off = onVisibilityChange((hidden) => {
      if (hidden) {
        window.clearTimeout(timer.current);
        setNextAt(null);
      } else {
        schedule();
      }
    });
    return () => {
      off();
      window.clearTimeout(timer.current);
    };
  }, [schedule]);

  const checkNow = useCallback(() => {
    window.clearTimeout(timer.current);
    void probe().then((back) => {
      if (!back) schedule();
    });
  }, [probe, schedule]);

  return { nextAt, checkNow };
}
