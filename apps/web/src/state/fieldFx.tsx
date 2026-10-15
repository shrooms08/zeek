/**
 * Effects the field plays for things that happen elsewhere: the ink flight
 * after your dig is sealed, and "sealing" (which pauses the props). The dig
 * modals sit over the field (child routes), so both are seen live.
 */

import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";

interface FieldFx {
  flight: { tiles: readonly number[]; n: number } | null;
  /** Ink flies from the seal to these tiles. */
  launch: (tiles: readonly number[]) => void;
  sealing: boolean;
  setSealing: (on: boolean) => void;
}

const Ctx = createContext<FieldFx | undefined>(undefined);

export function FieldFxProvider({ children }: { children: ReactNode }) {
  const [flight, setFlight] = useState<FieldFx["flight"]>(null);
  const [sealing, setSealing] = useState(false);
  const launch = useCallback((tiles: readonly number[]) => {
    setFlight((f) => ({ tiles, n: (f?.n ?? 0) + 1 }));
  }, []);
  const value = useMemo(() => ({ flight, launch, sealing, setSealing }), [flight, launch, sealing]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useFieldFx(): FieldFx {
  const v = useContext(Ctx);
  if (v === undefined) throw new Error("useFieldFx outside FieldFxProvider");
  return v;
}
