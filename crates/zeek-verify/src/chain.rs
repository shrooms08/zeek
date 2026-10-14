//! Chain data handed to the checks.

use std::collections::BTreeMap;

use prost::Message;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, MAIN_NETWORK, TEST_NETWORK};
use zeek_core::params::Network;

/// Errors in chain data.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChainDataError {
    #[error("compact block data is malformed: {0}")]
    BadBlocks(String),
    #[error("transaction {txid} does not parse: {reason}")]
    BadTransaction { txid: String, reason: String },
    #[error("block {0} is missing from the chain data")]
    MissingBlock(u32),
}

/// Consensus branch at `height`.
pub fn branch_for(network: Network, height: u32) -> BranchId {
    let h = BlockHeight::from_u32(height);
    match network {
        Network::Main => BranchId::for_height(&MAIN_NETWORK, h),
        Network::Test => BranchId::for_height(&TEST_NETWORK, h),
    }
}

/// Parses blocks as the Zeek relay serves them: per block a `u32`
/// little-endian length and the `CompactBlock` protobuf.
pub fn decode_relay_blocks(bytes: &[u8]) -> Result<Vec<CompactBlock>, ChainDataError> {
    let mut rest = bytes;
    let mut blocks = Vec::new();
    while !rest.is_empty() {
        let len_bytes: [u8; 4] = rest
            .get(..4)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| ChainDataError::BadBlocks("truncated length prefix".into()))?;
        let len = usize::try_from(u32::from_le_bytes(len_bytes))
            .map_err(|_| ChainDataError::BadBlocks("length overflows".into()))?;
        let body = rest
            .get(4..4 + len)
            .ok_or_else(|| ChainDataError::BadBlocks("truncated block".into()))?;
        blocks.push(
            CompactBlock::decode(body).map_err(|e| ChainDataError::BadBlocks(e.to_string()))?,
        );
        rest = &rest[4 + len..];
    }
    Ok(blocks)
}

/// Compact blocks by height.
#[derive(Clone, Debug, Default)]
pub struct Blocks(BTreeMap<u32, CompactBlock>);

impl Blocks {
    pub fn new(blocks: impl IntoIterator<Item = CompactBlock>) -> Self {
        Blocks(
            blocks
                .into_iter()
                .filter_map(|b| u32::try_from(b.height).ok().map(|h| (h, b)))
                .collect(),
        )
    }

    pub fn get(&self, height: u32) -> Result<&CompactBlock, ChainDataError> {
        self.0
            .get(&height)
            .ok_or(ChainDataError::MissingBlock(height))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &CompactBlock)> {
        self.0.iter()
    }

    /// `nTime` of the block at `height`.
    pub fn time(&self, height: u32) -> Result<u64, ChainDataError> {
        Ok(u64::from(self.get(height)?.time))
    }

    /// Block hash (display order hex) at `height`.
    pub fn hash_hex(&self, height: u32) -> Result<String, ChainDataError> {
        let mut hash = self.get(height)?.hash.clone();
        hash.reverse();
        Ok(hex::encode(hash))
    }

    /// Whether the compact block at `height` lists `txid` (internal order).
    pub fn contains_tx(&self, height: u32, txid: &[u8; 32]) -> bool {
        self.0
            .get(&height)
            .is_some_and(|b| b.vtx.iter().any(|t| t.txid == txid[..]))
    }

    /// Checks that each block's parent hash is the previous block's hash.
    pub fn linked(&self) -> Result<(), String> {
        let mut previous: Option<(u32, &[u8])> = None;
        for (height, block) in &self.0 {
            if let Some((ph, hash)) = previous {
                if ph + 1 == *height && block.prev_hash != hash {
                    return Err(format!("block {height} does not extend block {ph}"));
                }
            }
            previous = Some((*height, &block.hash));
        }
        Ok(())
    }
}

/// A full transaction and the height it was mined at.
#[derive(Clone, Debug)]
pub struct MinedTx {
    pub raw: Vec<u8>,
    pub height: u32,
}

/// Parses a mined transaction.
pub fn parse_tx(network: Network, txid: &str, tx: &MinedTx) -> Result<Transaction, ChainDataError> {
    Transaction::read(&tx.raw[..], branch_for(network, tx.height)).map_err(|e| {
        ChainDataError::BadTransaction {
            txid: txid.to_owned(),
            reason: e.to_string(),
        }
    })
}

/// Display-order hex txid to internal byte order.
pub fn txid_bytes(display_hex: &str) -> Option<[u8; 32]> {
    let mut bytes: [u8; 32] = hex::decode(display_hex).ok()?.try_into().ok()?;
    bytes.reverse();
    Some(bytes)
}
