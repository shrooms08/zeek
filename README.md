# Zeek

Zeek is a treasure hunt on shielded Zcash. Every round, 25 tiles, one hides
the treasure. Pick your tiles, dig, and whoever found it splits the pot.
Nobody can see who dug, which digs are whose, or who won.

**Play (testnet):** https://playzeek.pages.dev · [Demo video](https://x.com/shroomsgotsol/status/2113639248524091881?s=20) · **Track:** Wildcard

## How to play

1. Open https://playzeek.pages.dev and tap to start.
2. Create a Zeek Pocket, the wallet that lives in your browser: choose a
   passphrase and write down its 24 words.
3. Get free testnet ZEC from the
   [testnet faucet](https://fauzec.com) (1 TAZ per address, once every 24
   hours; linked from the Pocket's wallet screen) and send it to your Pocket's address.
4. Pick tiles on the 5×5 field and press DIG. Each dig is a shielded payment
   of at least 0.001 ZEC.
5. Wait for the reveal. The round closes after about 10 minutes, and about
   15 minutes later a public beacon picks the treasure tile. If you dug it,
   your share arrives in your Pocket by itself.

## How it works

- **Shielded digs.** A dig is a shielded payment to the game's vault, with
  the round and tile in its memo. The sender is never on chain: the field
  shows how deep each tile is dug, never who dug it
  ([who can learn what](docs/developer.md#privacy-who-can-learn-what)).
- **A draw nobody controls.** The treasure tile comes from drand, the League
  of Entropy's public beacon, from a beacon round published after the round
  closes. Block timestamps prove the round closed first; if they can't, the
  round is voided and refunded ([details](#how-fairness-works)).
- **A 2-of-3 FROST vault.** The pot is held by a key split three ways by a
  distributed key generation. A payout needs two keyholders, and each one
  recomputes the round from the chain and refuses any payout that doesn't
  match it ([details](#the-2-of-3-vault)).
- **Sealed payout addresses.** Each dig carries a fresh address of the
  player's wallet, sealed so only the vault's side can open it, and only a
  round's winning addresses are ever opened, in memory. No two digs share an
  address, so wins can't be linked to each other or to a player.
- **The Deep Cache.** A 5% rake from every pot, plus every pot nobody wins,
  goes into the Deep Cache. Each round it breaks open with 1-in-625 odds and
  goes to that round's winners.
- **Lite and Pro.** Lite digs one tile, quickly. Pro digs many tiles, adds
  auto-dig and keeps your own history in your browser
  ([Lite and Pro](docs/web-app.md#lite-and-pro)).
- **The Campfire.** A dig can carry a short public message, and anyone can
  shout (a message alone, at least 0.0005 ZEC, which feeds the Deep Cache).
  Messages show the round, tile and time, never the sender
  ([rules](docs/developer.md#game-rules-zeek-core)).
- **Anyone can check a round,** in the browser or from the command line
  ([verify any round](#verify-any-round); [prove a win](docs/prove-a-win.md)).

## What is private, and how

| Everyone can see | Nobody can see |
| --- | --- |
| How deep each tile is dug, the prize, the Deep Cache, every round's result, and each dig's amount, round and tile (the testnet vault's viewing key is public, so anyone can check the board) | Who dug, which wallet dug which tile, which digs belong to the same player, who won |

- **Digs are shielded payments.** The sender is never on chain.
- **Payout addresses are sealed.** Each dig carries a fresh diversified
  address of the player's wallet, sealed (HPKE) to the vault's sealing key,
  so no two digs share an address. Only a round's winning addresses are
  opened, in memory, by the engine and the keyholders that pay them: they
  see those fresh addresses and amounts, never whose they are.
- **The wallet stays in the browser.** Zeek Pocket keeps its keys there
  (WebAssembly, passphrase-sealed) and asks the chain for decoy
  transactions alongside its own.

Details: [docs/developer.md](docs/developer.md#privacy-who-can-learn-what)
and [docs/browser-wallet.md](docs/browser-wallet.md).

## How fairness works

- **The draw comes from drand,** the League of Entropy's public beacon. The
  rules bind each round to a drand round published after it closes.
- **The chain proves the timing.** A round counts only if the timestamps of
  the blocks after it prove it closed before that beacon existed. Otherwise
  it is voided and refunded ([the argument](crates/zeek-core/src/timing.rs);
  [measured over 336 rounds](docs/examples/phase8c/reveal-margin.md)).
- **One copy of the rules,** in `zeek-core`. The engine, each keyholder and
  the browser verifier run the same Rust code.

## The 2-of-3 vault

- **No single key moves the money.** The vault is a 2-of-3 FROST threshold
  key from a distributed key generation; no single key ever exists.
- **Each keyholder checks before it signs.** It follows the chain on its own,
  recomputes every round, and signs only a payout that pays exactly that
  result. A tampered payout is refused:
  [a recording of all three refusing](docs/video/clips/refusal.mp4).

**Honest note:** today the engine and all three keyholders run on Railway,
as separate services in one project operated by the Zeek team. Each
keyholder has its own key share on its own volume and is reachable only on
Railway's private network. They are not separate operators, and nothing
here claims they run on separate machines ([plan](docs/mainnet-runbook.md);
hosting: [docs/deployment.md](docs/deployment.md)). Mainnet is not hosted.

## Verify any round

- **In the browser:** open https://playzeek.pages.dev/check and enter a
  round number.
- **From the command line**, against any lightwalletd you trust:

  ```sh
  cargo run --release -p zeek-verify --features cli -- \
    round 251 --engine https://playzeek.pages.dev/api --lightwalletd https://testnet.zec.rocks:443
  ```

  It checks the beacon, that every dig is accounted for, the timing rule, the
  settlement (recomputed) and the payout transaction (mined, proofs and
  signatures valid, paying exactly the result).
- **When the engine is offline,** the site plays back saved rounds and
  still checks what needs no chain data:
  [Replay mode](docs/web-app.md#replay-mode).

## Run it yourself

You need Rust (stable, plus nightly for the browser wallet's WebAssembly),
Node 24 and pnpm.

```sh
cargo test --workspace                 # the game, engine, keyholders, verifier
pnpm install && pnpm build:wasm        # the browser wallet
pnpm --filter web dev                  # the app, against an engine at 127.0.0.1:8787
```

Running the engine, the keyholders and the public site:
[docs/developer.md](docs/developer.md) and
[docs/deployment.md](docs/deployment.md).

## Repository map

| Path | What |
| --- | --- |
| `crates/zeek-core` | The game rules: rounds, memos, sealing, timing, settlement, proof bundles. Pure, no I/O, also builds for wasm |
| `crates/zeek-engine` | Follows the chain with the vault's viewing key, settles rounds, serves the board, coordinates payouts (never holds spend authority) |
| `crates/zeek-keyholder` | The three keyholder nodes: DKG, their own chain view, verify-then-sign |
| `crates/zeek-vault` | FROST keys, the vault wallet, PCZT signing, the lightwalletd pool |
| `crates/zeek-verify` | Independent round and win verification (CLI, and the browser through `zeek-pocket`) |
| `crates/zeek-pocket`, `packages/zeek-pocket` | Zeek Pocket, the browser wallet (Rust → wasm, plus its worker and API) |
| `crates/zeek-dig` | The test player tool |
| `apps/web` | The web app (and its Pages Functions proxy) |
| `scripts/` | Run, deploy, snapshot and measurement scripts |
| `docs/` | Design notes, runbooks, measured results (`docs/examples/`) |

## Roadmap

- zkSNARKs holder badge and holders-only rounds, once holders can prove
  membership without revealing which NFT they hold.
- Open mainnet. The code is mainnet-ready but digging is off and the vault
  is not hosted; the steps are in [docs/mainnet-runbook.md](docs/mainnet-runbook.md).
- Keyholders run by separate, independent operators. Today all three run in
  one Railway project operated by the Zeek team (see the note above).
- **Private rewards with Zcash Shielded Assets.** Once ZSAs reach Zcash mainnet, each round can mint a private ZEEK reward to the winners, while nobody can see who holds it. ZSAs are not on mainnet or in NU7 yet; a proof of concept on the ZSA test network comes first.

## Security

- [docs/security-review.md](docs/security-review.md): an independent
  review, every finding and what was done about it.
- [docs/third-party.md](docs/third-party.md): third-party code and
  licenses.

## License

MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)).
