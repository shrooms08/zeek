//! The wallet: scanning, notes, balance and history.
//!
//! Scanning reuses `zcash_client_backend::scanning::scan_block`, which
//! trial-decrypts every Ironwood and Orchard output of a compact block with
//! the wallet's external and internal incoming viewing keys and reports each
//! decrypted note with its commitment-tree position and nullifier. It is
//! called with an empty nullifier set, so it reports every nullifier in the
//! block; this wallet matches those against its own notes to detect spends.
//! The block's Ironwood commitments go into the wallet's shard tree with one
//! checkpoint per block, which keeps witnesses for the wallet's notes.
//!
//! The store is this crate's own ([`crate::store`]) rather than
//! `zcash_client_memory`: that crate is not published for the pinned
//! `zcash_client_backend 0.24` line, and the wallet needs only one account,
//! one spendable pool and no transparent support.

use std::collections::BTreeSet;

use incrementalmerkletree::{Marking, Position, Retention};
use orchard::keys::{PreparedIncomingViewingKey, Scope};
use orchard::note_encryption::{IronwoodDomain, OrchardDomain};

use prost::Message;
use serde::{Deserialize, Serialize};
use shardtree::store::ShardStore;
use zcash_client_backend::data_api::BlockMetadata;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::TreeState;
use zcash_client_backend::scanning::{scan_block, Nullifiers, ScanError, ScanningKeys};
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::block::BlockHash;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, TEST_NETWORK};

use crate::error::PocketError;
use crate::keys::Keys;
use crate::store::{
    self, BlockMeta, IronwoodTree, NotePool, NoteScope, Outgoing, OutgoingStatus, SpentIn,
    StoredNote, TreeSnapshot, WalletState, DECOY_POOL, MAX_CHECKPOINTS, STATE_VERSION,
};

/// Confirmations a received (external) note needs before it is spent (ZIP 315).
pub const UNTRUSTED_CONFIRMATIONS: u32 = 10;
/// Confirmations this wallet's own change needs (ZIP 315).
pub const TRUSTED_CONFIRMATIONS: u32 = 3;

/// The wallet in memory.
pub struct Wallet {
    pub(crate) keys: Keys,
    pub(crate) state: WalletState,
    pub(crate) tree: IronwoodTree,
    scanning_keys: ScanningKeys<u32, (u32, zip32::Scope)>,
}

/// What one scan call did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub from_height: u32,
    pub scanned_height: u32,
    pub blocks: u32,
    pub new_notes: u32,
    pub spent_notes: u32,
}

/// A transaction the wallet wants to read memos from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoRequest {
    pub txid: String,
    pub height: u32,
}

/// The memo requests and a pool of decoy txids to hide them among.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoRequests {
    pub wanted: Vec<MemoRequest>,
    pub decoys: Vec<MemoRequest>,
}

/// Balance in zatoshis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Balance {
    /// Spendable now (confirmed, not reserved by a pending transaction).
    pub spendable_zat: u64,
    /// Waiting for confirmations, including change of pending transactions.
    pub pending_zat: u64,
    /// Notes in the closed legacy Orchard pool (not spendable here).
    pub orchard_pool_zat: u64,
    pub total_zat: u64,
    pub scanned_height: u32,
    /// Blocks until the first confirming amount becomes spendable (by this
    /// wallet's policy: received notes 10 confirmations, its own change 3,
    /// and a transaction not mined yet one block more); None if nothing is
    /// confirming.
    pub ready_in_blocks: Option<u32>,
    /// Blocks until everything confirming is spendable.
    pub all_ready_in_blocks: Option<u32>,
}

/// Blocks from `scanned` until a note mined at `height` reaches `needed`
/// confirmations (0 when it already has them). Spendable means
/// `scanned + 1 − height ≥ needed`.
pub fn blocks_until_spendable(height: u32, needed: u32, scanned: u32) -> u32 {
    height
        .saturating_add(needed)
        .saturating_sub(1)
        .saturating_sub(scanned)
}

/// One history line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HistoryEntry {
    /// A payment received (funding, a treasure payout, a refund).
    Received {
        txid: String,
        height: u32,
        value_zat: u64,
        /// Memo text, a description of a non-text memo, or `None` until the
        /// transaction is fetched.
        memo: Option<String>,
        pool: NotePool,
        spent: bool,
    },
    /// A transaction this wallet sent.
    Sent(Outgoing),
}

