# Mainnet runbook

Zeek is mainnet-ready but **not open**. Digging is off:

- The engine runs in readiness mode (`ZEEK_DIGGING=off`). It refuses to
  start a payout coordinator or a public listener.
- `/v1/config` withholds the vault address, sealing key and viewing key.
- The mainnet game config, `config/game.mainnet.json`, is kept out of git.
  Nobody can be shown an address to pay.

This runbook covers what was checked and how to open mainnet when you decide
to.

## What mainnet looks like (verified 2026-10-15)

| Fact | Value | Source |
| --- | --- | --- |
| Active upgrade | NU6.3, consensus branch `0x37a5165b`, since height 3,428,143. It brings the Ironwood pool and v6 transactions. | [ZIP 258](https://zips.z.cash/zip-0258); `GetLightdInfo` from 8 mainnet servers |
| NU7 | Not active on mainnet. ZIP 259 lists the mainnet height as "TBD (To be set on OCT 20)". On testnet it activated at 4,465,026 (branch `0x77190ad9`). | [ZIP 259](https://zips.z.cash/zip-0259) |
| Block time | 75 s target (post-Blossom). NU7 would cut it to 25 s (ZIP 218). | [ZIP 208](https://zips.z.cash/zip-0208) |
| Earlier upgrades | NU6.1 at 3,146,400; NU6.2 at 3,364,600 (the Orchard circuit fix) | [ZIP 255](https://zips.z.cash/zip-0255), [ZIP 257](https://zips.z.cash/zip-0257) |

The vendored `zcash_protocol` (see `vendor/zcash_protocol/ZEEK-PATCH.md`)
has the same mainnet heights and **no** mainnet NU7 height. Its patch only
changes testnet NU7.

`crates/zeek-vault/tests/offline_pczt.rs`
`mainnet_ironwood_spend_is_v6_on_the_nu6_3_branch` builds, proves and 2-of-3
FROST-signs a mainnet Ironwood spend at height 3,518,600. The result is
version 6 on branch `0x37a5165b`, and the mainnet version check accepts it.
Nothing is broadcast.

## Game parameters

`GameParams::mainnet` (the mainnet game config):

- **Rounds:** 8 blocks × 75 s, about 10 minutes.
- **Reveal delay:** 2,100 s, the minimum the timing rule needs at 75 s
  spacing (2 × 75 × 12 + 300).
- **Rules:** v4 from round 0: text memos, Campfire and dig fee.
- **Money:**
  - Rake 5%; Deep Cache odds 1 in 625.
  - Minimum dig 0.001 ZEC; dig fee 0.00005 ZEC.
  - ZIP 317 marginal fee 0.00005 ZEC.
  - Initial Deep Cache 0.

## What exists

- **The mainnet vault.** A separate DKG ceremony on 2026-10-15 created a 2-of-3
  vault:
  - Keyholder configs: `config/mainnet/keyholder-N.json` (ports 9201–9203).
  - Coordinator identity: `.secrets/mainnet/coordinator`.
  - The three keyholders derived the same address.
  - The address is in the gitignored `config/game.mainnet.json` and
    `.secrets/mainnet/vault.json` only. Do not post it anywhere until step 6.
- **Key shares.** `sudo scripts/isolate-services.sh --mainnet-dkg` replaced
  the first mainnet vault with a new one. That first vault was made under
  the operator account, never funded, and its address never published. The
  new ceremony ran inside three separate system users (`_zeekmkh1–3`); each
  share was created directly in its own user's directory under
  `/Library/Application Support/Zeek/mainnet/`.
- **The readiness run.** It runs on 127.0.0.1:8797 with
  `data/mainnet/zeek-engine.sqlite`; see "Readiness run" below.

## Before opening: decisions

1. **NU7.** The parameters assume 75 s blocks. After NU7, eight blocks would
   last about 3.3 minutes, and the reveal delay would be far longer than
   needed. Open only after NU7 is live on mainnet, then do both:
   - Patch the vendored `zcash_protocol` with the mainnet NU7 height and
     branch ID from the final ZIP 259. Until then a build computes the wrong
     branch after activation; `lightwalletd::connect` refuses a branch it
     does not know, so it fails safe.
   - Set the game to `blocks_per_round: 24`, `target_spacing_secs: 25`,
     `reveal_delay_secs: 900`, with a genesis after activation.

   Switching parameters mid-game is not supported.
2. **Per-dig cap.** The rules enforce a minimum dig but no maximum. Pick a cap
   (for example 0.1 ZEC per tile per round). Enforcing it for every wallet
   needs a rule change: a rules version that refunds the excess. Until then,
   a cap in the app binds only Pocket players.
3. **Keyholder independence.** Today one person runs all three keyholders on
   one Mac, each under its own system account. Before real money:
   - Move at least one keyholder to another operator and machine.
   - Encrypt DKG round-2 packages to their recipients (security review M5).
     They travel over localhost today and are only signed.
   - Move PCZT construction away from the internet-facing engine (review
     M6).
4. **Legal.** Show "18+. Not available where prohibited." before the first
   dig, and check the rules where the operator lives. The app shows this
   notice whenever the network is mainnet.

## Opening mainnet (only after the decisions above)

1. **Re-check the vault.** Every mainnet keyholder must print the same
   address:

   ```sh
   for n in 1 2 3; do sudo -u _zeekmkh$n "/Library/Application Support/Zeek/bin/zeek-keyholder" address \
     --config "/Library/Application Support/Zeek/mainnet/keyholder-$n/config.json" | head -1; done
   ```

2. **Fresh genesis.** In `config/game.mainnet.json`:
   - Set `genesis_height` and `params.genesis_height` to a height about one
     hour ahead.
   - Set `params.text_memo_activation_height` and
     `params.campfire.activation_height` to the same height.
   - Apply decision 1 to the parameters.
   - Delete `data/mainnet/zeek-engine.sqlite`; it belongs to the readiness
     run.
3. **Start the mainnet keyholders** as daemons. Create plists like the
   testnet ones that `scripts/isolate-services.sh` writes:
   - label `dev.zeek.mainnet-keyholder-N`;
   - user `_zeekmkhN`;
   - config `/Library/Application Support/Zeek/mainnet/keyholder-N/config.json`.

   Then run `sudo launchctl bootstrap system <plist>` for each and check
   `curl 127.0.0.1:920N/v1/health`.
4. **Fund the operator float.** The float pays fee deficits (a Deep Cache
   payout spends float notes) and never touches player money.
   - Send for example 0.05 ZEC from any shielded wallet to the vault
     address.
   - Sync the vault wallet:

     ```sh
     ZEEK_SECRETS_DIR=.secrets/mainnet ZEEK_DATA_DIR=data/mainnet/wallet \
       ZEEK_LIGHTWALLETD_URL=https://zec.rocks:443 target/release/zeek-vault sync
     ```

   - Check `operator_float` on `/v1/vault/ledger`.
5. **Start the engine with digging on.** Use a separate engine instance with:
   - `ZEEK_DIGGING=on`
   - `ZEEK_GAME_CONFIG=config/game.mainnet.json`
   - `ZEEK_ENGINE_DB=data/mainnet/zeek-engine.sqlite`
   - `ZEEK_KEYHOLDERS` = the mainnet keyholders' URLs and identities
   - `ZEEK_COORDINATOR_IDENTITY=.secrets/mainnet/coordinator/identity.json`
   - `ZEEK_VAULT_SECRETS_DIR=.secrets/mainnet`
   - `ZEEK_VAULT_DATA_DIR=data/mainnet/wallet`
   - the mainnet `ZEEK_LIGHTWALLETD_URL` list (docs/lightwalletd-servers.md)
   - its own ports, public listener and proxy secret

   Do not reuse the testnet agents.
6. **Publish.**
   - Commit `config/game.mainnet.json`.
   - Build the web app with `ZEEK_GAME_CONFIG=config/game.mainnet.json`. The
     Pocket then pins the mainnet vault and refuses anything else.
   - Deploy it to a separate Pages project, with its own KV namespace and
     tunnel.
   - The vault address becomes public at this step, and only here.
7. **Watch the first rounds.**
   - `/status`, `/v1/vault/ledger` and `zeek-engine audit`.
   - The keyholders' refusals on the Trust page.

## Stopping

- **To stop new digs**, set `ZEEK_DIGGING=off` and redeploy the app. The
  address stays on chain. Digs already sent still settle, and the
  coordinator still pays them, as long as it runs.
- **Never stop the keyholders while rounds are owed.** Payouts wait for two of
  the three.

## Readiness run (2026-10-15)

See the "Mainnet readiness" section of `docs/examples/phase8/README.md` for
the measured results.
