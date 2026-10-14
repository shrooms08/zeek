//! The wallet state persisted (encrypted) in the browser.
//!
//! Everything the wallet learned from the chain: the last scanned block and
//! the metadata of the 100 before it (for reorgs), the Ironwood note
//! commitment tree as shards and checkpoints, received notes with their
//! positions and nullifiers, transactions this wallet built, the next unused
//! diversifier index, and a pool of recently seen txids used as decoys when
//! fetching full transactions. Serialized with postcard, then encrypted by
//! [`crate::crypto::seal_state`].

use std::collections::BTreeSet;
use std::convert::Infallible;

use incrementalmerkletree::{Address, Level, Position};
use orchard::tree::MerkleHashOrchard;
use serde::{Deserialize, Serialize};
use shardtree::store::memory::MemoryShardStore;
use shardtree::store::{Checkpoint, ShardStore, TreeState};
use shardtree::{LocatedPrunableTree, ShardTree};
use zcash_client_backend::serialization::shardtree::{read_shard, write_shard};
use zcash_protocol::consensus::BlockHeight;

use crate::error::PocketError;

/// Depth of the Orchard-protocol note commitment tree.
pub const TREE_DEPTH: u8 = 32;
/// Shard height used by `zcash_client_backend` for Orchard-protocol trees.
pub const SHARD_HEIGHT: u8 = 16;
/// Checkpoints kept: one per scanned block, so reorgs up to this deep and
/// anchors up to this old are possible.
pub const MAX_CHECKPOINTS: usize = 100;
/// Decoy txids kept for memo fetches.
pub const DECOY_POOL: usize = 256;

/// The in-memory Ironwood tree.
pub type IronwoodTree =
    ShardTree<MemoryShardStore<MerkleHashOrchard, BlockHeight>, TREE_DEPTH, SHARD_HEIGHT>;

/// Format version of [`WalletState`].
pub const STATE_VERSION: u32 = 1;

/// A scanned block's position in the chain and the tree sizes after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockMeta {
    pub height: u32,
    /// Block hash, internal byte order.
    pub hash: [u8; 32],
    pub sapling_tree_size: u32,
    pub orchard_tree_size: u32,
    pub ironwood_tree_size: u32,
}

/// Which pool a note lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotePool {
    Ironwood,
    /// The legacy Orchard pool is closed to deposits since NU6.3; such notes
    /// are reported but not spendable by this wallet.
    Orchard,
}

/// Key scope a note was received under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteScope {
    /// Payments to one of the wallet's addresses.
    External,
    /// The wallet's own change.
    Internal,
}

/// Where a note was spent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpentIn {
    pub txid: String,
    pub height: u32,
}

/// A note the wallet received.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNote {
    /// Display-order hex.
    pub txid: String,
    pub action_index: u16,
    pub pool: NotePool,
    pub height: u32,
    pub value: u64,
    pub recipient: Vec<u8>,
    pub rho: [u8; 32],
    pub rseed: [u8; 32],
    /// Note plaintext version: 2 (Orchard) or 3 (Ironwood).
    pub note_version: u8,
    pub scope: NoteScope,
    /// Diversifier index of the receiving address (external notes).
    pub diversifier_index: Option<u64>,
    pub position: u64,
    pub nullifier: [u8; 32],
    pub spent: Option<SpentIn>,
    /// The 512-byte memo, once the full transaction was fetched.
    pub memo: Option<Vec<u8>>,
}

impl StoredNote {
    /// Rebuilds the Orchard note.
    pub fn note(&self) -> Result<orchard::Note, PocketError> {
        let corrupt = |what: &str| PocketError::Corrupt(format!("note {}: {what}", self.txid));
        let recipient_bytes = <[u8; 43]>::try_from(self.recipient.as_slice())
            .map_err(|_| corrupt("recipient length"))?;
        let recipient = Option::from(orchard::Address::from_raw_address_bytes(&recipient_bytes))
            .ok_or_else(|| corrupt("recipient"))?;
        let rho: orchard::note::Rho = Option::from(orchard::note::Rho::from_bytes(&self.rho))
            .ok_or_else(|| corrupt("rho"))?;
        let rseed = Option::from(orchard::note::RandomSeed::from_bytes(self.rseed, &rho))
            .ok_or_else(|| corrupt("rseed"))?;
        let version = match self.note_version {
            2 => orchard::note::NoteVersion::V2,
            3 => orchard::note::NoteVersion::V3,
            _ => return Err(corrupt("note version")),
        };
        Option::from(orchard::Note::from_parts(
            recipient,
            orchard::value::NoteValue::from_raw(self.value),
            rho,
            rseed,
            version,
        ))
        .ok_or_else(|| corrupt("note"))
    }
}