fn scan_err(e: ScanError) -> PocketError {
    match e {
        ScanError::PrevHashMismatch { at_height } => PocketError::Reorg {
            fork_height: u32::from(at_height),
        },
        other => PocketError::BadChainData(other.to_string()),
    }
}

/// Renders a ZIP 302 memo: text, or a description of anything else.
pub fn describe_memo(memo: &[u8]) -> String {
    let end = memo.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    let trimmed = &memo[..end];
    match trimmed.first() {
        None => "(no memo)".to_owned(),
        Some(0xF6) if trimmed.len() == 1 => "(no memo)".to_owned(),
        Some(b) if *b <= 0xF4 => match std::str::from_utf8(trimmed) {
            Ok(text) => text.to_owned(),
            Err(_) => format!("(invalid UTF-8 text memo, {} bytes)", trimmed.len()),
        },
        Some(0xFF) => format!("(binary memo, {} bytes)", trimmed.len()),
        Some(b) => format!("(reserved memo type {b:#04x})"),
    }
}

impl Wallet {
    fn scanning_keys(keys: &Keys) -> Result<ScanningKeys<u32, (u32, zip32::Scope)>, PocketError> {
        Ok(ScanningKeys::from_account_ufvks([(0u32, keys.ufvk()?)]))
    }

    /// A new wallet whose birthday is the block `tree_state` describes:
    /// scanning starts at the next block.
    pub fn new(keys: Keys, tree_state: &[u8]) -> Result<Self, PocketError> {
        let tree_state = TreeState::decode(tree_state)
            .map_err(|e| PocketError::BadChainData(format!("tree state: {e}")))?;
        let chain = tree_state
            .to_chain_state()
            .map_err(|e| PocketError::BadChainData(format!("tree state: {e}")))?;
        let size = |s: u64| u32::try_from(s).map_err(|_| PocketError::Overflow("tree size"));
        let scanned = BlockMeta {
            height: u32::from(chain.block_height()),
            hash: chain.block_hash().0,
            sapling_tree_size: size(chain.final_sapling_tree().tree_size())?,
            orchard_tree_size: size(chain.final_orchard_tree().tree_size())?,
            ironwood_tree_size: size(chain.final_ironwood_tree().tree_size())?,
        };
        let mut tree = store::restore(&TreeSnapshot::default())?;
        tree.insert_frontier(
            chain.final_ironwood_tree().clone(),
            Retention::Checkpoint {
                id: chain.block_height(),
                marking: Marking::Reference,
            },
        )
        .map_err(|e| PocketError::Corrupt(format!("birthday frontier: {e:?}")))?;
        let scanning_keys = Self::scanning_keys(&keys)?;
        Ok(Wallet {
            keys,
            state: WalletState {
                version: STATE_VERSION,
                birthday_height: scanned.height,
                scanned,
                recent: Vec::new(),
                tree: TreeSnapshot::default(),
                notes: Vec::new(),
                next_receiver_index: 1,
                outgoing: Vec::new(),
                decoys: Vec::new(),
            },
            tree,
            scanning_keys,
        })
    }

    /// Reopens a wallet from its decrypted state.
    pub fn from_state(keys: Keys, state: WalletState) -> Result<Self, PocketError> {
        let tree = store::restore(&state.tree)?;
        let scanning_keys = Self::scanning_keys(&keys)?;
        Ok(Wallet {
            keys,
            state,
            tree,
            scanning_keys,
        })
    }

