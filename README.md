# Zeek

**A treasure-digging game on Zcash: everyone sees the field, no one sees
you.**

Play on testnet (free coins): **https://playzeek.pages.dev**

## How to play

1. Pick tiles on the 5×5 field and dig. A dig is a shielded Zcash payment
   (from the in-browser Zeek Pocket, or any Zcash wallet).
2. When the round closes (about 10 minutes), a public random beacon picks
   the treasure tile.
3. Everyone who dug that tile splits the prize. The game pays them
   automatically and privately.

## What is private, and how

| Everyone can see | Nobody can see |
| --- | --- |
| How deep each tile is dug, the prize, the Deep Cache, every round's result | Who dug, which wallet dug which tile, who won, where winnings go |

- **Digs are shielded payments.** The sender is never on chain.
- **Payout addresses are sealed.** Each dig carries a fresh diversified
  address of the player's wallet, sealed to the vault's key, so no two digs
  share an address. Only a round's winning addresses are opened, in memory,
  to pay them.
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

**Honest note:** today all three keyholders run on one Mac operated by the
Zeek team, as separate processes under separate macOS users with separate
key shares. They are not yet separate machines or separate operators
([plan](docs/mainnet-runbook.md)).

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

## Security

- [docs/security-review.md](docs/security-review.md): an independent
  review, every finding and what was done about it.
- [docs/third-party.md](docs/third-party.md): third-party code and
  licenses.

## License

MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)).
