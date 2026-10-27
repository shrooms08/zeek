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

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::{BlockId, BlockRange, RawTransaction, TxFilter};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, MAIN_NETWORK, TEST_NETWORK};
use zeek_core::params::Network;
use zeek_vault::lightwalletd::{self, Client};
use zeek_vault::pool::{Confirmation, Pool};

use crate::config::GameConfig;
use crate::db::{BlockRow, NoteRow};
use crate::decrypt::DigScanner;
use crate::error::EngineError;
use crate::store::Store;

/// Longest wait for the next block of a block-range stream.
pub const STREAM_STALL: Duration = Duration::from_secs(60);

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
    /// The height the source reports for a transaction (0 when unknown).
    /// Not proof of inclusion: callers confirm it in the block's compact
    /// transactions.
    async fn transaction_height(&mut self, txid: &[u8; 32]) -> Result<u32, EngineError>;
}

/// The height a transaction is mined at, confirmed by finding its txid in
/// the compact block at the height the source reports; `None` while it is
/// not mined (or the source does not know it).
pub async fn confirmed_height(
    source: &mut dyn ChainSource,
    txid: &[u8; 32],
) -> Result<Option<u32>, EngineError> {
    let height = source.transaction_height(txid).await?;
    if height == 0 {
        return Ok(None);
    }
    let blocks = source.block_range(height, height).await?;
    let included = blocks
        .iter()
        .any(|b| b.height == u64::from(height) && b.vtx.iter().any(|t| t.txid == txid[..]));
    Ok(included.then_some(height))
}

/// A [`Pool`] of `lightwalletd` servers as a [`ChainSource`]: calls fail
/// over between servers, and the last block of every batch is confirmed by
/// the other servers (the majority wins; a tie is never trusted).
pub struct PoolSource {
    pool: Arc<Pool>,
}

impl PoolSource {
    /// Follows the chain through `pool`.
    pub fn new(pool: Arc<Pool>) -> Self {
        PoolSource { pool }
    }

    /// The pool (for status reporting).
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
}

/// Streams blocks `start..=end` from one server.
async fn stream_blocks(
    mut client: Client,
    start: u32,
    end: u32,
) -> Result<Vec<CompactBlock>, zeek_vault::VaultError> {
    let block_id = |h: u32| BlockId {
        height: u64::from(h),
        hash: vec![],
    };
    let mut stream = client
        .get_block_range(BlockRange {
            start: Some(block_id(start)),
            end: Some(block_id(end)),
            pool_types: vec![],
        })
        .await?
        .into_inner();
    let mut blocks = Vec::new();
    // The request timeout ends at the response headers; a connection that
    // dies mid-stream would otherwise hang the pass.
    while let Some(block) = tokio::time::timeout(STREAM_STALL, stream.message())
        .await
        .map_err(|_| {
            zeek_vault::VaultError::BadResponse(format!(
                "lightwalletd block stream stalled for {STREAM_STALL:?}"
            ))
        })??
    {
        blocks.push(block);
    }
    Ok(blocks)
}

/// Fails unless `raw` (as a server returned it) hashes to `txid`.
fn served_as(
    network: Network,
    txid: &[u8; 32],
    raw: &RawTransaction,
) -> Result<(), zeek_vault::VaultError> {
    let height = u32::try_from(raw.height).unwrap_or(0);
    let served = Transaction::read(&raw.data[..], branch_for(network, height))
        .map_err(|e| zeek_vault::VaultError::BadResponse(format!("transaction: {e}")))?
        .txid();
    if served.as_ref() != txid {
        return Err(zeek_vault::VaultError::BadResponse(
            "the transaction served does not hash to the txid asked for".into(),
        ));
    }
    Ok(())
}

/// Fetches transaction `txid` from one server, and fails (so the pool tries
/// the next server) unless the bytes served hash to `txid`.
async fn get_transaction(
    mut client: Client,
    network: Network,
    txid: [u8; 32],
) -> Result<RawTransaction, zeek_vault::VaultError> {
    let raw = client
        .get_transaction(TxFilter {
            block: None,
            index: 0,
            hash: txid.to_vec(),
        })
        .await?
        .into_inner();
    served_as(network, &txid, &raw)?;
    Ok(raw)
}

#[async_trait]
impl ChainSource for PoolSource {
    async fn latest_height(&mut self) -> Result<u32, EngineError> {
        Ok(self
            .pool
            .unary(|mut c| async move { lightwalletd::latest_height(&mut c).await })
            .await?
            .into())
    }

