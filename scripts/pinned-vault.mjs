// The vault a web build serves, read from the public game config at build
// time (ZEEK_GAME_CONFIG, default the testnet game). Zeek Pocket refuses an
// engine whose /v1/config names anything else (security review H3).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** @returns {{ network: string, vault_address: string, sealing_public_key: string, uivk: string }} */
export function pinnedVault() {
  const path =
    process.env.ZEEK_GAME_CONFIG ??
    fileURLToPath(new URL("../config/game.testnet.json", import.meta.url));
  const game = JSON.parse(readFileSync(path, "utf8"));
  return {
    network: game.params.network,
    vault_address: game.vault_address,
    sealing_public_key: game.sealing_public_key,
    uivk: game.uivk,
  };
}
