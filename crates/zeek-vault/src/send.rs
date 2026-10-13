//! Spending from the vault: proposal → PCZT → proof → FROST signatures →
//! extraction → broadcast.
//!
//! The PCZT roles involved, in order:
//!
//! 1. **Creator / Constructor / IO Finalizer** — run inside
//!    `zcash_client_backend::data_api::wallet::create_pczt_from_proposal`. The
//!    IO finalizer signs the dummy spends and leaves every real spend with its
//!    randomizer `alpha` and no `spend_auth_sig`.
//! 2. **Prover** — [`pczt::roles::prover::Prover::create_orchard_proof`] with
//!    a proving key for the circuit version of the current consensus branch.
//! 3. **Signer** — this module. For each unsigned Orchard action it runs the
//!    FROST rounds with randomizer `alpha`, checks that the randomized group
//!    key equals the action's `rk`, and applies the signature through
//!    [`pczt::roles::signer::Signer::apply_orchard_signature`], which verifies
//!    it a second time before storing it.
//! 4. **Spend Finalizer / Transaction Extractor** — produce the final
//!    transaction, which is broadcast and recorded in the wallet.

use std::convert::Infallible;

use orchard::circuit::{OrchardCircuitVersion, ProvingKey, VerifyingKey};
use orchard::primitives::redpallas::{self, SpendAuth};
use pasta_curves::pallas;
use pczt::roles::prover::Prover;
use pczt::roles::signer::Signer;
use pczt::roles::tx_extractor::TransactionExtractor;
use pczt::roles::verifier::Verifier;
use pczt::Pczt;
use rand_core::OsRng;
use zcash_address::ZcashAddress;
use zcash_client_backend::data_api::error::Error as WalletApiError;
use zcash_client_backend::data_api::wallet::input_selection::{
    GreedyInputSelector, GreedyInputSelectorError, SpendPolicy,
};
use zcash_client_backend::data_api::wallet::{
    create_pczt_from_proposal, extract_and_store_transaction_from_pczt, propose_transfer,
    ConfirmationsPolicy,
};
use zcash_client_backend::data_api::WalletCommitmentTrees;
use zcash_client_backend::fees::zip317::SingleOutputChangeStrategy;
use zcash_client_backend::fees::DustOutputPolicy;
use zcash_client_backend::wallet::OvkPolicy;
use zcash_client_sqlite::{AccountUuid, ReceivedNoteId};
use zcash_keys::address::Address;
use zcash_primitives::transaction::builder::BundlePadding;
use zcash_primitives::transaction::fees::zip317;
use zcash_primitives::transaction::{Transaction, TxId};
use zcash_protocol::consensus::{BlockHeight, BranchId, OrchardProtocolRevision};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::{Zatoshis, COIN};
use zcash_protocol::{PoolType, ShieldedPool};
use zip321::{Payment, TransactionRequest};

use crate::error::VaultError;
use crate::frost::{self, KeyPackage, PublicKeyPackage};
use crate::lightwalletd::{self, Client};
use crate::wallet::{wallet_err, Db};
use crate::NETWORK;

/// What the operator asked for.
#[derive(Clone, Debug)]
pub struct SendRequest {
    /// Destination address (must contain an Orchard receiver).
    pub to: String,
    /// Amount in ZEC as a decimal string, e.g. `0.001`.
    pub amount_zec: String,
    /// Optional UTF-8 memo.
    pub memo: Option<String>,
}

/// Outcome of a successful send.
#[derive(Clone, Debug)]
pub struct SendOutcome {
    /// Transaction id as broadcast.
    pub txid: TxId,
    /// Number of Orchard spends that were FROST-signed.
    pub signed_actions: usize,
    /// Fee paid, in zatoshis.
    pub fee: Zatoshis,
}

