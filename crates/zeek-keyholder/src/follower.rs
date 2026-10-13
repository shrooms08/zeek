//! The node's own chain follower: the engine's ingestion and round processor
//! over this node's database and lightwalletd endpoints, so that every
//! settlement the node later verifies against was computed by itself.

use std::time::{SystemTime, UNIX_EPOCH};

use zeek_engine::beacon::RelayFetcher;
use zeek_engine::chain::{self, LightwalletdSource};
use zeek_engine::config::GameConfig;
use zeek_engine::db::Db;
use zeek_engine::decrypt::DigScanner;
use zeek_engine::processor::Processor;

use crate::error::KeyholderError;

/// Follows the chain and settles rounds locally.
pub struct Follower {
    config: GameConfig,
    endpoints: Vec<String>,
    source: Option<(String, LightwalletdSource)>,
    scanner: DigScanner,
    processor: Processor,
    fetcher: RelayFetcher,
}

impl Follower {
    /// Builds a follower over `endpoints` (tried in order) and `relays`.
    pub fn new(
        config: GameConfig,
        endpoints: Vec<String>,
        relays: Vec<String>,
    ) -> Result<Self, KeyholderError> {
        let scanner = DigScanner::new(config.incoming_viewing_key()?);
        let processor = Processor::new(config.clone());
        let fetcher = RelayFetcher::new(relays)?;
        Ok(Follower {
            config,
            endpoints,
            source: None,
            scanner,
            processor,
            fetcher,
        })
    }

    /// The processor (for settlement lookups and board views).
    pub fn processor(&self) -> &Processor {
        &self.processor
    }

    /// The endpoint currently in use.
    pub fn endpoint(&self) -> Option<&str> {
        self.source.as_ref().map(|(url, _)| url.as_str())
    }

    async fn connect(&mut self) -> Result<(), KeyholderError> {
        if self.source.is_some() {
            return Ok(());
        }
        let mut errors = Vec::new();
        for url in &self.endpoints {
            match LightwalletdSource::connect(url).await {
                Ok(source) => {
                    tracing::info!(url, "following the chain through this lightwalletd");
                    self.source = Some((url.clone(), source));
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(url, error = %e, "lightwalletd endpoint unavailable");
                    errors.push(format!("{url}: {e}"));
                }
            }
        }
        Err(KeyholderError::Peer(format!(
            "no lightwalletd endpoint answered: {}",
            errors.join("; ")
        )))
    }

    /// One ingestion + settlement pass. A failing endpoint is dropped so the
    /// next pass tries the list again.
    pub async fn pass(&mut self, db: &mut Db) -> Result<(), KeyholderError> {
        self.connect().await?;
        let Some((url, source)) = self.source.as_mut() else {
            return Ok(());
        };
        let result = chain::ingest(source, db, &self.scanner, &self.config).await;
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
                tracing::warn!(url = %url, error = %e, "ingestion failed; endpoint will be re-selected");
                self.source = None;
                return Err(e.into());
            }
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let events = self.processor.process(db, &mut self.fetcher, now).await?;
        for event in events {
            tracing::info!(?event, "round event");
        }
        Ok(())
    }
}
