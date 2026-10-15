# Phase 7a live results (Zcash testnet, Ironwood)

Driven by `apps/web/e2e/live/*` against the live engine and 2-of-3 keyholders,
2026-10-14/15. The screenshots in this folder are from the same runs
(`apps/web/e2e/screenshots.spec.ts`), at 1440 and 390 wide.

## Pocket funding (`zeek-dig send` and the old harness Pocket)

| From | ZEC | txid |
| --- | --- | --- |
| alice | 0.0033 | dea092af1d344e047bf17ae34efb99ab72a4dd2b23bf4e020d3c75a98b94033e |
| carol | 0.0071 | 4a5baae3bd9b286f766ec4f3f0108bdd9b1178008a7bda0c5d025d81dc35c92c |
| harness Pocket | 0.0210 | 5749d0b52dcdda4a9eed5ef1e4d9924ab6c1c4a56b01f6f2eb5ff193127cb88e |
| alice (faucet top-up) | 0.04 | e1ccfc0f9623cefcb7dbb189008a50f52269593b8553233bb77cefa671ab4849 |

## Round 135: 2-tile dig with a message, and another wallet

- **Pocket dig.** Tiles 7 and 19 at 0.001 each, with a Campfire message.
  - txid: 8f6801c74acb05df403ecf16ad726e38945676360a4db56b0de254ba377958da
  - The field's depth changed for both tiles, and the message showed in
    Campfire as R135 · TILE 7 19.
- **Other-wallet dig.** Bob pasted a fresh address and paid tile 17 with
  `zeek-dig pay-uri`. The waiting state resolved to confirmed.
- **Settlement.** The treasure was tile 4, so there was no winner and the pot
  of 0.00285 went to the Deep Cache. The result card matched: not this time.
- **Check a round.** All five checks pass or don't apply.
  - Without a second source, the cross-check is "unavailable".
  - With the second relay (`zeek-verify relay` on testnet.zec.rocks), it
    passes. That relay is independent of the engine but not of zec.rocks.

## Round 166: a dig that stopped part-way (real partial failure)

- **What happened.** A 25-tile dig stopped after 2 of its 5 batches when the
  relay's public lightwalletd timed out.
- **Sent:** tiles 1–12.
  - 9bffeb09d796c044325f8b88df5ecb2735a061632b8c5432e564740e31012942
  - 3423cfdf493e89ef649dd4548c15f0aee4fcc9521cf7a14bb3773c9038a79017
- **Not sent:** tiles 13–25. Nothing was resent, and the app showed exactly
  what landed.
- **Settlement.** The treasure was tile 20, so there was no winner.
- **Fix.** The Pocket now retries a dropped read between batches. It never
  retries a broadcast.

## Round 188: the guaranteed win

- **Pocket dig.** All 25 tiles at 0.001 in 5 batches (6 + 6 + 6 + 6 + 1).
  Later batches waited for change to confirm.
  - ac02c427fa2a0b8e7b9817f04433505860abcc82586dadacf93e99ef9b6e2caa
  - f477980424ef2a4eccd01d296d1d3bfbea3c199feec2a59ce0005a4cc44b7a80
  - 5b6b90f68dcd2a4ac8bbe058a1e36838988ceec67dfb9bc22e3a707635cf11ab
  - fefe41ed63fefc69a7b4eb030412e8937925008082000c2ec17ff1e11a14bf32
  - 2561b72a7f9310fb657a89b1ed67b8a928f99f0cfb41e1efaea953655320aeb4
- **Settlement.** Treasure tile 2. Pot 0.02375 (after the 0.00005 per tile
  dig fee). Paid 0.0225125 to the tile-2 receiver.
  - Payout tx: 2e75a1f21cd0aeedea79ba115abbd809d90a69294cb4fbabf983c18a8ea451c4
- **Wallet.** The Pocket's history shows "Zeek round 188: treasure +0.0225125".
- **Prove-a-win.** Proof id a9cce3fda3c035c555468fb3ae306ec7. The engine
  verified it before storing.
  - `/share/win/a9cce3fda3c035c555468fb3ae306ec7` serves OG and Twitter
    `summary_large_image` meta.
  - Its PNG is 1200×675.
  - A browser opening the share page is sent on to
    `/win/a9cce3fda3c035c555468fb3ae306ec7`. That page re-verifies the proof
    in the browser and shows "Verified".
