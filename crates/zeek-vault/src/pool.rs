//! A pool of `lightwalletd` servers with health checks, per-request
//! timeouts, failover and a block-hash consistency check.
//!
//! Every Zeek process that reads the chain (engine, keyholders, relay,
//! `zeek-dig`) goes through a [`Pool`]:
//!
//! * Servers are tried in a preference order. Each keyholder rotates the
//!   order by its identifier, so with several healthy servers the nodes read
//!   from different upstreams.
//! * A server that fails a call is skipped for [`COOL_DOWN`] and the call is
//!   retried on the next one.
//! * [`Pool::check_block`] asks other servers for the hash of a block one
//!   server returned. When servers disagree the strict majority wins, the
//!   disagreement is logged, and a dissenting server is cooled down. When no
//!   strict majority exists (for example one against one) nothing is
//!   trusted: the call fails and is retried later.
//!
//! The pool is shared (`Arc<Pool>`): its state sits behind a `std` mutex
//! that is never held across an `.await`, and gRPC clients are cheap
//! channel clones.

use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zcash_client_backend::proto::service::BlockId;
use zeek_core::params::Network;

use crate::error::VaultError;
use crate::lightwalletd::{self, Client};

/// How long a failed server is skipped before it is tried again.
pub const COOL_DOWN: Duration = Duration::from_secs(30);

/// Deadline for one unary call (tip, block, transaction, tree state).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// Other servers asked to confirm a block hash (with the first server this
/// makes a majority of three).
const WITNESSES: usize = 2;

/// Public `lightwalletd` servers verified on 2026-10-15 with `GetLightdInfo`
/// (chain name, height, consensus branch); list sources in
/// `docs/lightwalletd-servers.md`.
pub const MAINNET_SERVERS: &[&str] = &[
    "https://zec.rocks:443",
    "https://eu.zec.rocks:443",
    "https://na.zec.rocks:443",
    "https://ap.zec.rocks:443",
    "https://zec.0xrpc.io:443",
    "https://zec-node.cakewallet.com:443",
    "https://zcash.mysideoftheweb.com:9067",
    "https://lwd.zcashexplorer.app:9067",
];

/// The only public testnet `lightwalletd` that answered on 2026-10-15.
pub const TESTNET_SERVERS: &[&str] = &["https://testnet.zec.rocks:443"];

/// Default servers for `network`.
pub fn default_servers(network: Network) -> Vec<String> {
    let list = match network {
        Network::Main => MAINNET_SERVERS,
        Network::Test => TESTNET_SERVERS,
    };
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// Parses a comma-separated server list.
pub fn parse_servers(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// One server's health, as reported on status pages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub url: String,
    /// Answered its last call.
    pub healthy: bool,
    /// Chain tip it reported at the last health check.
    pub tip: Option<u32>,
    /// Unix seconds of the last successful call.
    pub last_ok: Option<u64>,
    /// The last error (no request data is included).
    pub last_error: Option<String>,
    /// Round-trip time of the last health check.
    pub latency_ms: Option<u64>,
    /// Times it was outvoted on a block hash.
    pub outvoted: u64,
}

/// How a block hash was confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmation {
    /// Every server that answered agrees (`servers` includes the first).
    Agreed { servers: usize },
    /// No other server could be asked; the hash rests on one server.
    SingleSource,
    /// The majority disagrees with the server that returned the block: use
    /// `majority` (a server index that holds the majority hash) instead.
    Outvoted { majority: usize, hash: Vec<u8> },
}

struct Server {
    url: String,
    client: Option<Client>,
    cool_until: Option<Instant>,
    status: ServerStatus,
}

