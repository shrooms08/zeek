/**
 * The dig being prepared: picked tiles (in order), amount per tile,
 * "dig separately", and an optional Campfire message ("with my next dig").
 * Shared by the field and the dig flows.
 */

import { createContext, useContext, useMemo, useState, type ReactNode } from "react";
import type { Zat } from "../lib/money";

export interface DigDraft {
  tiles: number[];
  amountEach: Zat;
  separate: boolean;
  message: string;
}

interface DraftApi extends DigDraft {
  toggle: (tile: number) => void;
  setTiles: (tiles: number[]) => void;
  clear: () => void;
  setAmountEach: (amount: Zat) => void;
  setSeparate: (separate: boolean) => void;
  setMessage: (message: string) => void;
}

const DraftContext = createContext<DraftApi | undefined>(undefined);

export function DraftProvider({ children }: { children: ReactNode }) {
  const [tiles, setTiles] = useState<number[]>([]);
  const [amountEach, setAmountEach] = useState<Zat>(1_000_000n);
  const [separate, setSeparate] = useState(false);
  const [message, setMessage] = useState("");
  const value = useMemo<DraftApi>(
    () => ({
      tiles,
      amountEach,
      separate,
      message,
      toggle: (tile) => {
        setTiles((t) => (t.includes(tile) ? t.filter((x) => x !== tile) : [...t, tile]));
      },
      setTiles,
      clear: () => {
        setTiles([]);
      },
      setAmountEach,
      setSeparate,
      setMessage,
    }),
    [tiles, amountEach, separate, message],
  );
  return <DraftContext.Provider value={value}>{children}</DraftContext.Provider>;
}

export function useDraft(): DraftApi {
  const value = useContext(DraftContext);
  if (value === undefined) throw new Error("useDraft outside DraftProvider");
  return value;
}
