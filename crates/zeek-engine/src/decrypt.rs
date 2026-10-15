//! Dig detection: trial decryption with the vault's external-scope IVK.
//!
//! Compact blocks carry, per action, only the nullifier, note commitment,
//! ephemeral key and the first 52 ciphertext bytes, enough to detect that an
//! output is ours and to read its value, but not the memo. A hit is therefore
//! completed by fetching the full transaction and decrypting the whole action,
//! which yields the 512-byte memo.
//!
//! Ironwood outputs use the Ironwood note-encryption domain (ZIP 2005, note
//! plaintext lead byte `0x03`); legacy Orchard outputs use the Orchard domain.
//! Both are trial-decrypted with the same Orchard incoming viewing key.

use orchard::keys::PreparedIncomingViewingKey;
use orchard::note_encryption::{CompactAction, IronwoodDomain, OrchardDomain};
use zcash_client_backend::proto::compact_formats::{CompactOrchardAction, CompactTx};
use zcash_note_encryption::{try_compact_note_decryption, try_note_decryption};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;

use crate::db::Pool;
use crate::error::EngineError;

/// An output of a compact transaction that decrypts to the vault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactHit {
    pub pool: Pool,
    pub action_index: u16,
    /// Note value from the compact plaintext.
    pub value: u64,
    /// Extracted note commitment of the output.
    pub cmx: [u8; 32],
}

/// A fully decrypted output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullNote {
    pub pool: Pool,
    pub action_index: u16,
    pub value: u64,
    pub memo: [u8; 512],
    /// Extracted note commitment of the output.
    pub cmx: [u8; 32],
}

/// Trial-decrypts outputs with one prepared incoming viewing key.
pub struct DigScanner {
    ivk: PreparedIncomingViewingKey,
}

impl DigScanner {
    /// Wraps the vault's external-scope IVK.
    pub fn new(ivk: PreparedIncomingViewingKey) -> Self {
        DigScanner { ivk }
    }

    /// Finds the outputs of `tx` that pay the vault.
    pub fn scan_compact_tx(
        &self,
        height: u32,
        tx: &CompactTx,
    ) -> Result<Vec<CompactHit>, EngineError> {
        let mut hits = Vec::new();
        self.scan_actions(height, Pool::Ironwood, &tx.ironwood_actions, &mut hits)?;
        self.scan_actions(height, Pool::Orchard, &tx.actions, &mut hits)?;
        Ok(hits)
    }

    fn scan_actions(
        &self,
        height: u32,
        pool: Pool,
        actions: &[CompactOrchardAction],
        hits: &mut Vec<CompactHit>,
    ) -> Result<(), EngineError> {
        for (index, action) in actions.iter().enumerate() {
            let compact =
                CompactAction::try_from(action).map_err(|e| EngineError::MalformedBlock {
                    height,
                    reason: format!("compact action {index}: {e:?}"),
                })?;
            let action_index = u16::try_from(index).map_err(|_| EngineError::MalformedBlock {
                height,
                reason: "more than 65535 actions".into(),
            })?;
            let decrypted = match pool {
                Pool::Ironwood => try_compact_note_decryption(
                    &IronwoodDomain::for_compact_action(&compact),
                    &self.ivk,
                    &compact,
                ),
                Pool::Orchard => try_compact_note_decryption(
                    &OrchardDomain::for_compact_action(&compact),
                    &self.ivk,
                    &compact,
                ),
            };
            if let Some((note, _address)) = decrypted {
                let cmx = <[u8; 32]>::try_from(action.cmx.as_slice()).map_err(|_| {
                    EngineError::MalformedBlock {
                        height,
                        reason: format!("compact action {index}: cmx is not 32 bytes"),
                    }
                })?;
                hits.push(CompactHit {
                    pool,
                    action_index,
                    value: note.value().inner(),
                    cmx,
                });
            }
        }
        Ok(())
    }

    /// The extracted note commitment of action `index` of the given pool in
    /// the full transaction `raw` (mined under `branch`), for backfilling
    /// notes stored before commitments were recorded.
    pub fn commitment_of(
        txid_hex: &str,
        raw: &[u8],
        branch: BranchId,
        pool: Pool,
        index: u16,
    ) -> Result<[u8; 32], EngineError> {
        let tx = Transaction::read(raw, branch).map_err(|e| EngineError::MalformedTransaction {
            txid: txid_hex.to_owned(),
            reason: e.to_string(),
        })?;
        let bundle = match pool {
            Pool::Ironwood => tx.ironwood_bundle(),
            Pool::Orchard => tx.orchard_bundle(),
        }
        .ok_or_else(|| EngineError::MalformedTransaction {
            txid: txid_hex.to_owned(),
            reason: format!("no {pool:?} bundle"),
        })?;
        let action = bundle.actions().get(usize::from(index)).ok_or_else(|| {
            EngineError::MalformedTransaction {
                txid: txid_hex.to_owned(),
                reason: format!("action {index} missing"),
            }
        })?;
        Ok(action.cmx().to_bytes())
    }

