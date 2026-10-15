/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_ZEEK_ENGINE_URL?: string;
}

/** Defined by vite.config.ts from the public game config. */
declare const __ZEEK_PINNED_VAULT__: import("@zeek/pocket").PinnedVault;
