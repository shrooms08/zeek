//! Building and sending a dig.
//!
//! The memo and the ZIP 321 request come from `zeek-core`; the spend goes
//! through the same Ironwood PCZT pipeline as the vault (propose, prove,
//! sign, extract, broadcast), signed with the player's own spend authorizing
//! key instead of FROST.
//!
//! # Payout receivers
//!
//! Every dig seals a **fresh diversified address** of the player's wallet:
//! one new diversifier index per tile, taken from the wallet's own
//! next-address allocator, which persists each index as an exposed address
//! in its database. No receiver is ever reused across digs, so the
//! keyholders, who open the sealed receivers of the payouts they sign,
//! cannot link one player's digs to each other. Detection of the payouts
//! does not depend on the recorded indices: the wallet trial-decrypts every
//! output with its incoming viewing key, which covers all diversifiers, and
//! recovers the index from the recipient when a note arrives.

use crate::NETWORK;
use orchard::circuit::{ProvingKey, VerifyingKey};
use orchard::keys::SpendAuthorizingKey;
use orchard::ValuePool;
use pczt::roles::signer::Signer;
use pczt::Pczt;
use rand_09::TryRngCore;
use serde::Deserialize;
use zcash_client_backend::data_api::WalletWrite;
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_protocol::consensus::BlockHeight;
use zeek_core::memo::{DigMemo, PayoutReceiver};
use zeek_core::params::{GameParams, Network};
use zeek_core::payment;
use zeek_core::seal::SealingPublicKey;
use zeek_vault::lightwalletd::Client;
use zeek_vault::send::{self, PreparedPczt};
use zeek_vault::wallet::{self, Db};
use zeek_vault::VaultError;

use crate::wallet::{DigError, Player};

/// Blocks that must remain in the window for a dig to be sent.
pub const SAFETY_MARGIN_BLOCKS: u32 = 3;

/// The subset of `GET /v1/config` the player needs.
#[derive(Clone, Debug, Deserialize)]
pub struct EngineConfig {
    pub params: GameParams,
    pub vault_address: String,
    pub sealing_public_key: SealingPublicKey,
}

/// The subset of `GET /v1/board` the player needs.
#[derive(Clone, Debug, Deserialize)]
pub struct EngineBoard {
    pub tip_height: u32,
    pub round: u32,
    pub blocks_remaining: u32,
    pub status: String,
}

/// Fetches config and board from the engine.
pub async fn fetch_engine(url: &str) -> Result<(EngineConfig, EngineBoard), DigError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| DigError::Engine(e.to_string()))?;
    let base = url.trim_end_matches('/');
    let config: EngineConfig = client
        .get(format!("{base}/v1/config"))
        .send()
        .await
        .map_err(|e| DigError::Engine(e.to_string()))?
        .error_for_status()
        .map_err(|e| DigError::Engine(e.to_string()))?
        .json()
        .await
        .map_err(|e| DigError::EngineResponse(e.to_string()))?;
    let board: EngineBoard = client
        .get(format!("{base}/v1/board"))
        .send()
        .await
        .map_err(|e| DigError::Engine(e.to_string()))?
        .error_for_status()
        .map_err(|e| DigError::Engine(e.to_string()))?
        .json()
        .await
        .map_err(|e| DigError::EngineResponse(e.to_string()))?;
    Ok((config, board))
}

/// Which round the memo names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundChoice {
    /// The board's current round.
    Auto,
    /// An explicit round (may differ from the board's: the dig is refunded).
    Explicit(u32),
}

/// Decides the memo round and enforces the safety margin.
pub fn choose_round(board: &EngineBoard, choice: RoundChoice) -> Result<u32, DigError> {
    if board.blocks_remaining < SAFETY_MARGIN_BLOCKS + 1 {
        return Err(DigError::ClosingTooSoon {
            round: board.round,
            blocks_remaining: board.blocks_remaining,
            margin: SAFETY_MARGIN_BLOCKS,
        });
    }
    Ok(match choice {
        RoundChoice::Auto => board.round,
        RoundChoice::Explicit(round) => round,
    })
}

/// What the player asked for.
///
/// Several tiles go into **one** transaction with one vault output per tile,
/// each carrying its own sealed memo. Anyone holding the public UIVK sees
/// that those outputs share a txid, so digs sent together are linkable to
/// each other (not to the player); sending tiles separately avoids that at
/// the cost of one fee per transaction. The choice is the player's.
#[derive(Clone, Debug)]
pub struct DigRequest {
    pub choice: RoundChoice,
    /// Tiles to dig, each with `amount_zec`.
    pub tiles: Vec<u8>,
    pub amount_zec: String,
}