    async fn block_range(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<Vec<CompactBlock>, EngineError> {
        // Streams are long, so they run without the unary deadline (the
        // stall timeout bounds them); each server is tried at most once.
        let mut errors = Vec::new();
        let mut source = None;
        for _ in 0..self.pool.status().len() {
            let (index, client) = match source.take() {
                Some(chosen) => chosen,
                None => self.pool.client().await?,
            };
            let blocks = match stream_blocks(client, start, end).await {
                Ok(blocks) => blocks,
                Err(e) => {
                    errors.push(e.to_string());
                    self.pool.failed(index, &e);
                    continue;
                }
            };
            let Some(last) = blocks.last() else {
                return Ok(blocks);
            };
            let height = u32::try_from(last.height).map_err(|_| EngineError::UnexpectedBlock {
                expected: end,
                got: last.height,
            })?;
            match self.pool.check_block(index, height, &last.hash).await? {
                Confirmation::Agreed { .. } | Confirmation::SingleSource => return Ok(blocks),
                Confirmation::Outvoted { majority, .. } => {
                    // Refetch the batch from a server that holds the
                    // majority's chain.
                    let client = self.pool.client_at(majority).await?;
                    source = Some((majority, client));
                }
            }
        }
        Err(zeek_vault::VaultError::NoServer(errors.join("; ")).into())
    }

    async fn transaction(&mut self, txid: &[u8; 32]) -> Result<Vec<u8>, EngineError> {
        let (txid, network) = (*txid, self.pool.network());
        Ok(self
            .pool
            .unary(|c| get_transaction(c, network, txid))
            .await?
            .data)
    }

    async fn transaction_height(&mut self, txid: &[u8; 32]) -> Result<u32, EngineError> {
        let (txid, network) = (*txid, self.pool.network());
        let raw = self
            .pool
            .unary(|c| get_transaction(c, network, txid))
            .await?;
        Ok(u32::try_from(raw.height).unwrap_or(0))
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
    /// Rounds with no payout whose inputs the reorg reached: recomputed from
    /// the new chain and checked (empty for an ordinary reorg of the tip).
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
///
/// Every network call runs with no database connection held; each block is
/// then written (its notes and its header) in one short transaction.
pub async fn ingest(
    source: &mut dyn ChainSource,
    store: &Store,
    scanner: &DigScanner,
    config: &GameConfig,
) -> Result<IngestReport, EngineError> {
    let chain_tip = source.latest_height().await?;
    store.write(|db| db.set_chain_tip(chain_tip))?;
    backfill_commitments(source, store, config).await?;
    // Before genesis no block is stored; keep the tip's time so the board can
    // estimate when round 0 starts.
    if chain_tip < config.genesis_height {
        if let Some(block) = source
            .block_range(chain_tip, chain_tip)
            .await?
            .into_iter()
            .next()
        {
            store.write(|db| db.set_chain_tip_time(u64::from(block.time)))?;
        }
    }
    let stored_tip = store.read(|db| db.tip())?.map(|b| b.height);
    let mut report = IngestReport {
        chain_tip,
        stored_tip,
        ..IngestReport::default()
    };

    let mut next = match stored_tip {
        Some(tip) => tip + 1,
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
            if let Some(fork) = detect_reorg(source, store, config, &block).await? {
                tracing::warn!(fork, height, "reorg detected; rolling back");
                let params = &config.params;
                let last_read = |round: u32| -> Result<u32, EngineError> {
                    let window = zeek_core::rounds::round_window(params, round)?;
                    window
                        .end
                        .checked_add(params.mtp_window - 1)
                        .ok_or(EngineError::Overflow("last block read"))
                };
                let rollback = store.write(|db| db.rollback_to(fork, last_read))?;
                if !rollback.kept_paying.is_empty() {
                    tracing::error!(
                        rounds = ?rollback.kept_paying,
                        fork,
                        "a reorg reached the blocks these paid rounds were settled from; their settlements and payouts are kept as they are"
                    );
                }
                if !rollback.recheck.is_empty() {
                    tracing::warn!(
                        rounds = ?rollback.recheck,
                        fork,
                        "a reorg reached the blocks these rounds were settled from; they will be recomputed from the new chain and must match"
                    );
                }
                let discarded = rollback.recheck;
                report.reorg_from = Some(fork);
                report.rolled_back_settlements = discarded;
                next = fork;
                break;
            }
            let notes = block_notes(source, scanner, config, &block).await?;
            report.new_notes += store_block(store, &block, &notes)?;
            report.new_blocks += 1;
            next = height + 1;
        }
    }
    report.stored_tip = store.read(|db| db.tip())?.map(|b| b.height);
    Ok(report)
}

/// Records the note commitment of every note stored before schema version 2,
/// reading it from the note's transaction. Returns how many were filled.
pub async fn backfill_commitments(
    source: &mut dyn ChainSource,
    store: &Store,
    config: &GameConfig,
) -> Result<usize, EngineError> {
    let missing = store.read(|db| db.notes_without_cmx())?;
    for note in &missing {
        let raw = source.transaction(&note.txid).await?;
        let cmx = DigScanner::commitment_of(
            &note.txid_hex(),
            &raw,
            branch_for(config.params.network, note.height),
            note.pool,
            note.action_index,
        )?;
        store.write(|db| db.set_note_cmx(&note.txid, note.action_index, note.pool, &cmx))?;
    }
    if !missing.is_empty() {
        tracing::info!(
            notes = missing.len(),
            "backfilled note commitments for notes stored under schema 1"
        );
    }
    Ok(missing.len())
}

/// Returns the first height to forget if `block` does not extend the stored
/// chain.
async fn detect_reorg(
    source: &mut dyn ChainSource,
    store: &Store,
    config: &GameConfig,
    block: &CompactBlock,
) -> Result<Option<u32>, EngineError> {
    let height = block.height as u32;
    let Some(parent) = store.read(|db| db.block(height.wrapping_sub(1)))? else {
        return Ok(None);
    };
    if parent.hash == block.prev_hash {
        return Ok(None);
    }
    // Walk back until a stored block matches the chain.
    let mut candidate = parent.height;
    loop {
        let Some(stored) = store.read(|db| db.block(candidate))? else {
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

/// The vault's notes in one block: scans the compact outputs and fetches
/// and decrypts the full transaction of every hit. No database access.
async fn block_notes(
    source: &mut dyn ChainSource,
    scanner: &DigScanner,
    config: &GameConfig,
    block: &CompactBlock,
) -> Result<Vec<NoteRow>, EngineError> {
    let height = block.height as u32;
    let malformed = |reason: String| EngineError::MalformedBlock { height, reason };
    if block.hash.len() != 32 || block.prev_hash.len() != 32 {
        return Err(malformed("hash fields must be 32 bytes".into()));
    }
    let branch = branch_for(config.params.network, height);
    let mut notes = Vec::new();
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
        let full = scanner.decrypt_full(&txid_hex, &raw, branch, &hits)?;
        if !full.undecryptable.is_empty() {
            // Valid for the compact block, corrupt after it: not a game note.
            tracing::warn!(
                txid = %txid_hex,
                height,
                actions = ?full.undecryptable,
                "a payment to the vault does not decrypt in full; ignored (not a game note)"
            );
        }
        for note in full.notes {
            notes.push(NoteRow {
                txid,
                action_index: note.action_index,
                pool: note.pool,
                height,
                block_time: u64::from(block.time),
                value: note.value,
                memo: note.memo.to_vec(),
                cmx: Some(note.cmx),
            });
        }
    }
    Ok(notes)
}

/// Stores one block and its notes in one transaction; returns how many notes
/// were new.
fn store_block(
    store: &Store,
    block: &CompactBlock,
    notes: &[NoteRow],
) -> Result<usize, EngineError> {
    let height = block.height as u32;
    let inserted = store.write(|db| {
        db.transaction(|tx| {
            let mut inserted = Vec::new();
            for note in notes {
                if tx.insert_note(note)? {
                    inserted.push(note);
                }
            }
            tx.insert_block(&BlockRow {
                height,
                hash: block.hash.clone(),
                prev_hash: block.prev_hash.clone(),
                time: u64::from(block.time),
            })?;
            Ok(inserted)
        })
    })?;
    for note in &inserted {
        tracing::info!(
            height,
            txid = %note.txid_hex(),
            action = note.action_index,
            pool = ?note.pool,
            value = note.value,
            "received note at the vault address"
        );
    }
    Ok(inserted.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Pool;
    use crate::synthetic::{SyntheticChain, TestVault};

    #[test]
    fn a_server_serving_altered_bytes_is_failed_over() {
        let vault = TestVault::new(11);
        let mut chain = SyntheticChain::new(&vault, 100, 1_700_000_000);
        chain.push_block(vec![chain.payment(
            250_000,
            Pool::Ironwood,
            vault.dig_memo(0, 4, 2).encode(),
        )]);
        let id = chain.block(100).expect("block").vtx[0].txid.clone();
        let txid: [u8; 32] = id.as_slice().try_into().expect("32 bytes");
        let data = chain.transaction(&id).expect("raw");
        let served = |data: Vec<u8>| RawTransaction { data, height: 100 };
        assert!(served_as(Network::Test, &txid, &served(data.clone())).is_ok());

        // One byte of the last ciphertext (its tag) changed: same note
        // commitments, another txid.
        let mut altered = data.clone();
        let tx = Transaction::read(&data[..], BranchId::Nu6_3).expect("parse");
        let ct = tx.ironwood_bundle().expect("bundle").actions()[0]
            .encrypted_note()
            .enc_ciphertext
            .to_vec();
        let at = data
            .windows(ct.len())
            .position(|w| w == ct.as_slice())
            .expect("ciphertext");
        altered[at + ct.len() - 1] ^= 1;
        let err = served_as(Network::Test, &txid, &served(altered)).expect_err("altered");
        assert!(
            matches!(err, zeek_vault::VaultError::BadResponse(_)),
            "{err}"
        );

        let mut other = txid;
        other[0] ^= 1;
        assert!(served_as(Network::Test, &other, &served(data)).is_err());
        assert!(served_as(Network::Test, &txid, &served(vec![1, 2, 3])).is_err());
    }

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
        let store = Store::open_in_memory().expect("db");

        let report = ingest(&mut chain, &store, &scanner, &config)
            .await
            .expect("ingest");
        assert_eq!(report.new_notes, 1);
        assert_eq!(report.stored_tip, Some(1_005));
        assert_eq!(
            store
                .read(|db| db.block(1_000))
                .expect("block")
                .map(|b| b.height),
            Some(1_000)
        );
        assert!(
            store.read(|db| db.block(999)).expect("block").is_none(),
            "nothing below genesis"
        );
        let notes = store
            .read(|db| db.notes_in_range(1_000, 1_024))
            .expect("notes");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].height, 1_002);
        assert_eq!(notes[0].value, 200_000);
        let known = store.read(|db| db.known_note_ids()).expect("known");
        assert!(known.contains(&(notes[0].txid_hex(), notes[0].action_index)));
        let cmx = notes[0].cmx.expect("commitment recorded");
        assert_eq!(
            store
                .read(|db| db.note_by_cmx(&cmx))
                .expect("lookup")
                .map(|n| n.value),
            Some(200_000)
        );

        // A schema-1 row (no commitment) is backfilled from its transaction.
        store.write(|db| db.clear_note_commitments()).expect("null");
        assert_eq!(
            store
                .read(|db| db.notes_without_cmx())
                .expect("missing")
                .len(),
            1
        );
        let filled = backfill_commitments(&mut chain, &store, &config)
            .await
            .expect("backfill");
        assert_eq!(filled, 1);
        assert_eq!(
            store
                .read(|db| db.notes_in_range(1_000, 1_024))
                .expect("notes")[0]
                .cmx,
            Some(cmx)
        );

        // A second pass with nothing new is a no-op.
        let again = ingest(&mut chain, &store, &scanner, &config)
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
        let store = Store::open_in_memory().expect("db");
        ingest(&mut chain, &store, &scanner, &config)
            .await
            .expect("ingest");
        assert_eq!(store.read(|db| db.note_count()).expect("count"), 1);
        let before = store.read(|db| db.dump()).expect("dump");

        // The chain forks at 105: the note's block is replaced by an empty one
        // and a different note lands at 106.
        chain.truncate(105);
        chain.advance(1); // new 105
        chain.push_block(vec![chain.payment(400_000, Pool::Ironwood, [0xF6; 512])]); // new 106
        chain.advance(3); // 107..=109
        let report = ingest(&mut chain, &store, &scanner, &config)
            .await
            .expect("ingest");
        assert_eq!(report.reorg_from, Some(105));
        assert!(report.rolled_back_settlements.is_empty());
        assert_eq!(report.stored_tip, Some(109));
        let notes = store.read(|db| db.notes_in_range(100, 200)).expect("notes");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].height, 106);
        assert_eq!(notes[0].value, 400_000);
        assert_ne!(store.read(|db| db.dump()).expect("dump"), before);
        assert_eq!(
            store.read(|db| db.block(105)).expect("b").map(|b| b.hash),
            chain.block(105).map(|b| b.hash.clone())
        );
    }
}
