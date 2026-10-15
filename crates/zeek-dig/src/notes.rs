//! Listing a player's received notes with their memos.
//!
//! Compact-block syncing never sees memos, so the wallet database holds none.
//! This fetches each note's full transaction from lightwalletd and decrypts
//! the action with the player's own incoming viewing keys (external for
//! payments, internal for change), printing the memo text. Nothing is written.

use crate::NETWORK;
use orchard::keys::{PreparedIncomingViewingKey, Scope};
use orchard::note_encryption::{IronwoodDomain, OrchardDomain};
use orchard::ValuePool;
use zcash_client_backend::proto::service::TxFilter;
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::{Transaction, TxId};
use zcash_protocol::consensus::{BlockHeight, BranchId};
use zeek_vault::lightwalletd::Client;
use zeek_vault::wallet::NoteReport;

use crate::wallet::DigError;

/// A received note with its decrypted memo.
pub struct NoteWithMemo {
    pub txid: String,
    pub output_index: u16,
    pub value: u64,
    pub height: Option<u32>,
    pub scope: &'static str,
    /// Memo text, or a description of a non-text memo.
    pub memo: String,
}

/// Fetches and decrypts the memos of `notes`.
pub async fn notes_with_memos(
    client: &mut Client,
    usk: &UnifiedSpendingKey,
    notes: &[NoteReport],
) -> Result<Vec<NoteWithMemo>, DigError> {
    let fvk = usk.to_unified_full_viewing_key();
    let fvk = fvk
        .orchard()
        .ok_or_else(|| DigError::Keys("wallet has no Orchard key".into()))?;
    let ivks = [
        (
            "external",
            PreparedIncomingViewingKey::new(&fvk.to_ivk(Scope::External)),
        ),
        (
            "internal",
            PreparedIncomingViewingKey::new(&fvk.to_ivk(Scope::Internal)),
        ),
    ];
    let mut out = Vec::with_capacity(notes.len());
    for note in notes {
        let txid = TxId::from_hex(&note.txid).ok_or_else(|| {
            DigError::EngineResponse(format!("txid {} is not valid hex", note.txid))
        })?;
        let raw = client
            .get_transaction(TxFilter {
                block: None,
                index: 0,
                hash: txid.as_ref().to_vec(),
            })
            .await
            .map_err(zeek_vault::VaultError::from)?
            .into_inner();
        let height = note
            .mined_height
            .unwrap_or(u32::try_from(raw.height).unwrap_or(0));
        let branch = BranchId::for_height(&NETWORK, BlockHeight::from_u32(height));
        let tx = Transaction::read(&raw.data[..], branch)
            .map_err(|e| DigError::EngineResponse(format!("transaction {}: {e}", note.txid)))?;
        let (pool, bundle) = match note.pool {
            ValuePool::Ironwood => (ValuePool::Ironwood, tx.ironwood_bundle()),
            ValuePool::Orchard => (ValuePool::Orchard, tx.orchard_bundle()),
        };
        let action = bundle
            .and_then(|b| b.actions().get(usize::from(note.output_index)))
            .ok_or_else(|| {
                DigError::EngineResponse(format!(
                    "{}: action {} missing",
                    note.txid, note.output_index
                ))
            })?;
        let mut decrypted = None;
        for (scope, ivk) in &ivks {
            let result = match pool {
                ValuePool::Ironwood => {
                    try_note_decryption(&IronwoodDomain::for_action(action), ivk, action)
                }
                ValuePool::Orchard => {
                    try_note_decryption(&OrchardDomain::for_action(action), ivk, action)
                }
            };
            if let Some((_, _, memo)) = result {
                decrypted = Some((*scope, memo));
                break;
            }
        }
        let (scope, memo) = decrypted.ok_or_else(|| {
            DigError::EngineResponse(format!(
                "{}: action {} does not decrypt with this wallet",
                note.txid, note.output_index
            ))
        })?;
        out.push(NoteWithMemo {
            txid: note.txid.clone(),
            output_index: note.output_index,
            value: note.value.into_u64(),
            height: note.mined_height,
            scope,
            memo: describe_memo(&memo),
        });
    }
    Ok(out)
}