/// What a transaction this wallet built was for. Externally tagged
/// (`{"dig": {...}}`): postcard, the storage format, cannot read internally
/// tagged enums.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutgoingKind {
    Dig {
        round: u32,
        tiles: Vec<u8>,
        amount_each_zat: u64,
        /// Diversifier indices of the fresh payout receivers, one per tile.
        receiver_indices: Vec<u64>,
        memo_encoding: String,
    },
    Send {
        /// The destination address as given.
        to: String,
        amount_zat: u64,
        memo: Option<String>,
    },
}

/// Lifecycle of an outgoing transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutgoingStatus {
    /// Built and recorded; not yet accepted by the relay.
    Built,
    /// Accepted by the relay, waiting to be mined.
    Broadcast,
    /// Seen spending this wallet's notes in a block.
    Mined,
    /// Not mined before its expiry height, or abandoned before broadcast.
    Expired,
}

/// A transaction this wallet built.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outgoing {
    pub txid: String,
    pub kind: OutgoingKind,
    pub fee_zat: u64,
    /// The notes it spends: (txid, action index).
    pub spends: Vec<(String, u16)>,
    pub change_zat: u64,
    pub target_height: u32,
    pub expiry_height: u32,
    pub status: OutgoingStatus,
    pub mined_height: Option<u32>,
}

/// One shard of the tree, serialized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardBytes {
    pub level: u8,
    pub index: u64,
    pub bytes: Vec<u8>,
}

/// One checkpoint: id (block height), tree position, marks removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointBytes {
    pub height: u32,
    pub position: Option<u64>,
    pub marks_removed: Vec<u64>,
}

/// The tree, serialized.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSnapshot {
    pub shards: Vec<ShardBytes>,
    pub cap: Vec<u8>,
    pub checkpoints: Vec<CheckpointBytes>,
}

/// Everything persisted, apart from the live tree (see [`TreeSnapshot`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletState {
    pub version: u32,
    pub birthday_height: u32,
    /// The last scanned block.
    pub scanned: BlockMeta,
    /// Up to [`MAX_CHECKPOINTS`] blocks before `scanned`, oldest first.
    pub recent: Vec<BlockMeta>,
    pub tree: TreeSnapshot,
    pub notes: Vec<StoredNote>,
    /// Next unused external diversifier index (0 is the receive address).
    pub next_receiver_index: u64,
    pub outgoing: Vec<Outgoing>,
    /// Recently seen txids with shielded actions: (txid, height).
    pub decoys: Vec<(String, u32)>,
}

fn tree_err<E: std::fmt::Debug>(e: E) -> PocketError {
    PocketError::Corrupt(format!("commitment tree: {e:?}"))
}

/// Serializes the tree's shards, cap and checkpoints.
pub fn snapshot(tree: &mut IronwoodTree) -> Result<TreeSnapshot, PocketError> {
    let store = tree.store_mut();
    let mut shards = Vec::new();
    for addr in store.get_shard_roots().map_err(tree_err::<Infallible>)? {
        if let Some(shard) = store.get_shard(addr).map_err(tree_err::<Infallible>)? {
            let mut bytes = Vec::new();
            write_shard(&mut bytes, shard.root()).map_err(tree_err)?;
            shards.push(ShardBytes {
                level: u8::from(addr.level()),
                index: addr.index(),
                bytes,
            });
        }
    }
    let mut cap = Vec::new();
    write_shard(&mut cap, &store.get_cap().map_err(tree_err::<Infallible>)?).map_err(tree_err)?;
    let mut checkpoints = Vec::new();
    let count = store.checkpoint_count().map_err(tree_err::<Infallible>)?;
    store
        .with_checkpoints(count, |id, checkpoint| {
            checkpoints.push(CheckpointBytes {
                height: u32::from(*id),
                position: checkpoint.position().map(u64::from),
                marks_removed: checkpoint
                    .marks_removed()
                    .iter()
                    .map(|p| u64::from(*p))
                    .collect(),
            });
            Ok(())
        })
        .map_err(tree_err::<Infallible>)?;
    Ok(TreeSnapshot {
        shards,
        cap,
        checkpoints,
    })
}