/// Parses a decimal ZEC amount (up to 8 fractional digits) into zatoshis.
pub fn parse_zec(amount: &str) -> Result<Zatoshis, VaultError> {
    let invalid = || VaultError::InvalidAmount(amount.to_owned());
    let trimmed = amount.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.starts_with('+') {
        return Err(invalid());
    }
    let (whole, frac) = match trimmed.split_once('.') {
        Some((w, f)) => (w, f),
        None => (trimmed, ""),
    };
    if frac.len() > 8 || (whole.is_empty() && frac.is_empty()) {
        return Err(invalid());
    }
    let whole_value: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| invalid())?
    };
    let frac_value: u64 = if frac.is_empty() {
        0
    } else {
        let padded = format!("{frac:0<8}");
        padded.parse().map_err(|_| invalid())?
    };
    let zats = whole_value
        .checked_mul(COIN)
        .and_then(|w| w.checked_add(frac_value))
        .ok_or_else(invalid)?;
    if zats == 0 {
        return Err(invalid());
    }
    Zatoshis::from_u64(zats).map_err(|_| invalid())
}

/// Formats zatoshis as a ZEC decimal string.
pub fn format_zec(value: Zatoshis) -> String {
    let zats = value.into_u64();
    format!("{}.{:08}", zats / COIN, zats % COIN)
}

/// Parses and validates the destination address.
pub fn parse_destination(address: &str) -> Result<ZcashAddress, VaultError> {
    let parsed = ZcashAddress::try_from_encoded(address.trim())
        .map_err(|e| VaultError::InvalidAddress(format!("{address}: {e}")))?;
    // `try_from_zcash_address` rejects addresses for any other network.
    let checked = Address::try_from_zcash_address(&NETWORK, parsed)
        .map_err(|e| VaultError::InvalidAddress(format!("{address}: {e}")))?;
    let parsed = checked.to_zcash_address(&NETWORK);
    if !parsed.can_receive_as(PoolType::Shielded(ShieldedPool::Orchard)) {
        return Err(VaultError::NoOrchardReceiver);
    }
    Ok(parsed)
}

/// Encodes an optional text memo.
pub fn parse_memo(memo: Option<&str>) -> Result<Option<MemoBytes>, VaultError> {
    memo.map(|text| {
        MemoBytes::from_bytes(text.as_bytes()).map_err(|e| VaultError::InvalidMemo(e.to_string()))
    })
    .transpose()
}

/// Maps a consensus branch to the Orchard circuit it requires.
pub fn circuit_version_for(height: BlockHeight) -> Result<OrchardCircuitVersion, VaultError> {
    match BranchId::for_height(&NETWORK, height).orchard_protocol_revision() {
        Some(OrchardProtocolRevision::V2) => Ok(OrchardCircuitVersion::FixedPostNu6_2),
        Some(OrchardProtocolRevision::V3) => Ok(OrchardCircuitVersion::PostNu6_3),
        Some(OrchardProtocolRevision::InsecureV1) | None => Err(VaultError::UnsupportedBranch),
    }
}

/// An Orchard action awaiting a spend authorization signature.
#[derive(Clone, Debug)]
pub struct PendingSpend {
    /// Index of the action in the bundle.
    pub index: usize,
    /// Spend-auth randomizer `alpha`.
    pub alpha: pallas::Scalar,
    /// The randomized verification key the signature must verify under.
    pub rk: [u8; 32],
}

/// Lists the real (non-dummy) Orchard spends that still need a signature.
pub fn pending_spends(pczt: &Pczt) -> Result<Vec<PendingSpend>, VaultError> {
    // The raw `pczt::orchard::Spend` keeps `alpha` private; the Verifier role
    // parses the bundle into `orchard::pczt::Bundle`, whose actions expose it.
    let mut pending = Vec::new();
    Verifier::new(pczt.clone())
        .with_orchard::<Infallible, _>(|bundle| {
            for (index, action) in bundle.actions().iter().enumerate() {
                let spend = action.spend();
                if spend.spend_auth_sig().is_some() {
                    continue;
                }
                pending.push((
                    index,
                    spend.dummy_sk().is_some(),
                    *spend.alpha(),
                    spend.rk().clone(),
                ));
            }
            Ok(())
        })
        .map_err(|e| VaultError::Pczt(format!("parse Orchard bundle: {e:?}")))?;

    let mut spends = Vec::with_capacity(pending.len());
    for (index, has_dummy_sk, alpha, rk) in pending {
        if has_dummy_sk {
            return Err(VaultError::DummyKeyPresent { index });
        }
        let alpha = alpha.ok_or(VaultError::MissingAlpha { index })?;
        spends.push(PendingSpend {
            index,
            alpha,
            rk: <[u8; 32]>::from(&rk),
        });
    }
    if spends.is_empty() {
        return Err(VaultError::NothingToSign);
    }
    Ok(spends)
}

