//! Chain following: compact blocks in, blocks and notes out, with reorg
//! detection.
//!
//! The engine needs, per block, only its hash, previous hash and `nTime`, plus
//! the outputs that decrypt to the vault's external IVK. It therefore follows
//! `lightwalletd`'s compact block stream from the game's genesis height and
//! fetches a full transaction only when a compact output is a hit.
//!
//! Reorg safety: every stored block records its hash and previous hash. When
//! the next block's `prev_hash` does not match the stored tip, the follower
//! walks back until a stored block matches the chain again, forgets every
//! block and note above that fork point, and rescans from there.

use async_trait::async_trait;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::{BlockId, BlockRange, TxFilter};
use zcash_protocol::consensus::{BlockHeight, BranchId, MAIN_NETWORK, TEST_NETWORK};
use zeek_core::params::Network;
use zeek_vault::lightwalletd::{self, Client};

use crate::config::GameConfig;
use crate::db::{BlockRow, Db, NoteRow};
use crate::decrypt::DigScanner;
use crate::error::EngineError;

/// Where compact blocks and transactions come from.
#[async_trait]
pub trait ChainSource: Send {
    /// Current chain tip height.
    async fn latest_height(&mut self) -> Result<u32, EngineError>;
    /// Compact blocks `start..=end` in height order (may return fewer if the
    /// chain is shorter).
    async fn block_range(&mut self, start: u32, end: u32)
        -> Result<Vec<CompactBlock>, EngineError>;
    /// Raw bytes of a mined transaction (txid in internal byte order).
    async fn transaction(&mut self, txid: &[u8; 32]) -> Result<Vec<u8>, EngineError>;
}

/// `lightwalletd` as a [`ChainSource`].
pub struct LightwalletdSource {
    client: Client,
}

impl LightwalletdSource {
    /// Connects to a testnet `lightwalletd` (the vault's connection checks
    /// chain name and consensus branch support).
    pub async fn connect(url: &str) -> Result<Self, EngineError> {
        Ok(LightwalletdSource {
            client: lightwalletd::connect(url).await?,
        })
    }
}

#[async_trait]
impl ChainSource for LightwalletdSource {
    async fn latest_height(&mut self) -> Result<u32, EngineError> {
        Ok(lightwalletd::latest_height(&mut self.client).await?.into())
    }

    async fn block_range(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<Vec<CompactBlock>, EngineError> {
        let block_id = |h: u32| BlockId {
            height: u64::from(h),
            hash: vec![],
        };
        let mut stream = self
            .client
            .get_block_range(BlockRange {
                start: Some(block_id(start)),
                end: Some(block_id(end)),
                pool_types: vec![],
            })
            .await
            .map_err(zeek_vault::VaultError::from)?
            .into_inner();
        let mut blocks = Vec::new();
        while let Some(block) = stream
            .message()
            .await
            .map_err(zeek_vault::VaultError::from)?
        {
            blocks.push(block);
        }
        Ok(blocks)
    }

    async fn transaction(&mut self, txid: &[u8; 32]) -> Result<Vec<u8>, EngineError> {
        let raw = self
            .client
            .get_transaction(TxFilter {
                block: None,
                index: 0,
                hash: txid.to_vec(),
            })
            .await
            .map_err(zeek_vault::VaultError::from)?
            .into_inner();
        Ok(raw.data)
    }
}

/// What one ingestion pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IngestReport {
    /// Chain tip the source reported.
    pub chain_tip: u32,
    /// Stored tip after the pass.
    pub stored_tip: Option<u32>,
    /// Blocks stored in this pass.
    pub new_blocks: u32,
    /// Notes stored in this pass.
    pub new_notes: usize,
    /// First height that was rolled back, if a reorg was handled.
    pub reorg_from: Option<u32>,
    /// Settlements discarded by the rollback (should be empty; logged).
    pub rolled_back_settlements: Vec<u32>,
}

/// Blocks fetched per request.
pub const BATCH_SIZE: u32 = 200;

/// Consensus branch active at `height` on the game's network.
pub fn branch_for(network: Network, height: u32) -> BranchId {
    let h = BlockHeight::from_u32(height);
    match network {
        Network::Main => BranchId::for_height(&MAIN_NETWORK, h),
        Network::Test => BranchId::for_height(&TEST_NETWORK, h),
    }
}

