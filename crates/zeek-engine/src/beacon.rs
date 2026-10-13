//! drand beacon fetching from several independent relays.
//!
//! Relays (docs.drand.love/developer/http-api): `api.drand.sh`,
//! `api2.drand.sh`, `api3.drand.sh` and `drand.cloudflare.com`. All of them
//! serve the v1 route `/{chain_hash}/public/{round}`, which also pins the
//! chain hash into the request, so that route is used uniformly. Relays are
//! not trusted: the first response whose signature verifies under the chain's
//! public key (zeek-core) wins, and every other outcome is reported.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use zeek_core::beacon::{self, Beacon, Randomness};
use zeek_core::params::DrandChain;

use crate::error::EngineError;

/// A verified beacon and the relay it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedBeacon {
    pub beacon: Beacon,
    pub randomness: Randomness,
    pub relay: String,
}

/// Source of beacons.
#[async_trait]
pub trait BeaconFetcher: Send {
    /// Fetches and verifies drand round `round` of `chain`.
    async fn fetch(&mut self, chain: &DrandChain, round: u64)
        -> Result<FetchedBeacon, EngineError>;
}

/// v1 beacon response (`randomness` and `previous_signature` are ignored:
/// randomness is recomputed from the signature).
#[derive(Deserialize)]
struct V1Beacon {
    round: u64,
    signature: String,
}

/// HTTP relay fetcher.
pub struct RelayFetcher {
    client: reqwest::Client,
    relays: Vec<String>,
}

impl RelayFetcher {
    /// Builds a fetcher over `relays` (base URLs).
    pub fn new(relays: Vec<String>) -> Result<Self, EngineError> {
        if relays.is_empty() {
            return Err(EngineError::Config("no drand relays configured".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("zeek-engine/0.1")
            .build()
            .map_err(|e| EngineError::Relay(e.to_string()))?;
        Ok(RelayFetcher { client, relays })
    }

    async fn fetch_one(
        &self,
        relay: &str,
        chain: &DrandChain,
        round: u64,
    ) -> Result<FetchedBeacon, String> {
        let url = format!(
            "{}/{}/public/{}",
            relay.trim_end_matches('/'),
            chain.chain_hash,
            round
        );
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let body: V1Beacon = response.json().await.map_err(|e| e.to_string())?;
        if body.round != round {
            return Err(format!("relay returned round {}", body.round));
        }
        let beacon = Beacon {
            round: body.round,
            signature: body.signature,
        };
        let randomness = beacon::verify_beacon(chain, &beacon).map_err(|e| e.to_string())?;
        Ok(FetchedBeacon {
            beacon,
            randomness,
            relay: relay.to_owned(),
        })
    }
}

#[async_trait]
impl BeaconFetcher for RelayFetcher {
    async fn fetch(
        &mut self,
        chain: &DrandChain,
        round: u64,
    ) -> Result<FetchedBeacon, EngineError> {
        let mut attempts = Vec::new();
        for relay in &self.relays {
            match self.fetch_one(relay, chain, round).await {
                Ok(fetched) => {
                    tracing::info!(round, relay, "verified drand beacon");
                    return Ok(fetched);
                }
                Err(e) => {
                    tracing::warn!(round, relay, error = %e, "relay did not yield a verifiable beacon");
                    attempts.push(format!("{relay}: {e}"));
                }
            }
        }
        Err(EngineError::BeaconUnavailable {
            round,
            attempts: attempts.join("; "),
        })
    }
}

/// Exponential backoff schedule for beacon retries: `base · 2^attempt`, capped.
#[derive(Clone, Debug, Default)]
pub struct Backoff {
    attempts: HashMap<u64, (u32, u64)>,
}

impl Backoff {
    /// Base delay in seconds.
    pub const BASE_SECS: u64 = 10;
    /// Longest delay in seconds.
    pub const MAX_SECS: u64 = 300;

    /// Whether round `round` may be retried at `now`.
    pub fn ready(&self, round: u64, now: u64) -> bool {
        self.attempts
            .get(&round)
            .is_none_or(|(_, next_try)| now >= *next_try)
    }

    /// Records a failure at `now` and schedules the next try.
    pub fn failed(&mut self, round: u64, now: u64) -> u64 {
        let entry = self.attempts.entry(round).or_insert((0, now));
        let delay = (Self::BASE_SECS << entry.0.min(5)).min(Self::MAX_SECS);
        entry.0 += 1;
        entry.1 = now.saturating_add(delay);
        delay
    }

    /// Forgets a round once it succeeded.
    pub fn succeeded(&mut self, round: u64) {
        self.attempts.remove(&round);
    }
}

/// Beacons from a fixed table (tests).
#[derive(Clone, Debug, Default)]
pub struct FixedBeacons {
    pub beacons: HashMap<u64, Beacon>,
}

#[async_trait]
impl BeaconFetcher for FixedBeacons {
    async fn fetch(
        &mut self,
        chain: &DrandChain,
        round: u64,
    ) -> Result<FetchedBeacon, EngineError> {
        let beacon =
            self.beacons
                .get(&round)
                .cloned()
                .ok_or_else(|| EngineError::BeaconUnavailable {
                    round,
                    attempts: "not in the fixed table".into(),
                })?;
        let randomness = beacon::verify_beacon(chain, &beacon)?;
        Ok(FetchedBeacon {
            beacon,
            randomness,
            relay: "fixed".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let mut b = Backoff::default();
        assert!(b.ready(7, 0));
        assert_eq!(b.failed(7, 0), 10);
        assert!(!b.ready(7, 9));
        assert!(b.ready(7, 10));
        assert_eq!(b.failed(7, 10), 20);
        assert_eq!(b.failed(7, 30), 40);
        assert_eq!(b.failed(7, 70), 80);
        assert_eq!(b.failed(7, 150), 160);
        assert_eq!(b.failed(7, 310), 300);
        assert_eq!(b.failed(7, 610), 300);
        b.succeeded(7);
        assert!(b.ready(7, 0));
    }
}