    /// The serialized state (tree included), ready for encryption.
    pub fn state_bytes(&mut self) -> Result<Vec<u8>, PocketError> {
        self.state.tree = store::snapshot(&mut self.tree)?;
        store::to_bytes(&self.state)
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    pub fn scanned_height(&self) -> u32 {
        self.state.scanned.height
    }

    pub fn birthday_height(&self) -> u32 {
        self.state.birthday_height
    }

    fn block_metadata(meta: &BlockMeta) -> BlockMetadata {
        BlockMetadata::from_parts(
            BlockHeight::from_u32(meta.height),
            BlockHash(meta.hash),
            Some(meta.sapling_tree_size),
            Some(meta.orchard_tree_size),
            Some(meta.ironwood_tree_size),
        )
    }

    /// Scans blocks encoded as the relay serves them: per block a `u32`
    /// little-endian length and the `CompactBlock` protobuf. Blocks must
    /// follow the last scanned block; a block whose parent hash does not
    /// match is reported as [`PocketError::Reorg`] and nothing after it is
    /// applied.
    pub fn scan(&mut self, encoded: &[u8]) -> Result<ScanReport, PocketError> {
        let from_height = self.state.scanned.height + 1;
        let mut report = ScanReport {
            from_height,
            scanned_height: self.state.scanned.height,
            blocks: 0,
            new_notes: 0,
            spent_notes: 0,
        };
        let mut rest = encoded;
        while !rest.is_empty() {
            if rest.len() < 4 {
                return Err(PocketError::BadChainData("truncated length prefix".into()));
            }
            let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            let body = rest
                .get(4..4 + len)
                .ok_or_else(|| PocketError::BadChainData("truncated block".into()))?;
            rest = &rest[4 + len..];
            let block = CompactBlock::decode(body)
                .map_err(|e| PocketError::BadChainData(format!("compact block: {e}")))?;
            let (new_notes, spent) = self.scan_one(block)?;
            report.blocks += 1;
            report.new_notes += new_notes;
            report.spent_notes += spent;
            report.scanned_height = self.state.scanned.height;
        }
        Ok(report)
    }

    fn scan_one(&mut self, block: CompactBlock) -> Result<(u32, u32), PocketError> {
        let height = u32::try_from(block.height)
            .map_err(|_| PocketError::BadChainData("block height overflows".into()))?;
        let expected = self.state.scanned.height + 1;
        if height != expected {
            return Err(PocketError::BadChainData(format!(
                "expected block {expected}, got {height}"
            )));
        }
        // Decoy candidates: transactions with shielded actions.
        for tx in &block.vtx {
            if !tx.ironwood_actions.is_empty() || !tx.actions.is_empty() {
                let mut display = tx.txid.clone();
                display.reverse();
                self.state.decoys.push((hex::encode(display), height));
            }
        }
        if self.state.decoys.len() > DECOY_POOL {
            let excess = self.state.decoys.len() - DECOY_POOL;
            self.state.decoys.drain(..excess);
        }

        let prior = Self::block_metadata(&self.state.scanned);
        let scanned = scan_block(
            &TEST_NETWORK,
            block,
            &self.scanning_keys,
            &Nullifiers::<u32>::empty(),
            Some(&prior),
        )
        .map_err(scan_err)?;
        let metadata = scanned.to_block_metadata();

        // Received notes.
        let mut new_notes = 0u32;
        for tx in scanned.transactions() {
            let txid = tx.txid().to_string();
            let pools = [
                (NotePool::Ironwood, tx.ironwood_outputs()),
                (NotePool::Orchard, tx.orchard_outputs()),
            ];
            for (pool, outputs) in pools {
                for output in outputs {
                    let (note, _) = output.note();
                    let scope = match output.recipient_key_scope() {
                        Some(zip32::Scope::Internal) => NoteScope::Internal,
                        _ => NoteScope::External,
                    };
                    let nullifier = output
                        .nf()
                        .ok_or_else(|| {
                            PocketError::Corrupt("decrypted note without nullifier".into())
                        })?
                        .to_bytes();
                    let diversifier_index = match scope {
                        NoteScope::External => self
                            .keys
                            .fvk()
                            .to_ivk(Scope::External)
                            .diversifier_index(&note.recipient())
                            .and_then(|j| {
                                let bytes = j.as_bytes();
                                let mut le = [0u8; 8];
                                le.copy_from_slice(&bytes[..8]);
                                bytes[8..]
                                    .iter()
                                    .all(|b| *b == 0)
                                    .then(|| u64::from_le_bytes(le))
                            }),
                        NoteScope::Internal => None,
                    };
                    let index = u16::try_from(output.index())
                        .map_err(|_| PocketError::BadChainData("action index overflows".into()))?;
                    if self
                        .state
                        .notes
                        .iter()
                        .any(|n| n.txid == txid && n.action_index == index && n.pool == pool)
                    {
                        continue;
                    }
                    self.state.notes.push(StoredNote {
                        txid: txid.clone(),
                        action_index: index,
                        pool,
                        height,
                        value: note.value().inner(),
                        recipient: note.recipient().to_raw_address_bytes().to_vec(),
                        rho: note.rho().to_bytes(),
                        rseed: *note.rseed().as_bytes(),
                        note_version: match note.version() {
                            orchard::note::NoteVersion::V2 => 2,
                            orchard::note::NoteVersion::V3 => 3,
                        },
                        scope,
                        diversifier_index,
                        position: u64::from(output.note_commitment_tree_position()),
                        nullifier,
                        spent: None,
                        memo: None,
                    });
                    new_notes += 1;
                }
            }
        }

        // Spends: every nullifier revealed in the block.
        let mut spent = 0u32;
        let maps = [
            scanned.ironwood().nullifier_map(),
            scanned.orchard().nullifier_map(),
        ];
        for map in maps {
            for (_, txid, nullifiers) in map {
                let txid = txid.to_string();
                for nf in nullifiers {
                    let nf = nf.to_bytes();
                    if let Some(note) = self
                        .state
                        .notes
                        .iter_mut()
                        .find(|n| n.nullifier == nf && n.spent.is_none())
                    {
                        note.spent = Some(SpentIn {
                            txid: txid.clone(),
                            height,
                        });
                        spent += 1;
                        if let Some(out) = self.state.outgoing.iter_mut().find(|o| o.txid == txid) {
                            out.status = OutgoingStatus::Mined;
                            out.mined_height = Some(height);
                        }
                    }
                }
            }
        }

        // Commitments into the tree, one checkpoint per block.
        let commitments = scanned.into_commitments();
        if !commitments.ironwood.is_empty() {
            self.tree
                .batch_insert(
                    Position::from(u64::from(self.state.scanned.ironwood_tree_size)),
                    commitments.ironwood.into_iter(),
                )
                .map_err(|e| PocketError::Corrupt(format!("tree insert: {e:?}")))?;
        }
        let block_height = BlockHeight::from_u32(height);
        let checkpointed = self
            .tree
            .store_mut()
            .max_checkpoint_id()
            .map_err(|e| PocketError::Corrupt(format!("{e:?}")))?;
        if checkpointed != Some(block_height) {
            self.tree
                .checkpoint(block_height)
                .map_err(|e| PocketError::Corrupt(format!("checkpoint: {e:?}")))?;
        }

        // Expire transactions that can no longer be mined.
        for out in &mut self.state.outgoing {
            if matches!(
                out.status,
                OutgoingStatus::Broadcast | OutgoingStatus::Built
            ) && height >= out.expiry_height
            {
                out.status = OutgoingStatus::Expired;
            }
        }

        self.state.recent.push(self.state.scanned);
        if self.state.recent.len() > MAX_CHECKPOINTS {
            let excess = self.state.recent.len() - MAX_CHECKPOINTS;
            self.state.recent.drain(..excess);
        }
        let size =
            |s: Option<u32>| s.ok_or_else(|| PocketError::BadChainData("tree size missing".into()));
        self.state.scanned = BlockMeta {
            height,
            hash: metadata.block_hash().0,
            sapling_tree_size: size(metadata.sapling_tree_size())?,
            orchard_tree_size: size(metadata.orchard_tree_size())?,
            ironwood_tree_size: size(metadata.ironwood_tree_size())?,
        };
        Ok((new_notes, spent))
    }

    /// Forgets everything above `height` (a reorg). `height` must be one of
    /// the last [`MAX_CHECKPOINTS`] scanned blocks.
    pub fn rewind(&mut self, height: u32) -> Result<(), PocketError> {
        if height >= self.state.scanned.height {
            return Ok(());
        }
        let position = self
            .state
            .recent
            .iter()
            .rposition(|m| m.height == height)
            .ok_or_else(|| {
                PocketError::Corrupt(format!(
                    "cannot rewind to {height}: older than the {MAX_CHECKPOINTS} kept blocks; restore from the phrase"
                ))
            })?;
        let truncated = self
            .tree
            .truncate_to_checkpoint(&BlockHeight::from_u32(height))
            .map_err(|e| PocketError::Corrupt(format!("rewind: {e:?}")))?;
        if !truncated && height != self.state.birthday_height {
            return Err(PocketError::Corrupt(format!("no checkpoint at {height}")));
        }
        self.state.scanned = self.state.recent[position];
        self.state.recent.truncate(position);
        self.state.notes.retain(|n| n.height <= height);
        for note in &mut self.state.notes {
            if note.spent.as_ref().is_some_and(|s| s.height > height) {
                note.spent = None;
            }
        }
        for out in &mut self.state.outgoing {
            if out.mined_height.is_some_and(|h| h > height) {
                out.mined_height = None;
                out.status = OutgoingStatus::Broadcast;
            }
        }
        self.state.decoys.retain(|(_, h)| *h <= height);
        Ok(())
    }

    /// Transactions whose memos the wallet still needs, and decoys.
    pub fn memo_requests(&self) -> MemoRequests {
        let mut seen = BTreeSet::new();
        let wanted: Vec<MemoRequest> = self
            .state
            .notes
            .iter()
            .filter(|n| n.scope == NoteScope::External && n.memo.is_none())
            .filter(|n| seen.insert(n.txid.clone()))
            .map(|n| MemoRequest {
                txid: n.txid.clone(),
                height: n.height,
            })
            .collect();
        let decoys = self
            .state
            .decoys
            .iter()
            .filter(|(txid, _)| !seen.contains(txid))
            .map(|(txid, height)| MemoRequest {
                txid: txid.clone(),
                height: *height,
            })
            .collect();
        MemoRequests { wanted, decoys }
    }

    /// Reads the memos of this wallet's notes from a full transaction mined
    /// at `height`. Returns how many memos were attached.
    pub fn attach_transaction(&mut self, raw: &[u8], height: u32) -> Result<u32, PocketError> {
        let branch = BranchId::for_height(&TEST_NETWORK, BlockHeight::from_u32(height));
        let tx = Transaction::read(raw, branch)
            .map_err(|e| PocketError::BadChainData(format!("transaction: {e}")))?;
        let txid = tx.txid().to_string();
        let fvk = self.keys.fvk().clone();
        let ivks = [
            PreparedIncomingViewingKey::new(&fvk.to_ivk(Scope::External)),
            PreparedIncomingViewingKey::new(&fvk.to_ivk(Scope::Internal)),
        ];
        let mut attached = 0u32;
        for note in self
            .state
            .notes
            .iter_mut()
            .filter(|n| n.txid == txid && n.memo.is_none())
        {
            let bundle = match note.pool {
                NotePool::Ironwood => tx.ironwood_bundle(),
                NotePool::Orchard => tx.orchard_bundle(),
            };
            let Some(action) = bundle.and_then(|b| b.actions().get(usize::from(note.action_index)))
            else {
                continue;
            };
            for ivk in &ivks {
                let decrypted = match note.pool {
                    NotePool::Ironwood => {
                        try_note_decryption(&IronwoodDomain::for_action(action), ivk, action)
                    }
                    NotePool::Orchard => {
                        try_note_decryption(&OrchardDomain::for_action(action), ivk, action)
                    }
                };
                if let Some((decrypted_note, _, memo)) = decrypted {
                    if decrypted_note.value().inner() == note.value {
                        note.memo = Some(memo.to_vec());
                        attached += 1;
                    }
                    break;
                }
            }
        }
        Ok(attached)
    }

    /// Notes reserved by transactions that may still be mined.
    fn reserved(&self) -> BTreeSet<(String, u16)> {
        self.state
            .outgoing
            .iter()
            .filter(|o| matches!(o.status, OutgoingStatus::Built | OutgoingStatus::Broadcast))
            .flat_map(|o| o.spends.iter().cloned())
            .collect()
    }

    /// Whether `note` is spendable in a transaction targeting `target`.
    pub(crate) fn is_spendable(note: &StoredNote, target: u32) -> bool {
        let needed = match note.scope {
            NoteScope::External => UNTRUSTED_CONFIRMATIONS,
            NoteScope::Internal => TRUSTED_CONFIRMATIONS,
        };
        note.pool == NotePool::Ironwood
            && note.spent.is_none()
            && note.value > 0
            && target.saturating_sub(note.height) >= needed
    }

    /// Unspent, unreserved Ironwood notes spendable at `target`, largest first.
    pub(crate) fn spendable_notes(&self, target: u32) -> Vec<&StoredNote> {
        let reserved = self.reserved();
        let mut notes: Vec<&StoredNote> = self
            .state
            .notes
            .iter()
            .filter(|n| Self::is_spendable(n, target))
            .filter(|n| !reserved.contains(&(n.txid.clone(), n.action_index)))
            .collect();
        notes.sort_by(|a, b| b.value.cmp(&a.value).then(a.position.cmp(&b.position)));
        notes
    }

    pub fn balance(&self) -> Result<Balance, PocketError> {
        let target = self.state.scanned.height + 1;
        let reserved = self.reserved();
        let mut b = Balance {
            spendable_zat: 0,
            pending_zat: 0,
            orchard_pool_zat: 0,
            total_zat: 0,
            scanned_height: self.state.scanned.height,
            ready_in_blocks: None,
            all_ready_in_blocks: None,
        };
        let scanned = self.state.scanned.height;
        let mut waits: Vec<u32> = Vec::new();
        let add = |a: u64, v: u64| a.checked_add(v).ok_or(PocketError::Overflow("balance"));
        for note in self.state.notes.iter().filter(|n| n.spent.is_none()) {
            if reserved.contains(&(note.txid.clone(), note.action_index)) {
                continue;
            }
            if note.pool == NotePool::Orchard {
                b.orchard_pool_zat = add(b.orchard_pool_zat, note.value)?;
            } else if Self::is_spendable(note, target) {
                b.spendable_zat = add(b.spendable_zat, note.value)?;
            } else {
                b.pending_zat = add(b.pending_zat, note.value)?;
                if note.value > 0 {
                    let needed = match note.scope {
                        NoteScope::External => UNTRUSTED_CONFIRMATIONS,
                        NoteScope::Internal => TRUSTED_CONFIRMATIONS,
                    };
                    waits.push(blocks_until_spendable(note.height, needed, scanned).max(1));
                }
            }
        }
        for out in &self.state.outgoing {
            if matches!(
                out.status,
                OutgoingStatus::Built | OutgoingStatus::Broadcast
            ) {
                let back = add(out.change_zat, out.kind.self_outputs_zat()?)?;
                b.pending_zat = add(b.pending_zat, back)?;
                if back > 0 {
                    // Mined in the next block at best, then its confirmations.
                    waits.push(TRUSTED_CONFIRMATIONS);
                }
            }
        }
        b.ready_in_blocks = waits.iter().copied().min();
        b.all_ready_in_blocks = waits.iter().copied().max();
        b.total_zat = add(add(b.spendable_zat, b.pending_zat)?, b.orchard_pool_zat)?;
        Ok(b)
    }

    /// Received payments and sent transactions, newest first.
    pub fn history(&self) -> Vec<HistoryEntry> {
        let mut entries: Vec<(u32, HistoryEntry)> = Vec::new();
        let mut seen = BTreeSet::new();
        for note in self
            .state
            .notes
            .iter()
            .filter(|n| n.scope == NoteScope::External)
        {
            if !seen.insert((note.txid.clone(), note.action_index)) {
                continue;
            }
            entries.push((
                note.height,
                HistoryEntry::Received {
                    txid: note.txid.clone(),
                    height: note.height,
                    value_zat: note.value,
                    memo: note.memo.as_deref().map(describe_memo),
                    pool: note.pool,
                    spent: note.spent.is_some(),
                },
            ));
        }
        for out in &self.state.outgoing {
            let height = out.mined_height.unwrap_or(out.target_height);
            entries.push((height, HistoryEntry::Sent(out.clone())));
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.0));
        entries.into_iter().map(|(_, e)| e).collect()
    }