/// Rebuilds the tree from a snapshot.
pub fn restore(snapshot: &TreeSnapshot) -> Result<IronwoodTree, PocketError> {
    let mut store = MemoryShardStore::<MerkleHashOrchard, BlockHeight>::empty();
    for shard in &snapshot.shards {
        let addr = Address::from_parts(Level::from(shard.level), shard.index);
        let tree = read_shard(shard.bytes.as_slice()).map_err(tree_err)?;
        let located = LocatedPrunableTree::from_parts(addr, tree).map_err(tree_err)?;
        store.put_shard(located).map_err(tree_err::<Infallible>)?;
    }
    if !snapshot.cap.is_empty() {
        store
            .put_cap(read_shard(snapshot.cap.as_slice()).map_err(tree_err)?)
            .map_err(tree_err::<Infallible>)?;
    }
    for checkpoint in &snapshot.checkpoints {
        let state = match checkpoint.position {
            Some(p) => TreeState::AtPosition(Position::from(p)),
            None => TreeState::Empty,
        };
        let marks: BTreeSet<Position> = checkpoint
            .marks_removed
            .iter()
            .map(|p| Position::from(*p))
            .collect();
        store
            .add_checkpoint(
                BlockHeight::from_u32(checkpoint.height),
                Checkpoint::from_parts(state, marks),
            )
            .map_err(tree_err::<Infallible>)?;
    }
    Ok(ShardTree::new(store, MAX_CHECKPOINTS))
}

/// Serializes the state for encryption.
pub fn to_bytes(state: &WalletState) -> Result<Vec<u8>, PocketError> {
    postcard::to_allocvec(state).map_err(|e| PocketError::Corrupt(format!("serialize: {e}")))
}

/// Parses decrypted state.
pub fn from_bytes(bytes: &[u8]) -> Result<WalletState, PocketError> {
    let state: WalletState = postcard::from_bytes(bytes)
        .map_err(|e| PocketError::Corrupt(format!("deserialize: {e}")))?;
    if state.version != STATE_VERSION {
        return Err(PocketError::Corrupt(format!(
            "unsupported state version {}",
            state.version
        )));
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use incrementalmerkletree::Retention;

    #[test]
    fn tree_snapshot_round_trips_with_witnesses() {
        let mut tree = restore(&TreeSnapshot::default()).expect("empty tree");
        let leaf = |i: u64| {
            MerkleHashOrchard::from_bytes(&{
                let mut b = [0u8; 32];
                b[..8].copy_from_slice(&i.to_le_bytes());
                b
            })
            .expect("canonical")
        };
        for i in 0..40u64 {
            let retention = if i == 7 {
                Retention::Marked
            } else {
                Retention::Ephemeral
            };
            tree.append(leaf(i), retention).expect("append");
            if i % 10 == 9 {
                tree.checkpoint(BlockHeight::from_u32(100 + i as u32))
                    .expect("checkpoint");
            }
        }
        let anchor = BlockHeight::from_u32(139);
        let witness = tree
            .witness_at_checkpoint_id(Position::from(7), &anchor)
            .expect("witness")
            .expect("present");
        let root = tree
            .root_at_checkpoint_id(&anchor)
            .expect("root")
            .expect("present");
        let snap = snapshot(&mut tree).expect("snapshot");
        assert_eq!(snap.checkpoints.len(), 4);
        let bytes = postcard::to_allocvec(&snap).expect("ser");
        let back: TreeSnapshot = postcard::from_bytes(&bytes).expect("de");
        let restored = restore(&back).expect("restore");
        assert_eq!(
            restored
                .witness_at_checkpoint_id(Position::from(7), &anchor)
                .expect("witness")
                .expect("present"),
            witness
        );
        assert_eq!(
            restored
                .root_at_checkpoint_id(&anchor)
                .expect("root")
                .expect("present"),
            root
        );
    }
}