/// Follows the chain from the stored tip (or the game's genesis) to the
/// source's tip, handling one reorg if encountered.
pub async fn ingest(
    source: &mut dyn ChainSource,
    db: &mut Db,
    scanner: &DigScanner,
    config: &GameConfig,
) -> Result<IngestReport, EngineError> {
    let chain_tip = source.latest_height().await?;
    db.set_chain_tip(chain_tip)?;
    // Before genesis no block is stored; keep the tip's time so the board can
    // estimate when round 0 starts.
    if chain_tip < config.genesis_height {
        if let Some(block) = source
            .block_range(chain_tip, chain_tip)
            .await?
            .into_iter()
            .next()
        {
            db.set_chain_tip_time(u64::from(block.time))?;
        }
    }
    let mut report = IngestReport {
        chain_tip,
        stored_tip: db.tip()?.map(|b| b.height),
        ..IngestReport::default()
    };

    let mut next = match db.tip()? {
        Some(tip) => tip.height + 1,
        None => config.genesis_height,
    };
    while next <= chain_tip {
        let end = next.saturating_add(BATCH_SIZE - 1).min(chain_tip);
        let blocks = source.block_range(next, end).await?;
        if blocks.is_empty() {
            break;
        }
        for block in blocks {
            let height = u32::try_from(block.height).map_err(|_| EngineError::UnexpectedBlock {
                expected: next,
                got: block.height,
            })?;
            if height != next {
                return Err(EngineError::UnexpectedBlock {
                    expected: next,
                    got: block.height,
                });
            }
            if let Some(fork) = detect_reorg(source, db, config, &block).await? {
                tracing::warn!(fork, height, "reorg detected; rolling back");
                let discarded = db.rollback_to(fork)?;
                if !discarded.is_empty() {
                    tracing::error!(?discarded, "rollback discarded settled rounds");
                }
                report.reorg_from = Some(fork);
                report.rolled_back_settlements = discarded;
                next = fork;
                break;
            }
            report.new_notes += store_block(source, db, scanner, config, &block).await?;
            report.new_blocks += 1;
            next = height + 1;
        }
    }
    report.stored_tip = db.tip()?.map(|b| b.height);
    Ok(report)
}

/// Returns the first height to forget if `block` does not extend the stored
/// chain.
async fn detect_reorg(
    source: &mut dyn ChainSource,
    db: &Db,
    config: &GameConfig,
    block: &CompactBlock,
) -> Result<Option<u32>, EngineError> {
    let height = block.height as u32;
    let Some(parent) = db.block(height.wrapping_sub(1))? else {
        return Ok(None);
    };
    if parent.hash == block.prev_hash {
        return Ok(None);
    }
    // Walk back until a stored block matches the chain.
    let mut candidate = parent.height;
    loop {
        let Some(stored) = db.block(candidate)? else {
            return Err(EngineError::ReorgTooDeep(candidate));
        };
        let fetched = source
            .block_range(candidate, candidate)
            .await?
            .into_iter()
            .next()
            .ok_or(EngineError::ReorgTooDeep(candidate))?;
        if fetched.hash == stored.hash {
            return Ok(Some(candidate + 1));
        }
        if candidate <= config.genesis_height {
            return Err(EngineError::ReorgTooDeep(candidate));
        }
        candidate -= 1;
    }
}