/// What a dig produced.
pub struct DigOutcome {
    pub txid: String,
    pub memo_round: u32,
    pub tiles: Vec<u8>,
    /// Amount per tile.
    pub amount_zat: u64,
    pub fee_zat: u64,
    pub target_height: u32,
    /// Diversifier index of the fresh payout receiver sealed for each tile
    /// (player-local bookkeeping; the receivers themselves are never shown).
    pub diversifier_indices: Vec<u128>,
}

/// Builds, signs and broadcasts a dig from `player`.
pub async fn dig(
    player: &Player,
    db: &mut Db,
    client: &mut Client,
    config: &EngineConfig,
    board: &EngineBoard,
    request: &DigRequest,
) -> Result<DigOutcome, DigError> {
    if config.params.network != Network::Test {
        return Err(DigError::Game("this tool plays on testnet only".into()));
    }
    if request.tiles.is_empty() {
        return Err(DigError::Game("at least one tile is required".into()));
    }
    let memo_round = choose_round(board, request.choice)?;
    let amount = send::parse_zec(&request.amount_zec)?;
    if amount.into_u64() < config.params.min_dig_zat {
        return Err(DigError::Game(format!(
            "amount is below the minimum dig of {} zatoshis",
            config.params.min_dig_zat
        )));
    }

    let usk = player.spending_key()?;
    let account = player
        .find_account(db)?
        .ok_or_else(|| DigError::PlayerMissing(player.name().to_owned()))?;
    wallet::sync(db, client).await?;
    let report = wallet::balance(db, account)?;

    // One fresh diversified payout receiver per tile, sealed to the vault's key.
    let mut rng = rand_09::rngs::OsRng.unwrap_err();
    let vault = zcash_address::ZcashAddress::try_from_encoded(&config.vault_address)
        .map_err(|e| DigError::Game(format!("vault address: {e}")))?;
    let mut payments = Vec::with_capacity(request.tiles.len());
    let mut diversifier_indices = Vec::with_capacity(request.tiles.len());
    let mut used = std::collections::BTreeSet::new();
    for tile in &request.tiles {
        let (address, index) = db
            .get_next_available_address(account, UnifiedAddressRequest::ORCHARD)
            .map_err(|e| DigError::Keys(format!("next payout address: {e}")))?
            .ok_or_else(|| DigError::Keys("the wallet has no account for this player".into()))?;
        let mut index_bytes = [0u8; 16];
        index_bytes[..11].copy_from_slice(index.as_bytes());
        let index_value = u128::from_le_bytes(index_bytes);
        if !used.insert(index_value) {
            return Err(DigError::Keys(
                "the wallet handed out the same diversifier index twice".into(),
            ));
        }
        diversifier_indices.push(index_value);
        let receiver =
            PayoutReceiver::from_unified_address(Network::Test, &address.encode(&NETWORK))
                .map_err(|e| DigError::Game(e.to_string()))?;
        let memo = DigMemo::seal(
            memo_round,
            *tile,
            &receiver,
            &config.sealing_public_key,
            &mut rng,
        )
        .map_err(|e| DigError::Game(e.to_string()))?;
        // One payment per tile; the single-tile form is checked against the
        // ZIP 321 URI builder so both paths encode the memo identically.
        if request.tiles.len() == 1 {
            payment::dig_payment_uri(
                Network::Test,
                &config.vault_address,
                amount.into_u64(),
                &memo,
            )
            .map_err(|e| DigError::Game(e.to_string()))?;
        }
        let memo_bytes = zcash_protocol::memo::MemoBytes::from_bytes(&memo.encode_unpadded())
            .map_err(|e| DigError::Game(format!("memo: {e}")))?;
        payments.push(
            zip321::Payment::new(
                vault.clone(),
                Some(amount),
                Some(memo_bytes),
                None,
                None,
                vec![],
            )
            .map_err(|e| DigError::Game(format!("payment: {e}")))?,
        );
    }
    let request_tx = zip321::TransactionRequest::new(payments)
        .map_err(|e| DigError::Game(format!("payment request: {e}")))?;

    let expected_target = BlockHeight::from_u32(report.chain_tip) + 1;
    let circuit = send::circuit_version_for(&NETWORK, expected_target)?;
    tracing::info!("building Orchard proving key for {circuit:?}");
    let proving_key = ProvingKey::build(circuit);
    let verifying_key = VerifyingKey::build(circuit);

    let PreparedPczt {
        pczt,
        target_height,
        fee,
    } = send::prepare_pczt(db, account, request_tx, &report)?;
    if send::circuit_version_for(&NETWORK, target_height)? != circuit {
        return Err(VaultError::CircuitMismatch {
            built: format!("{circuit:?}"),
            required: format!("{:?}", send::circuit_version_for(&NETWORK, target_height)?),
        }
        .into());
    }
    let pczt = send::prove(pczt, &proving_key)?;
    let pczt = sign_with_key(pczt, &usk)?;
    let transaction =
        send::extract_broadcast_store(db, client, pczt, &verifying_key, target_height).await?;
    Ok(DigOutcome {
        txid: transaction.txid().to_string(),
        memo_round,
        tiles: request.tiles.clone(),
        amount_zat: amount.into_u64(),
        fee_zat: fee.into_u64(),
        target_height: target_height.into(),
        diversifier_indices,
    })
}