/// FROST-signs every pending Orchard spend in a proven PCZT.
///
/// Returns the signed PCZT and the number of actions signed. Each signature is
/// verified twice: once against the randomized group key inside
/// [`frost::randomized_sign`], and once more by the PCZT signer role against
/// the action's `rk`.
pub fn sign_pczt(
    pczt: Pczt,
    signers: &[&KeyPackage],
    public_key_package: &PublicKeyPackage,
) -> Result<(Pczt, usize), VaultError> {
    let pending = pending_spends(&pczt)?;
    let mut signer =
        Signer::new(pczt).map_err(|e| VaultError::Pczt(format!("signer init: {e:?}")))?;
    let sighash = signer.shielded_sighash();

    for spend in &pending {
        let produced =
            frost::randomized_sign(&sighash, &spend.alpha, signers, public_key_package, OsRng)?;
        if produced.randomized_verifying_key != spend.rk {
            return Err(VaultError::RkMismatch { index: spend.index });
        }
        let signature = redpallas::Signature::<SpendAuth>::from(produced.signature);
        signer
            .apply_orchard_signature(spend.index, signature)
            .map_err(|e| {
                VaultError::Pczt(format!("apply signature to action {}: {e:?}", spend.index))
            })?;
    }

    Ok((signer.finish(), pending.len()))
}

/// Builds the ZIP 317 change strategy used for every vault transaction.
fn change_strategy() -> SingleOutputChangeStrategy<zip317::FeeRule, Db> {
    SingleOutputChangeStrategy::new(
        zip317::FeeRule::standard(),
        None,
        ShieldedPool::Orchard,
        DustOutputPolicy::default(),
    )
}

/// Runs the full spend pipeline against the wallet and lightwalletd.
pub async fn send(
    db: &mut Db,
    client: &mut Client,
    account: AccountUuid,
    public_key_package: &PublicKeyPackage,
    signers: &[&KeyPackage],
    request: &SendRequest,
) -> Result<SendOutcome, VaultError> {
    let destination = parse_destination(&request.to)?;
    let amount = parse_zec(&request.amount_zec)?;
    let memo = parse_memo(request.memo.as_deref())?;

    let payment = Payment::new(destination, Some(amount), memo, None, None, vec![])
        .map_err(|e| VaultError::InvalidAddress(format!("payment: {e}")))?;
    let tx_request = TransactionRequest::new(vec![payment])
        .map_err(|e| VaultError::Build(format!("request: {e}")))?;

    tracing::info!("proposing transfer of {} ZEC", format_zec(amount));
    let proposal = propose_transfer::<_, _, _, _, <Db as WalletCommitmentTrees>::Error>(
        db,
        &NETWORK,
        account,
        &GreedyInputSelector::<Db>::new(),
        &change_strategy(),
        tx_request,
        ConfirmationsPolicy::default(),
        &SpendPolicy::shielded_pools([ShieldedPool::Orchard]),
        None,
        None,
    )
    .map_err(|e| match e {
        WalletApiError::InsufficientFunds {
            available,
            required,
        } => VaultError::insufficient(available, required),
        other => VaultError::Build(other.to_string()),
    })?;

    let fee = proposal
        .steps()
        .iter()
        .map(|step| step.balance().fee_required())
        .try_fold(Zatoshis::ZERO, |acc, f| acc + f)
        .ok_or_else(|| VaultError::Build("fee overflow".into()))?;
    let target_height = BlockHeight::from(proposal.min_target_height());
    tracing::info!(
        "proposal ready: fee {} ZEC, target height {}",
        format_zec(fee),
        target_height
    );

    let pczt = create_pczt_from_proposal::<_, _, GreedyInputSelectorError, _, zip317::FeeError, _>(
        db,
        &NETWORK,
        account,
        OvkPolicy::Sender,
        &proposal,
        None,
        BundlePadding::DEFAULT,
    )
    .map_err(|e| VaultError::Build(e.to_string()))?;

    let circuit = circuit_version_for(target_height)?;
    tracing::info!("building Orchard proving key for {circuit:?} (this takes a while)");
    let proving_key = ProvingKey::build(circuit);
    let verifying_key = VerifyingKey::build(circuit);

    let pczt = Prover::new(pczt)
        .create_orchard_proof(&proving_key)
        .map_err(|e| VaultError::Prove(format!("{e:?}")))?
        .finish();

    let (pczt, signed_actions) = sign_pczt(pczt, signers, public_key_package)?;
    tracing::info!("applied {signed_actions} FROST spend authorization signature(s)");

    let transaction = TransactionExtractor::new(pczt.clone())
        .with_orchard(&verifying_key)
        .extract()
        .map_err(|e| VaultError::Pczt(format!("extract: {e:?}")))?;
    let txid = transaction.txid();
    let raw = serialize(&transaction)?;

    lightwalletd::broadcast(client, raw).await?;
    tracing::info!("broadcast accepted: {txid}");

    let stored = extract_and_store_transaction_from_pczt::<_, ReceivedNoteId>(
        db,
        pczt,
        None,
        Some(&verifying_key),
    )
    .map_err(wallet_err)?;
    if stored != txid {
        return Err(VaultError::Wallet(format!(
            "stored txid {stored} differs from broadcast txid {txid}"
        )));
    }

    Ok(SendOutcome {
        txid,
        signed_actions,
        fee,
    })
}