/// Stores one block and the vault's notes in it; returns how many notes.
async fn store_block(
    source: &mut dyn ChainSource,
    db: &mut Db,
    scanner: &DigScanner,
    config: &GameConfig,
    block: &CompactBlock,
) -> Result<usize, EngineError> {
    let height = block.height as u32;
    let malformed = |reason: String| EngineError::MalformedBlock { height, reason };
    if block.hash.len() != 32 || block.prev_hash.len() != 32 {
        return Err(malformed("hash fields must be 32 bytes".into()));
    }
    let branch = branch_for(config.params.network, height);
    let mut stored = 0usize;
    for tx in &block.vtx {
        let hits = scanner.scan_compact_tx(height, tx)?;
        if hits.is_empty() {
            continue;
        }
        let txid = <[u8; 32]>::try_from(tx.txid.as_slice())
            .map_err(|_| malformed("txid is not 32 bytes".into()))?;
        let mut display = txid;
        display.reverse();
        let txid_hex = hex::encode(display);
        let raw = source.transaction(&txid).await?;
        for note in scanner.decrypt_full(&txid_hex, &raw, branch, &hits)? {
            let inserted = db.insert_note(&NoteRow {
                txid,
                action_index: note.action_index,
                pool: note.pool,
                height,
                block_time: u64::from(block.time),
                value: note.value,
                memo: note.memo.to_vec(),
            })?;
            if inserted {
                stored += 1;
                tracing::info!(
                    height,
                    txid = %txid_hex,
                    action = note.action_index,
                    pool = ?note.pool,
                    value = note.value,
                    "received note at the vault address"
                );
            }
        }
    }
    db.insert_block(&BlockRow {
        height,
        hash: block.hash.clone(),
        prev_hash: block.prev_hash.clone(),
        time: u64::from(block.time),
    })?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Pool;
    use crate::synthetic::{SyntheticChain, TestVault};

    #[tokio::test]
    async fn follows_the_chain_and_records_vault_notes() {
        let vault = TestVault::new(10);
        let config = vault.config(1_000);
        let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
        let mut chain = SyntheticChain::new(&vault, 990, 1_700_000_000);
        chain.advance(12); // 990..=1001
        let memo = vault.dig_memo(0, 1, 1);
        chain.push_block(vec![chain.payment(200_000, Pool::Ironwood, memo.encode())]); // 1002
        chain.advance(3);
        let mut db = Db::open_in_memory().expect("db");

        let report = ingest(&mut chain, &mut db, &scanner, &config)
            .await
            .expect("ingest");
        assert_eq!(report.new_notes, 1);
        assert_eq!(report.stored_tip, Some(1_005));
        assert_eq!(
            db.block(1_000).expect("block").map(|b| b.height),
            Some(1_000)
        );
        assert!(
            db.block(999).expect("block").is_none(),
            "nothing below genesis"
        );
        let notes = db.notes_in_range(1_000, 1_024).expect("notes");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].height, 1_002);
        assert_eq!(notes[0].value, 200_000);

        // A second pass with nothing new is a no-op.
        let again = ingest(&mut chain, &mut db, &scanner, &config)
            .await
            .expect("ingest");
        assert_eq!(again.new_blocks, 0);
        assert_eq!(again.new_notes, 0);
    }

    #[tokio::test]
    async fn reorg_rolls_back_and_rescans() {
        let vault = TestVault::new(11);
        let config = vault.config(100);
        let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
        let mut chain = SyntheticChain::new(&vault, 100, 1_700_000_000);
        chain.advance(5); // 100..=104
        chain.push_block(vec![chain.payment(300_000, Pool::Ironwood, [0xF6; 512])]); // 105
        chain.advance(2); // 106, 107
        let mut db = Db::open_in_memory().expect("db");
        ingest(&mut chain, &mut db, &scanner, &config)
            .await
            .expect("ingest");
        assert_eq!(db.note_count().expect("count"), 1);
        let before = db.dump().expect("dump");

        // The chain forks at 105: the note's block is replaced by an empty one
        // and a different note lands at 106.
        chain.truncate(105);
        chain.advance(1); // new 105
        chain.push_block(vec![chain.payment(400_000, Pool::Ironwood, [0xF6; 512])]); // new 106
        chain.advance(3); // 107..=109
        let report = ingest(&mut chain, &mut db, &scanner, &config)
            .await
            .expect("ingest");
        assert_eq!(report.reorg_from, Some(105));
        assert!(report.rolled_back_settlements.is_empty());
        assert_eq!(report.stored_tip, Some(109));
        let notes = db.notes_in_range(100, 200).expect("notes");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].height, 106);
        assert_eq!(notes[0].value, 400_000);
        assert_ne!(db.dump().expect("dump"), before);
        assert_eq!(
            db.block(105).expect("b").map(|b| b.hash),
            chain.block(105).map(|b| b.hash.clone())
        );
    }
}
