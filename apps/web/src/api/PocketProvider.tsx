/**
 * Zeek Pocket for the whole app: the worker (and its 13 MB of wasm) starts
 * on first use, and the wallet lifecycle (no wallet / locked / unlocked /
 * syncing) is a reducer every screen reads.
 */

import { Pocket } from "@zeek/pocket";
import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useReducer,
  useRef,
  type ReactNode,
} from "react";
import {
  initialWallet,
  walletReducer,
  type WalletEvent,
  type WalletState,
} from "../machines/wallet";
import { ENGINE_URL, PINNED_VAULT } from "./engine";

interface PocketApi {
  /** Starts the worker if needed. */
  pocket: () => Promise<Pocket>;
  wallet: WalletState;
  dispatch: (event: WalletEvent) => void;
  /** Re-reads the wallet status from the worker. */
  refresh: () => Promise<void>;
}

const PocketContext = createContext<PocketApi | undefined>(undefined);

export function PocketProvider({ children }: { children: ReactNode }) {
  const started = useRef<Promise<Pocket> | undefined>(undefined);
  const [wallet, dispatch] = useReducer(walletReducer, initialWallet);
  const pocket = useCallback(() => {
    started.current ??= Pocket.start(ENGINE_URL, PINNED_VAULT);
    return started.current;
  }, []);
  const refresh = useCallback(async () => {
    const p = await pocket();
    const status = await p.status();
    dispatch({ type: "status", exists: status.exists, unlocked: status.unlocked });
  }, [pocket]);
  const value = useMemo(() => ({ pocket, wallet, dispatch, refresh }), [pocket, wallet, refresh]);
  return <PocketContext.Provider value={value}>{children}</PocketContext.Provider>;
}

export function usePocket(): PocketApi {
  const value = useContext(PocketContext);
  if (value === undefined) throw new Error("usePocket outside PocketProvider");
  return value;
}