/// Decrypts every output of one transaction that belongs to the player
/// (spent or not), with its memo.
pub async fn notes_in_transaction(
    client: &mut Client,
    usk: &UnifiedSpendingKey,
    txid_hex: &str,
) -> Result<Vec<NoteWithMemo>, DigError> {
    let fvk = usk.to_unified_full_viewing_key();
    let fvk = fvk
        .orchard()
        .ok_or_else(|| DigError::Keys("wallet has no Orchard key".into()))?;
    let ivks = [
        (
            "external",
            PreparedIncomingViewingKey::new(&fvk.to_ivk(Scope::External)),
        ),
        (
            "internal",
            PreparedIncomingViewingKey::new(&fvk.to_ivk(Scope::Internal)),
        ),
    ];
    let txid = TxId::from_hex(txid_hex)
        .ok_or_else(|| DigError::EngineResponse(format!("txid {txid_hex} is not valid hex")))?;
    let raw = client
        .get_transaction(TxFilter {
            block: None,
            index: 0,
            hash: txid.as_ref().to_vec(),
        })
        .await
        .map_err(zeek_vault::VaultError::from)?
        .into_inner();
    let height = u32::try_from(raw.height).ok().filter(|h| *h > 0);
    let branch = BranchId::for_height(&NETWORK, BlockHeight::from_u32(height.unwrap_or(u32::MAX)));
    let tx = Transaction::read(&raw.data[..], branch)
        .map_err(|e| DigError::EngineResponse(format!("transaction {txid_hex}: {e}")))?;
    let mut out = Vec::new();
    for (pool, bundle) in [
        (ValuePool::Ironwood, tx.ironwood_bundle()),
        (ValuePool::Orchard, tx.orchard_bundle()),
    ] {
        let Some(bundle) = bundle else { continue };
        for (index, action) in bundle.actions().iter().enumerate() {
            for (scope, ivk) in &ivks {
                let result = match pool {
                    ValuePool::Ironwood => {
                        try_note_decryption(&IronwoodDomain::for_action(action), ivk, action)
                    }
                    ValuePool::Orchard => {
                        try_note_decryption(&OrchardDomain::for_action(action), ivk, action)
                    }
                };
                if let Some((note, _, memo)) = result {
                    out.push(NoteWithMemo {
                        txid: txid_hex.to_owned(),
                        output_index: u16::try_from(index).unwrap_or(u16::MAX),
                        value: note.value().inner(),
                        height,
                        scope,
                        memo: describe_memo(&memo),
                    });
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Renders a ZIP 302 memo for display.
pub fn describe_memo(memo: &[u8; 512]) -> String {
    let trimmed = {
        let end = memo
            .iter()
            .rposition(|b| *b != 0)
            .map(|i| i + 1)
            .unwrap_or(0);
        &memo[..end]
    };
    match trimmed.first() {
        None => "(empty)".to_owned(),
        Some(0xF6) if trimmed.len() == 1 => "(empty)".to_owned(),
        Some(b) if *b <= 0xF4 => match std::str::from_utf8(trimmed) {
            Ok(text) => format!("text: {text}"),
            Err(_) => format!("invalid UTF-8 text memo ({} bytes)", trimmed.len()),
        },
        Some(0xFF) => format!("arbitrary data ({} bytes)", trimmed.len()),
        Some(b) => format!("reserved memo type {b:#04x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_descriptions() {
        let mut m = [0u8; 512];
        assert_eq!(describe_memo(&m), "(empty)");
        m[0] = 0xF6;
        assert_eq!(describe_memo(&m), "(empty)");
        let mut t = [0u8; 512];
        t[..5].copy_from_slice(b"hello");
        assert_eq!(describe_memo(&t), "text: hello");
        let mut a = [0u8; 512];
        a[0] = 0xFF;
        a[1] = 1;
        assert_eq!(describe_memo(&a), "arbitrary data (2 bytes)");
    }
}