/// What a plain payment produced.
pub struct PaymentOutcome {
    pub txid: String,
    pub payments: usize,
    pub total_zat: u64,
    pub fee_zat: u64,
}

/// Pays a ZIP 321 request exactly as an external wallet would: every payment
/// with its amount and its memo bytes as given (text memos stay text).
pub async fn pay_request(
    player: &Player,
    db: &mut Db,
    client: &mut Client,
    request: zip321::TransactionRequest,
) -> Result<PaymentOutcome, DigError> {
    let payments = request.payments().len();
    let total_zat = request
        .total()
        .map_err(|e| DigError::Game(format!("request total: {e:?}")))?
        .ok_or_else(|| DigError::Game("every payment in the request needs an amount".into()))?
        .into_u64();
    let usk = player.spending_key()?;
    let account = player
        .find_account(db)?
        .ok_or_else(|| DigError::PlayerMissing(player.name().to_owned()))?;
    wallet::sync(db, client).await?;
    let report = wallet::balance(db, account)?;
    let expected_target = BlockHeight::from_u32(report.chain_tip) + 1;
    let circuit = send::circuit_version_for(&NETWORK, expected_target)?;
    tracing::info!("building Orchard proving key for {circuit:?}");
    let proving_key = ProvingKey::build(circuit);
    let verifying_key = VerifyingKey::build(circuit);
    let PreparedPczt {
        pczt,
        target_height,
        fee,
    } = send::prepare_pczt(db, account, request, &report)?;
    if send::circuit_version_for(&NETWORK, target_height)? != circuit {
        return Err(VaultError::CircuitMismatch {
            built: format!("{circuit:?}"),
            required: format!("{:?}", send::circuit_version_for(&NETWORK, target_height)?),
        }
        .into());
    }
    let pczt = send::prove(pczt, &proving_key)?;
    let pczt = sign_with_key(pczt, &usk)?;
    let transaction =
        send::extract_broadcast_store(db, client, pczt, &verifying_key, target_height).await?;
    Ok(PaymentOutcome {
        txid: transaction.txid().to_string(),
        payments,
        total_zat,
        fee_zat: fee.into_u64(),
    })
}

/// Signs every pending spend with the player's Orchard spend authorizing key.
fn sign_with_key(pczt: Pczt, usk: &UnifiedSpendingKey) -> Result<Pczt, DigError> {
    let ask = SpendAuthorizingKey::from(usk.orchard());
    let pending = send::pending_spends(&pczt)?;
    let mut signer =
        Signer::new(pczt).map_err(|e| VaultError::Pczt(format!("signer init: {e:?}")))?;
    for spend in &pending {
        match spend.pool {
            ValuePool::Ironwood => signer.sign_ironwood(spend.index, &ask),
            ValuePool::Orchard => signer.sign_orchard(spend.index, &ask),
        }
        .map_err(|e| VaultError::Pczt(format!("sign action {}: {e:?}", spend.index)))?;
    }
    Ok(signer.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(blocks_remaining: u32) -> EngineBoard {
        EngineBoard {
            tip_height: 100,
            round: 4,
            blocks_remaining,
            status: "open".into(),
        }
    }

    #[test]
    fn safety_margin_is_enforced() {
        assert_eq!(choose_round(&board(10), RoundChoice::Auto).expect("ok"), 4);
        assert_eq!(
            choose_round(&board(4), RoundChoice::Explicit(9)).expect("ok"),
            9
        );
        assert!(matches!(
            choose_round(&board(3), RoundChoice::Auto),
            Err(DigError::ClosingTooSoon {
                round: 4,
                blocks_remaining: 3,
                margin: 3
            })
        ));
        assert!(matches!(
            choose_round(&board(0), RoundChoice::Auto),
            Err(DigError::ClosingTooSoon { .. })
        ));
    }
}