    /// Trial-decrypts every Ironwood and legacy Orchard action of the full
    /// transaction `raw` (parsed under `branch`), with no compact hits to
    /// guide it: used for mempool transactions, which have no compact form.
    /// Returns the txid (internal byte order) and the outputs that pay the
    /// vault, Ironwood first, each pool in action order.
    pub fn decrypt_all(
        &self,
        raw: &[u8],
        branch: BranchId,
    ) -> Result<([u8; 32], Vec<FullNote>), EngineError> {
        let tx = Transaction::read(raw, branch).map_err(|e| EngineError::MalformedTransaction {
            txid: "(unparsed)".to_owned(),
            reason: e.to_string(),
        })?;
        let txid: [u8; 32] = *tx.txid().as_ref();
        let mut notes = Vec::new();
        for pool in [Pool::Ironwood, Pool::Orchard] {
            let bundle = match pool {
                Pool::Ironwood => tx.ironwood_bundle(),
                Pool::Orchard => tx.orchard_bundle(),
            };
            let Some(bundle) = bundle else {
                continue;
            };
            for (index, action) in bundle.actions().iter().enumerate() {
                let decrypted = match pool {
                    Pool::Ironwood => {
                        try_note_decryption(&IronwoodDomain::for_action(action), &self.ivk, action)
                    }
                    Pool::Orchard => {
                        try_note_decryption(&OrchardDomain::for_action(action), &self.ivk, action)
                    }
                };
                let Some((note, _address, memo)) = decrypted else {
                    continue;
                };
                let action_index =
                    u16::try_from(index).map_err(|_| EngineError::MalformedTransaction {
                        txid: hex::encode(txid),
                        reason: "more than 65535 actions".into(),
                    })?;
                notes.push(FullNote {
                    pool,
                    action_index,
                    value: note.value().inner(),
                    memo,
                    cmx: action.cmx().to_bytes(),
                });
            }
        }
        Ok((txid, notes))
    }

    /// Decrypts the hit outputs of the full transaction `raw` (mined under
    /// `branch`), returning value and memo for each.
    pub fn decrypt_full(
        &self,
        txid_hex: &str,
        raw: &[u8],
        branch: BranchId,
        hits: &[CompactHit],
    ) -> Result<Vec<FullNote>, EngineError> {
        let tx = Transaction::read(raw, branch).map_err(|e| EngineError::MalformedTransaction {
            txid: txid_hex.to_owned(),
            reason: e.to_string(),
        })?;
        let mut notes = Vec::with_capacity(hits.len());
        for hit in hits {
            let bundle = match hit.pool {
                Pool::Ironwood => tx.ironwood_bundle(),
                Pool::Orchard => tx.orchard_bundle(),
            }
            .ok_or_else(|| EngineError::MalformedTransaction {
                txid: txid_hex.to_owned(),
                reason: format!("no {:?} bundle for a detected output", hit.pool),
            })?;
            let action = bundle
                .actions()
                .get(usize::from(hit.action_index))
                .ok_or_else(|| EngineError::MalformedTransaction {
                    txid: txid_hex.to_owned(),
                    reason: format!("action {} missing", hit.action_index),
                })?;
            let decrypted = match hit.pool {
                Pool::Ironwood => {
                    try_note_decryption(&IronwoodDomain::for_action(action), &self.ivk, action)
                }
                Pool::Orchard => {
                    try_note_decryption(&OrchardDomain::for_action(action), &self.ivk, action)
                }
            };
            let (note, _address, memo) =
                decrypted.ok_or_else(|| EngineError::MalformedTransaction {
                    txid: txid_hex.to_owned(),
                    reason: format!(
                        "action {} decrypted in the compact block but not in the full transaction",
                        hit.action_index
                    ),
                })?;
            if note.value().inner() != hit.value {
                return Err(EngineError::MalformedTransaction {
                    txid: txid_hex.to_owned(),
                    reason: "value differs between compact and full decryption".into(),
                });
            }
            let cmx = action.cmx().to_bytes();
            if cmx != hit.cmx {
                return Err(EngineError::MalformedTransaction {
                    txid: txid_hex.to_owned(),
                    reason: "note commitment differs between the compact block and the transaction"
                        .into(),
                });
            }
            notes.push(FullNote {
                pool: hit.pool,
                action_index: hit.action_index,
                value: hit.value,
                memo,
                cmx,
            });
        }
        Ok(notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::{SyntheticChain, TestVault};
    use zeek_core::memo::DigMemo;

    #[test]
    fn external_ivk_sees_payments_but_not_change() {
        let vault = TestVault::new(3);
        let mut chain = SyntheticChain::new(&vault, 100, 1_700_000_000);
        let memo = vault.dig_memo(0, 4, 2);
        chain.push_block(vec![
            chain.payment(250_000, Pool::Ironwood, memo.encode()),
            chain.change(50_000),
            chain.payment(70_000, Pool::Orchard, [0xF6; 512]),
        ]);
        let block = chain.block(100).expect("block");
        let scanner = DigScanner::new(vault.config(100).incoming_viewing_key().expect("ivk"));

        let mut all = Vec::new();
        for (i, tx) in block.vtx.iter().enumerate() {
            let hits = scanner.scan_compact_tx(100, tx).expect("scan");
            for hit in &hits {
                let raw = chain.transaction(&tx.txid).expect("raw");
                let notes = scanner
                    .decrypt_full("tx", &raw, BranchId::Nu6_3, std::slice::from_ref(hit))
                    .expect("full");
                all.push((i, notes));
            }
        }
        assert_eq!(
            all.len(),
            2,
            "payment and the Orchard-pool payment, not the change"
        );
        let (_, ironwood) = &all[0];
        assert_eq!(ironwood[0].pool, Pool::Ironwood);
        assert_eq!(ironwood[0].value, 250_000);
        assert_eq!(
            ironwood[0].cmx,
            <[u8; 32]>::try_from(block.vtx[0].ironwood_actions[0].cmx.as_slice()).expect("cmx"),
            "the recorded commitment is the compact block's"
        );
        assert_eq!(DigMemo::decode(&ironwood[0].memo).expect("memo"), memo);
        let (_, orchard) = &all[1];
        assert_eq!(orchard[0].pool, Pool::Orchard);
        assert_eq!(orchard[0].value, 70_000);
        assert_eq!(orchard[0].memo[0], 0xF6);
    }
}
