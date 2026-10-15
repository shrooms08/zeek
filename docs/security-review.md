# Security review (Phase 8)

An agent that did not write the code reviewed the repository read-only in
two rounds on 2026-10-15:

- **First round:** the whole system before Phase 8 deployed it.
- **Second round:** a re-review of every Phase 8 change and of the new public
  surface (the Pages proxy, the tunnels, the scripts).

Below is every finding and what was done. "Fixed" means fixed at the root,
with a test where one is possible.

## High

| ID | Finding | Decision |
| --- | --- | --- |
| H1 | A dig whose sealed receiver does not open froze every later payout | **Fixed.** See H1 below. |
| H2 | All three key shares were readable by the account running the internet-facing engine | **Fixed** by `scripts/isolate-services.sh` (run by the operator with sudo). See H2 below. |
| H3 | The browser wallet took the vault address and sealing key from the engine | **Fixed.** See H3 below. |
| H4 | Behind the tunnel every client shared one rate-limit bucket | **Fixed.** See H4 below. |
| N1 | The first version of the H2 script ran as root on files the engine's account could rewrite | **Fixed.** See N1 below. |
| N2 | The mainnet DKG had run with all three shares under one account | **Fixed.** See N2 below. |

### H1: unopenable receivers

The coordinator and every keyholder now apply one rule,
`coordinator::payable_receiver`: a payout whose receiver does not open, or
opens to an invalid address, is unclaimable.

- **What happens to the round:** it is paid without that payout, and the
  payout's value stays in the vault.
- **Where it is recorded:** in the proof bundle, on the verifier's payout
  check (as Warn) and on the ledger (never as operator float).
- **The queue:** a failing round backs off (1 minute, doubling to 30)
  while later rounds are paid.
- **Tests:**
  - `verify_pczt::a_junk_receiver_is_left_in_the_vault_and_the_round_still_pays`
  - `coordinator::a_failing_round_backs_off_and_later_rounds_go_first`
  - `checks::unclaimable_entries_must_match_the_settlement`

### H2: separate users for the engine and keyholders

`scripts/isolate-services.sh` runs the engine and each keyholder as its own
macOS user. Each has a 0700 directory and a root-owned copy of its binary,
and runs as a LaunchDaemon. The engine's user:

- cannot read any key share;
- cannot write the repository;
- does not have the operator's sudo rights or Cloudflare login.

### H3: the wallet pins the vault

The web build pins the game's network, vault address, sealing key, viewing
key and parameters (`scripts/pinned-vault.mjs`). The Pocket refuses any
engine config that differs (`relay.test.ts`).

### H4: rate limits behind the tunnel

The tunnel reaches a separate public listener. That listener refuses
requests without the proxy's secret, compared in constant time. Client
limits key on the address the proxy reports (IPv6 per /64). Test:
`edge::tests`.

### N1: hardening the H2 script

The script now:

- refuses to run with uncommitted changes;
- refuses symbolic links in anything it moves;
- installs binaries and configs once, and afterwards only:
  - with the binary's SHA-256 from a build the operator ran (`--update-binary`);
  - or with `--replace-config` for configs.

Moving the engine to its own user (H2) also removes what N1 needed: the
engine's account can no longer write the repository.

### N2: re-running the mainnet DKG

`isolate-services.sh --mainnet-dkg` discarded the first mainnet vault. That
vault was never funded, and its address was never published. The script
then ran a new ceremony inside three separate users, so each share was
created directly in its own user's directory.

## Medium

| ID | Finding | Decision |
| --- | --- | --- |
| M1 | The public relay could get the shared upstream to throttle this host | **Partly fixed.** The engine relay and the second-source relay have per-client and global limits, and the site proxy limits every route. **Accepted:** testnet has a single public lightwalletd, so a separate upstream is impossible there (docs/lightwalletd-servers.md). |
| M2 | A database lock was held across network calls; the API queued on it | **Fixed.** The writer plus read-only pool means no lock is held across network, beacon, proving or signing. The stall test measured p95 0.8 ms (`tests/stall.rs`). |
| M3 | `/v1/stream` was unbounded | **Fixed.** 2,000 connections in total, 8 per client, 1 KiB client frames, and the stream ends on client data. |
| M4 | No CSP; spending without re-entering the passphrase; no auto-lock | **Fixed.** A strict CSP on the site and on every proxied response. Withdraw asks for the passphrase, checked inside the wallet core. The Pocket locks after 15 idle minutes. |
| M5 | DKG round-2 packages are signed but not encrypted | **Accepted for now.** All nodes are on one host and talk over localhost. It must be fixed before any keyholder moves to another machine (docs/mainnet-runbook.md, decision 3). |
| M6 | The sealing private key and the vault's full viewing key sit on the engine | **Accepted for now.** The engine now runs as its own user. Moving PCZT construction off the internet-facing process is listed for mainnet (runbook decision 3). |
| N3 | Engine HTML was served on the wallet's origin without a CSP | **Fixed.** Each proxied route may only answer with its own content types. Every proxied response gets a script-free CSP, and `Location` is no longer passed on. |
| N4 | The second-source relay was unthrottled and its secret unchecked | **Fixed.** It requires the proxy secret and has per-client and global limits. The Check page says that on testnet it reads the same lightwalletd as the engine. |
| N5 | IPv6 rotation sidestepped per-client limits; limiter memory grew; most routes had no limit | **Fixed** at the origin: /64 keys, the global ceiling checked first, at most 100,000 tracked clients, every public route limited. **Accepted:** the Workers Free quota of 100,000 requests a day can still be spent by one client. A pages.dev site cannot have a WAF rule, so the paid plan would be needed. |
| N6 | Proof submissions re-ran full verification | **Fixed.** A proof already stored is answered from the store, and at most two verifications run at once. |