fn serialize(transaction: &Transaction) -> Result<Vec<u8>, VaultError> {
    let mut bytes = Vec::new();
    transaction
        .write(&mut bytes)
        .map_err(|e| VaultError::Build(format!("serialize transaction: {e}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zec_amounts() {
        assert_eq!(parse_zec("1").expect("1").into_u64(), COIN);
        assert_eq!(parse_zec("0.001").expect("0.001").into_u64(), 100_000);
        assert_eq!(parse_zec(".5").expect(".5").into_u64(), 50_000_000);
        assert_eq!(
            parse_zec("2.00000001").expect("8 digits").into_u64(),
            200_000_001
        );
        for bad in ["", "0", "-1", "1.000000001", "abc", "1.2.3", "21000001"] {
            assert!(parse_zec(bad).is_err(), "{bad} should be rejected");
        }
        assert_eq!(
            format_zec(Zatoshis::const_from_u64(123_456_789)),
            "1.23456789"
        );
    }

    #[test]
    fn rejects_non_orchard_and_mainnet_destinations() {
        // Mainnet transparent address.
        assert!(matches!(
            parse_destination("t1KZ6HVupHe9ygeEGSS5V4EQ4tJZbWDYkCg"),
            Err(VaultError::InvalidAddress(_))
        ));
        // Testnet transparent address: right network, no Orchard receiver.
        assert!(matches!(
            parse_destination("tmEZhbWHTpdKMw5it8YDspUXSMGQyFwovpU"),
            Err(VaultError::NoOrchardReceiver)
        ));
        assert!(matches!(
            parse_destination("not-an-address"),
            Err(VaultError::InvalidAddress(_))
        ));
    }

    #[test]
    fn memo_length_is_enforced() {
        assert!(parse_memo(Some("hello zeek")).expect("ok").is_some());
        let long = "x".repeat(513);
        assert!(matches!(
            parse_memo(Some(&long)),
            Err(VaultError::InvalidMemo(_))
        ));
        assert!(parse_memo(None).expect("none").is_none());
    }

    #[test]
    fn circuit_version_follows_testnet_branches() {
        // NU6.2 activated on testnet at 4_052_000, NU6.3 at 4_134_000.
        assert!(matches!(
            circuit_version_for(BlockHeight::from_u32(4_000_000)),
            Err(VaultError::UnsupportedBranch)
        ));
        assert_eq!(
            circuit_version_for(BlockHeight::from_u32(4_100_000)).expect("v2"),
            OrchardCircuitVersion::FixedPostNu6_2
        );
        assert_eq!(
            circuit_version_for(BlockHeight::from_u32(4_400_000)).expect("v3"),
            OrchardCircuitVersion::PostNu6_3
        );
    }
}