/// A shared, failover-capable set of `lightwalletd` servers for one network.
pub struct Pool {
    network: Network,
    /// Indices into `servers`, most preferred first.
    order: Vec<usize>,
    servers: Mutex<Vec<Server>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Shortens an error for status output (servers can echo long messages).
fn brief(e: &VaultError) -> String {
    let text = e.to_string();
    match text.char_indices().nth(200) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

/// The strict-majority value among `votes` (`None` without one).
pub fn majority<T: PartialEq + Clone>(votes: &[T]) -> Option<T> {
    votes
        .iter()
        .find(|v| votes.iter().filter(|w| w == v).count() * 2 > votes.len())
        .cloned()
}

impl Pool {
    /// A pool over `urls` for `network`. `prefer` rotates the preference
    /// order (keyholder `n` passes `n - 1`).
    pub fn new(network: Network, urls: Vec<String>, prefer: usize) -> Result<Self, VaultError> {
        if urls.is_empty() {
            return Err(VaultError::Config(
                "no lightwalletd servers configured".into(),
            ));
        }
        let n = urls.len();
        let order = (0..n).map(|i| (i + prefer) % n).collect();
        let servers = urls
            .into_iter()
            .map(|url| Server {
                status: ServerStatus {
                    url: url.clone(),
                    healthy: false,
                    tip: None,
                    last_ok: None,
                    last_error: None,
                    latency_ms: None,
                    outvoted: 0,
                },
                url,
                client: None,
                cool_until: None,
            })
            .collect();
        Ok(Pool {
            network,
            order,
            servers: Mutex::new(servers),
        })
    }

    /// The network this pool serves.
    pub fn network(&self) -> Network {
        self.network
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Server>> {
        self.servers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every server's health.
    pub fn status(&self) -> Vec<ServerStatus> {
        self.lock().iter().map(|s| s.status.clone()).collect()
    }

    /// The URL of server `index`.
    pub fn url(&self, index: usize) -> String {
        self.lock()
            .get(index)
            .map(|s| s.url.clone())
            .unwrap_or_default()
    }

    /// Servers in the order to try now: preferred first, cooled-down ones
    /// last (they are still tried when nothing else answers).
    fn attempt_order(&self) -> Vec<usize> {
        let now = Instant::now();
        let servers = self.lock();
        let (ready, cooling): (Vec<usize>, Vec<usize>) = self
            .order
            .iter()
            .copied()
            .partition(|&i| servers[i].cool_until.is_none_or(|t| t <= now));
        ready.into_iter().chain(cooling).collect()
    }

    /// A connected client for server `index`.
    async fn client_for(&self, index: usize) -> Result<Client, VaultError> {
        let (url, existing) = {
            let servers = self.lock();
            let s = &servers[index];
            (s.url.clone(), s.client.clone())
        };
        if let Some(client) = existing {
            return Ok(client);
        }
        let client = tokio::time::timeout(
            REQUEST_TIMEOUT,
            lightwalletd::connect_to(&url, self.network),
        )
        .await
        .map_err(|_| VaultError::Timeout(REQUEST_TIMEOUT))??;
        self.lock()[index].client = Some(client.clone());
        Ok(client)
    }

    /// Records a failed call on server `index` and cools it down.
    pub fn failed(&self, index: usize, error: &VaultError) {
        let mut servers = self.lock();
        let Some(s) = servers.get_mut(index) else {
            return;
        };
        tracing::warn!(server = %s.url, error = %brief(error), "lightwalletd call failed; trying the next server");
        s.client = None;
        s.cool_until = Some(Instant::now() + COOL_DOWN);
        s.status.healthy = false;
        s.status.last_error = Some(brief(error));
    }

    fn succeeded(&self, index: usize) {
        let mut servers = self.lock();
        if let Some(s) = servers.get_mut(index) {
            s.cool_until = None;
            s.status.healthy = true;
            s.status.last_ok = Some(now_secs());
        }
    }

    /// A connected client from the most preferred server that connects,
    /// with its index (for long calls such as wallet sync; report a failure
    /// with [`Pool::failed`]).
    pub async fn client(&self) -> Result<(usize, Client), VaultError> {
        let mut errors = Vec::new();
        for index in self.attempt_order() {
            match self.client_for(index).await {
                Ok(client) => return Ok((index, client)),
                Err(e) => {
                    errors.push(format!("{}: {}", self.url(index), brief(&e)));
                    self.failed(index, &e);
                }
            }
        }
        Err(VaultError::NoServer(errors.join("; ")))
    }

    /// A connected client for server `index` (for example the majority
    /// server named by [`Confirmation::Outvoted`]).
    pub async fn client_at(&self, index: usize) -> Result<Client, VaultError> {
        self.client_for(index).await
    }

    /// Runs one unary call with [`REQUEST_TIMEOUT`], failing over through the
    /// servers. Returns the result and the index of the server that gave it.
    pub async fn call<T, F, Fut>(&self, mut f: F) -> Result<(T, usize), VaultError>
    where
        F: FnMut(Client) -> Fut,
        Fut: Future<Output = Result<T, VaultError>>,
    {
        let mut errors = Vec::new();
        for index in self.attempt_order() {
            let attempt = async {
                let client = self.client_for(index).await?;
                tokio::time::timeout(REQUEST_TIMEOUT, f(client))
                    .await
                    .map_err(|_| VaultError::Timeout(REQUEST_TIMEOUT))?
            };
            match attempt.await {
                Ok(value) => {
                    self.succeeded(index);
                    return Ok((value, index));
                }
                Err(e) if is_definitive(&e) => return Err(e),
                Err(e) => {
                    errors.push(format!("{}: {}", self.url(index), brief(&e)));
                    self.failed(index, &e);
                }
            }
        }
        Err(VaultError::NoServer(errors.join("; ")))
    }

    /// Like [`Pool::call`], without the server index.
    pub async fn unary<T, F, Fut>(&self, f: F) -> Result<T, VaultError>
    where
        F: FnMut(Client) -> Fut,
        Fut: Future<Output = Result<T, VaultError>>,
    {
        Ok(self.call(f).await?.0)
    }

    /// Confirms that block `height` has `hash`, as returned by server
    /// `source`, against up to two other servers.
    pub async fn check_block(
        &self,
        source: usize,
        height: u32,
        hash: &[u8],
    ) -> Result<Confirmation, VaultError> {
        let witnesses: Vec<usize> = self
            .attempt_order()
            .into_iter()
            .filter(|&i| i != source)
            .take(WITNESSES)
            .collect();
        let mut votes: Vec<(usize, Vec<u8>)> = vec![(source, hash.to_vec())];
        for index in witnesses {
            let answer = async {
                let mut client = self.client_for(index).await?;
                let block = tokio::time::timeout(
                    REQUEST_TIMEOUT,
                    client.get_block(BlockId {
                        height: u64::from(height),
                        hash: vec![],
                    }),
                )
                .await
                .map_err(|_| VaultError::Timeout(REQUEST_TIMEOUT))??
                .into_inner();
                Ok::<_, VaultError>(block.hash)
            };
            match answer.await {
                Ok(h) => {
                    self.succeeded(index);
                    votes.push((index, h));
                }
                // A lagging server that does not have the block yet, or one
                // that is down, does not vote.
                Err(e) => {
                    tracing::debug!(server = %self.url(index), error = %brief(&e), height, "no hash vote")
                }
            }
        }
        if votes.len() == 1 {
            return Ok(Confirmation::SingleSource);
        }
        if votes.iter().all(|(_, h)| h == hash) {
            return Ok(Confirmation::Agreed {
                servers: votes.len(),
            });
        }
        let hashes: Vec<Vec<u8>> = votes.iter().map(|(_, h)| h.clone()).collect();
        let describe = || {
            votes
                .iter()
                .map(|(i, h)| {
                    let mut display = h.clone();
                    display.reverse();
                    format!("{} says {}", self.url(*i), hex::encode(display))
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let Some(winner) = majority(&hashes) else {
            tracing::error!(height, votes = %describe(), "lightwalletd servers disagree with no majority; not trusting any");
            return Err(VaultError::ServersDisagree {
                height,
                detail: describe(),
            });
        };
        tracing::warn!(height, votes = %describe(), "lightwalletd servers disagree; following the majority");
        let mut majority_server = source;
        {
            let mut servers = self.lock();
            for (i, h) in &votes {
                if *h == winner {
                    majority_server = *i;
                } else if let Some(s) = servers.get_mut(*i) {
                    s.status.outvoted += 1;
                    s.status.healthy = false;
                    s.status.last_error = Some(format!("outvoted on block {height}"));
                    s.cool_until = Some(Instant::now() + COOL_DOWN);
                    s.client = None;
                }
            }
        }
        if winner == hash {
            Ok(Confirmation::Agreed {
                servers: votes.iter().filter(|(_, h)| *h == winner).count(),
            })
        } else {
            Ok(Confirmation::Outvoted {
                majority: majority_server,
                hash: winner,
            })
        }
    }

    /// Asks every server for its chain info and records tip and latency.
    pub async fn probe(&self) {
        let count = self.lock().len();
        for index in 0..count {
            let started = Instant::now();
            let info = async {
                let mut client = self.client_for(index).await?;
                tokio::time::timeout(REQUEST_TIMEOUT, lightwalletd::lightd_info(&mut client))
                    .await
                    .map_err(|_| VaultError::Timeout(REQUEST_TIMEOUT))?
            };
            match info.await {
                Ok(info) => {
                    self.succeeded(index);
                    let mut servers = self.lock();
                    let s = &mut servers[index];
                    s.status.tip = u32::try_from(info.block_height).ok();
                    s.status.latency_ms = u64::try_from(started.elapsed().as_millis()).ok();
                }
                Err(e) => self.failed(index, &e),
            }
        }
    }
}

/// Errors that another server would answer the same way (the request
/// itself is at fault), so failing over would only repeat them.
fn is_definitive(e: &VaultError) -> bool {
    match e {
        VaultError::BroadcastRejected { .. } => true,
        VaultError::Rpc(status) => matches!(
            status.code(),
            tonic::Code::InvalidArgument | tonic::Code::NotFound | tonic::Code::OutOfRange
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majority_needs_more_than_half() {
        assert_eq!(majority(&[1, 1, 2]), Some(1));
        assert_eq!(majority(&[2, 1, 1]), Some(1));
        assert_eq!(majority(&[1, 2]), None, "one against one trusts nobody");
        assert_eq!(majority(&[1, 2, 3]), None);
        assert_eq!(majority(&[7]), Some(7));
        assert_eq!(majority::<u8>(&[]), None);
    }

    #[test]
    fn keyholders_prefer_different_servers() {
        let urls = vec![
            "http://a".to_owned(),
            "http://b".to_owned(),
            "http://c".to_owned(),
        ];
        let firsts: Vec<usize> = (0..3)
            .map(|k| {
                Pool::new(Network::Test, urls.clone(), k)
                    .expect("pool")
                    .attempt_order()[0]
            })
            .collect();
        assert_eq!(firsts, vec![0, 1, 2]);
    }

    #[test]
    fn a_failed_server_moves_to_the_back_until_it_cools_down() {
        let urls = vec!["http://a".to_owned(), "http://b".to_owned()];
        let pool = Pool::new(Network::Test, urls, 0).expect("pool");
        pool.failed(0, &VaultError::Timeout(REQUEST_TIMEOUT));
        assert_eq!(pool.attempt_order(), vec![1, 0]);
        let status = pool.status();
        assert!(!status[0].healthy);
        assert!(status[0]
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("timed out")));
        pool.lock()[0].cool_until = Some(Instant::now());
        assert_eq!(pool.attempt_order(), vec![0, 1]);
    }

    #[tokio::test]
    async fn every_server_down_is_a_clear_error() {
        let urls = vec![
            "http://127.0.0.1:9".to_owned(),
            "http://127.0.0.1:1".to_owned(),
        ];
        let pool = Pool::new(Network::Test, urls, 0).expect("pool");
        let err = pool
            .unary(|mut c| async move { lightwalletd::latest_height(&mut c).await })
            .await
            .expect_err("no server");
        assert!(
            matches!(err, VaultError::NoServer(ref m) if m.contains("127.0.0.1:9") && m.contains("127.0.0.1:1"))
        );
        assert!(pool.status().iter().all(|s| !s.healthy));
    }

    #[test]
    fn an_empty_list_is_refused() {
        assert!(Pool::new(Network::Main, vec![], 0).is_err());
        assert_eq!(
            parse_servers(" https://a:443, ,https://b:443 "),
            vec!["https://a:443", "https://b:443"]
        );
        assert_eq!(
            default_servers(Network::Test),
            vec!["https://testnet.zec.rocks:443"]
        );
    }
}
