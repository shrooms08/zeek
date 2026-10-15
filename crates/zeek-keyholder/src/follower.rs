//! The node's own chain follower: the engine's ingestion and round processor
//! over this node's database and lightwalletd endpoints, so that every
//! settlement the node later verifies against was computed by itself.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use zeek_engine::beacon::RelayFetcher;
use zeek_engine::chain::{self, PoolSource};
use zeek_engine::config::GameConfig;
use zeek_engine::decrypt::DigScanner;
use zeek_engine::processor::Processor;
use zeek_engine::store::Store;

use zeek_vault::pool::Pool;

use crate::error::KeyholderError;

/// Follows the chain and settles rounds locally.
pub struct Follower {
    config: GameConfig,
    source: PoolSource,
    scanner: DigScanner,
    processor: Processor,
    fetcher: RelayFetcher,
}

impl Follower {
    /// Builds a follower over the `lightwalletd` servers `endpoints` (tried
    /// starting at `prefer`, so the keyholders read from different servers
    /// when several are healthy) and the drand `relays`.
    pub fn new(
        config: GameConfig,
        endpoints: Vec<String>,
        prefer: usize,
        relays: Vec<String>,
    ) -> Result<Self, KeyholderError> {
        let scanner = DigScanner::new(config.incoming_viewing_key()?);
        let processor = Processor::new(config.clone());
        let fetcher = RelayFetcher::new(relays)?;
        let pool = Pool::new(config.params.network, endpoints, prefer)
            .map_err(|e| KeyholderError::Peer(e.to_string()))?;
        Ok(Follower {
            config,
            source: PoolSource::new(Arc::new(pool)),
            scanner,
            processor,
            fetcher,
        })
    }

    /// The processor (for settlement lookups and board views).
    pub fn processor(&self) -> &Processor {
        &self.processor
    }

    /// The pool's servers (shared with the health endpoint).
    pub fn pool(&self) -> Arc<Pool> {
        self.source.pool().clone()
    }

    /// One ingestion + settlement pass. A failing endpoint is dropped so the
    /// next pass tries the list again. No connection to `store` is held
    /// across a network call, so signing and health keep reading.
    pub async fn pass(&mut self, store: &Store) -> Result<(), KeyholderError> {
        let result = chain::ingest(&mut self.source, store, &self.scanner, &self.config).await;
        match result {
            Ok(report) => {
                if report.new_blocks > 0 || report.new_notes > 0 || report.reorg_from.is_some() {
                    tracing::info!(
                        tip = ?report.stored_tip,
                        new_blocks = report.new_blocks,
                        new_notes = report.new_notes,
                        reorg_from = ?report.reorg_from,
                        "ingested"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "ingestion failed; the pool fails over on the next pass");
                return Err(e.into());
            }
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let events = self
            .processor
            .process(store, &mut self.fetcher, now)
            .await?;
        for event in events {
            tracing::info!(?event, "round event");
        }
        Ok(())
    }
}