## Low

| ID | Finding | Decision |
| --- | --- | --- |
| L1 | Error text leaked internals | **Fixed.** Generic 500 bodies in the engine API, the share pages, the engine relay and the second-source relay; details go to the log. |
| L2 | `mark_signed` was not atomic | **Fixed.** It is one transaction, with a test. |
| L3 | A stale keyholder could take a dig for float | **Fixed.** A keyholder refuses to sign when its newest block is older than 40 spacings or more than 3 blocks behind the chain tip (test). N8 below keeps the coordinator from proposing such notes. |
| L4 | Campfire showed invisible and text-direction characters | **Fixed** for zero-width, bidi, tag and BOM characters, with a test. **Accepted:** homoglyphs, and no NFKC normalisation. |
| L5 | `data/` was world-readable | **Fixed.** umask 077 in zeek-up and the agents; existing files tightened. |
| L6 | Share pages pointed at localhost | **Fixed.** The public and app URLs are the Pages origin. |
| L7 | Any-wallet players' payout addresses are public in proof bundles | **Accepted.** This is by design. How to play and the any-wallet flow tell players to paste a fresh address. |
| L8 | A tampered envelope could demand huge Argon2 costs | **Fixed.** Exactly the version-1 costs are required (test). |
| N7 | Unclaimable payouts verified as Pass | **Fixed.** They show as Warn with the reason, control characters stripped. Keyholders refuse to start when their sealing key is not the game's. |
| N8 | Float planning could pick other rounds' dig notes | **Fixed.** No note the game knows is ever float (test). |
| N9 | Read connections were unbounded | **Fixed.** 8 pooled plus at most 32 extra, then 503. |
| N10 | One client could hold every stream | **Fixed** (8 per client). |
| N11 | The pin did not cover the game parameters | **Fixed.** The parameters are pinned too (test). |
| N12 | Removing a wallet needs no passphrase | **Accepted.** "Restore with 24 words" must replace a wallet whose passphrase is forgotten. Wiping needs script on the site's origin, which the CSP blocks. |

## Information items and tooling

- The pool's majority check compares block hashes, not tree states. A server
  that hides outputs while keeping correct hashes is not caught. This is
  noted in docs/lightwalletd-servers.md.
- Do not build a mainnet web bundle before digging opens: the pinned
  address would be in its JavaScript (docs/mainnet-runbook.md).
- `pnpm audit`: no known vulnerabilities. Before this, five advisories in
  development tooling were fixed with vitest 4.1.11 and sharp >= 0.35.5.
- `cargo audit`: the four rustls advisories (RUSTSEC-2026-0346 to 0349)
  were fixed with rustls 0.23.46. One warning is accepted:
  `atomic-polyfill` is unmaintained (RUSTSEC-2023-0089). It is not a
  vulnerability; it comes in through `postcard`/`heapless` 0.7 under
  frost-core and cannot be removed here.
- **Git history:** 55 local secret values, the mainnet vault address and
  viewing key, and the patterns for PEM keys, share fields, private-key
  fields and API tokens were searched across every commit. None was found.
  The one 24-word match is the sample backup phrase drawn in
  `design/mono/Zeek More.html`.

## Phase 8b additions (self-assessed)

These were not part of the independent review; each is a decision.

| Change | Risk | Decision |
| --- | --- | --- |
| Copy words (recovery phrase) | The phrase reaches the system clipboard, which other apps and clipboard history can read. | **Accepted with mitigations.** The warning sits next to the button; after 60 s the clipboard is overwritten if it still holds the phrase (never with a permission prompt); the phrase is never logged, stored or sent; the write-it-down check stays. Clipboard history managers may keep a copy, which the warning covers. |
| Pro auto-dig | The Pocket spends in later rounds without a passphrase per dig, and the 15-minute auto-lock waits while a run is active. | **Accepted.** The passphrase is checked once at the start (the worker's `confirmPassphrase`; nothing secret returned), the spend cap and N rounds bound the total, and the run stops on low balance, the cap, a whole round hidden, any error, or Stop. It lives only in the open tab. |
| Edge cache in the proxy | A poisoned or stale cached answer. | **Accepted.** Only three route shapes (share pages and cards, round proof bundles, stored proofs) are cached, only when the engine itself marks the answer `public` with a max-age, keyed by the full public URL; the engine is reached only through the secret-guarded tunnel from KV. Bundles still awaiting their payout are `no-store`. |
| drand relays in `connect-src` | Four more hosts the page may contact. | **Accepted.** They are drand's own public relays, read only for the beacon of a saved round, and the BLS signature is verified locally regardless of what they answer. |
| Replay snapshot | Publishing round data in the build. | **Accepted.** It is the same public data the engine serves (`/v1/rounds/{n}/proof`, `/v1/config`); `snapshot-rounds.mjs` refuses an engine that withholds its vault (mainnet before digging opens). |