    /// Marks an outgoing transaction as accepted by the relay.
    pub fn mark_broadcast(&mut self, txid: &str) -> Result<(), PocketError> {
        let out = self
            .state
            .outgoing
            .iter_mut()
            .find(|o| o.txid == txid)
            .ok_or_else(|| PocketError::InvalidRequest(format!("unknown transaction {txid}")))?;
        if out.status == OutgoingStatus::Built {
            out.status = OutgoingStatus::Broadcast;
        }
        Ok(())
    }

    /// Releases the notes of a transaction that was never broadcast.
    pub fn abandon(&mut self, txid: &str) -> Result<(), PocketError> {
        let out = self
            .state
            .outgoing
            .iter_mut()
            .find(|o| o.txid == txid)
            .ok_or_else(|| PocketError::InvalidRequest(format!("unknown transaction {txid}")))?;
        if out.status == OutgoingStatus::Built {
            out.status = OutgoingStatus::Expired;
        }
        Ok(())
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[test]
    fn blocks_until_spendable_follows_the_confirmation_policy() {
        // Mined at 100: 10 confirmations reached once scanned is 109.
        assert_eq!(blocks_until_spendable(100, UNTRUSTED_CONFIRMATIONS, 100), 9);
        assert_eq!(blocks_until_spendable(100, UNTRUSTED_CONFIRMATIONS, 108), 1);
        assert_eq!(blocks_until_spendable(100, UNTRUSTED_CONFIRMATIONS, 109), 0);
        // Own change: 3 confirmations.
        assert_eq!(blocks_until_spendable(100, TRUSTED_CONFIRMATIONS, 100), 2);
        assert_eq!(blocks_until_spendable(100, TRUSTED_CONFIRMATIONS, 102), 0);
    }
}
