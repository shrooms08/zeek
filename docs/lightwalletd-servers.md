# lightwalletd servers

Every Zeek process that reads the chain uses `zeek_vault::pool`:

- the engine: chain loop, relay, mempool watcher and payout coordinator;
- the keyholders: each one prefers a different server;
- `zeek-dig` and `zeek-vault`.

`ZEEK_LIGHTWALLETD_URL` takes one or more servers, comma-separated. When it
is unset, the lists below are used. Keyholder configs list theirs under
`lightwalletd`.

## How the pool behaves

- **Preference.** Servers are tried in order. Keyholder *n* starts at server
  *n − 1*, so with several healthy servers the three nodes read from
  different upstreams.
- **Timeouts.**
  - Unary calls (tip, block, transaction, tree state, broadcast) have a
    45 s deadline.
  - Block streams end if no block arrives for 60 s.
  - A connection attempt is abandoned after 45 s.
- **Failover.** A server that fails a call is skipped for 30 s and the call
  moves to the next server. A request the server rejected on its merits is
  not retried elsewhere: an unknown transaction, a bad range, or a broadcast
  the node refused.
- **Health.** The engine asks every server for `GetLightdInfo` once a
  minute. The status page shows each server's tip, latency, last error and
  how often it was outvoted.
- **Consistency.** The last block of every ingested batch is checked
  against up to two other servers.
  - When they disagree, the strict majority wins. The disagreement is
    logged, the dissenting server is cooled down, and a batch from an
    outvoted server is fetched again from a majority server.
  - Without a strict majority (for example one against one) nothing is
    trusted. The pass fails and is retried.
  - A server that does not have the block yet does not vote.
  - Earlier blocks in a batch are tied to the confirmed one through their
    previous-hash links.
- **Single source.** With only one reachable server the check cannot run.
  The block rests on that server, and the status page says so.

## Mainnet (verified 2026-10-15)

Each server answered `GetLightdInfo` with chain `main`, the same height and
consensus branch `37a5165b` (NU6.3). `crates/zeek-vault/tests/network.rs`
`mainnet_pool_servers_answer_and_agree_on_a_block` re-checks this (run it
with `--ignored`). On 2026-10-15:

- All eight servers were healthy.
- Block 3,518,584 was confirmed by three servers.
- A forged hash for the same block was outvoted.

| Server | Backend reported |
| --- | --- |
| `https://zec.rocks:443` | Zebra 6.3.0 |
| `https://eu.zec.rocks:443` | Zebra 6.3.0 |
| `https://na.zec.rocks:443` | Zebra 6.3.0 |
| `https://ap.zec.rocks:443` | Zebra 6.3.0 |
| `https://zec.0xrpc.io:443` | Zaino 0.10.1 |
| `https://zec-node.cakewallet.com:443` | Zakura 1.6.0 |
| `https://zcash.mysideoftheweb.com:9067` | Zebra 6.3.0 |
| `https://lwd.zcashexplorer.app:9067` | Zebra 6.3.0 |

Sources:

- The live server list at <https://hosh.zec.rocks/zec>.
- Each server's own `GetLightdInfo` response.

## Testnet (verified 2026-10-15)

`https://testnet.zec.rocks:443` (Zebra 7.0.0-rc.0, branch `77190ad9`, NU7)
is the only public testnet server that answered.

| Server | Result |
| --- | --- |
| `lightwalletd.testnet.electriccoin.co:443` | TLS handshake failed |
| `lightwalletd.testnet.electriccoin.co:9067` | Port closed |
| `zaino.testnet.unsafe.zec.rocks` | HTTP 503 |
| `testnet.lightwalletd.com` | Does not resolve |

On testnet the pool therefore runs in single-source mode. Failover and the
majority check start working as soon as a second testnet server is added to
`ZEEK_LIGHTWALLETD_URL`.
