//! Spending from the vault: proposal → PCZT → proof → FROST signatures →
//! extraction → broadcast.
//!
//! Since NU6.3 the vault's funds live in the **Ironwood** pool, so every
//! transaction is a version 6 transaction (the version implied by the target
//! height; `propose_transfer` is told to pick it) carrying an Ironwood bundle.
//! Ironwood actions are Orchard-shaped: each spend is authorized by a
//! RedPallas signature under the randomized key `rk = ak + [alpha] G`, exactly
//! as for Orchard, so the FROST signing path is unchanged; only the bundle the
//! signatures are applied to differs.
//!
//! The PCZT roles involved, in order:
//!
//! 1. **Creator / Constructor / IO Finalizer** — run inside
//!    `zcash_client_backend::data_api::wallet::create_pczt_from_proposal`. The
//!    IO finalizer signs the protocol padding dummies (the ones that carry a
//!    `dummy_sk`) and leaves every other spend with its randomizer `alpha` and
//!    no `spend_auth_sig`. Besides the real spends that includes the
//!    wallet-controlled zero-value spends the builder pairs with change outputs
//!    in an Orchard-pool bundle under the post-NU6.3 cross-address rule; those
//!    are keyed by the vault's own `ak`, so they too must be FROST-signed.
//! 2. **Prover** — [`prove`]: the PCZT prover role proves the Orchard and the
//!    Ironwood bundle separately, both with the post-NU6.3 circuit.
//! 3. **Signer** — this module. For every spend in either bundle that still
//!    lacks a signature it runs the FROST rounds with that spend's `alpha`,
//!    checks that the randomized group key equals the action's `rk`, and
//!    applies the signature through the pool-specific
//!    [`pczt::roles::signer::Signer`] method, which verifies it a second time
//!    before storing it.
//! 4. **Spend Finalizer / Transaction Extractor** — produce the final
//!    transaction, which is checked to be V6 on the expected consensus branch,
//!    broadcast and recorded in the wallet.

use std::convert::Infallible;

use orchard::circuit::{OrchardCircuitVersion, ProvingKey, VerifyingKey};
use orchard::primitives::redpallas::{self, SpendAuth};
use orchard::ValuePool;
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
use zcash_client_backend::wallet::{Note, OvkPolicy};
use zcash_client_sqlite::{AccountUuid, ReceivedNoteId};
use zcash_keys::address::Address;
use zcash_primitives::transaction::builder::BundlePadding;
use zcash_primitives::transaction::fees::zip317;
use zcash_primitives::transaction::{Transaction, TxId, TxVersion};
use zcash_protocol::consensus::{BlockHeight, BranchId, OrchardProtocolRevision};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::{Zatoshis, COIN};
use zcash_protocol::{PoolType, ShieldedPool};
use zip321::{Payment, TransactionRequest};

use crate::error::VaultError;
use crate::frost::{self, KeyPackage, PublicKeyPackage};
use crate::lightwalletd::{self, Client};
use crate::wallet::{self, pool_name, wallet_err, Db};
use crate::NETWORK;

/// What the operator asked for.
#[derive(Clone, Debug)]
pub struct SendRequest {
    /// Destination address (must contain an Orchard-protocol receiver).
    pub to: String,
    /// Amount in ZEC as a decimal string, e.g. `0.001`.
    pub amount_zec: String,
    /// Optional UTF-8 memo.
    pub memo: Option<String>,
}

/// One spend that was FROST-signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedSpend {
    /// Bundle the action belongs to.
    pub pool: ValuePool,
    /// Index of the action in that bundle.
    pub index: usize,
    /// Value of the spent note, if the PCZT exposes it (`None` once redacted).
    pub value: Option<u64>,
}

/// Outcome of a successful send.
#[derive(Clone, Debug)]
pub struct SendOutcome {
    /// Transaction id as broadcast.
    pub txid: TxId,
    /// Transaction version that was broadcast.
    pub version: TxVersion,
    /// Consensus branch the transaction commits to.
    pub branch: BranchId,
    /// Height the proposal targeted.
    pub target_height: u32,
    /// Every spend that was FROST-signed, in signing order.
    pub signed_spends: Vec<SignedSpend>,
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
///
/// The Ironwood pool is addressed through the Orchard receiver of a unified
/// address (`zcash_keys` treats the two pools as sharing that receiver), so
/// the check is for an Orchard receiver.
pub fn parse_destination(address: &str) -> Result<ZcashAddress, VaultError> {
    let parsed = ZcashAddress::try_from_encoded(address.trim())
        .map_err(|e| VaultError::InvalidAddress(format!("{address}: {e}")))?;
    // `try_from_zcash_address` rejects addresses for any other network.
    let checked = Address::try_from_zcash_address(&NETWORK, parsed)
        .map_err(|e| VaultError::InvalidAddress(format!("{address}: {e}")))?;
    if !checked.can_receive_as(PoolType::IRONWOOD) {
        return Err(VaultError::NoOrchardReceiver);
    }
    Ok(checked.to_zcash_address(&NETWORK))
}

/// Encodes an optional text memo.
pub fn parse_memo(memo: Option<&str>) -> Result<Option<MemoBytes>, VaultError> {
    memo.map(|text| {
        MemoBytes::from_bytes(text.as_bytes()).map_err(|e| VaultError::InvalidMemo(e.to_string()))
    })
    .transpose()
}

/// Maps a consensus branch to the Orchard circuit it requires.
///
/// Both the Orchard and the Ironwood bundle of a post-NU6.3 transaction are
/// proven with the same circuit.
pub fn circuit_version_for(height: BlockHeight) -> Result<OrchardCircuitVersion, VaultError> {
    match BranchId::for_height(&NETWORK, height).orchard_protocol_revision() {
        Some(OrchardProtocolRevision::V2) => Ok(OrchardCircuitVersion::FixedPostNu6_2),
        Some(OrchardProtocolRevision::V3) => Ok(OrchardCircuitVersion::PostNu6_3),
        Some(OrchardProtocolRevision::InsecureV1) | None => Err(VaultError::UnsupportedBranch),
    }
}

/// Proves whichever Orchard-protocol bundles of the PCZT still lack a proof.
///
/// A V6 transaction carries separate Orchard and Ironwood bundles and the
/// PCZT prover role proves them separately; post-NU6.3 both use the same
/// proving key. Fails if a bundle that needs a proof still has none after
/// proving.
pub fn prove(pczt: Pczt, proving_key: &ProvingKey) -> Result<Pczt, VaultError> {
    let mut prover = Prover::new(pczt);
    if prover.requires_orchard_proof() {
        prover = prover
            .create_orchard_proof(proving_key)
            .map_err(|e| VaultError::Prove(format!("Orchard bundle: {e:?}")))?;
    }
    if prover.requires_ironwood_proof() {
        prover = prover
            .create_ironwood_proof(proving_key)
            .map_err(|e| VaultError::Prove(format!("Ironwood bundle: {e:?}")))?;
    }
    if prover.requires_orchard_proof() || prover.requires_ironwood_proof() {
        return Err(VaultError::Prove(
            "a bundle still lacks a proof after proving".into(),
        ));
    }
    Ok(prover.finish())
}

/// An Orchard-protocol action awaiting a spend authorization signature.
#[derive(Clone, Debug)]
pub struct PendingSpend {
    /// Bundle (pool) the action belongs to.
    pub pool: ValuePool,
    /// Index of the action in that bundle.
    pub index: usize,
    /// Spend-auth randomizer `alpha`.
    pub alpha: pallas::Scalar,
    /// The randomized verification key the signature must verify under.
    pub rk: [u8; 32],
    /// Value of the spent note, if present in the PCZT. Zero for the
    /// wallet-controlled spends paired with change outputs.
    pub value: Option<u64>,
}

/// Lists every spend in the Orchard and Ironwood bundles that still needs a
/// signature.
///
/// Protocol padding dummies are pre-signed by the IO finalizer and skipped by
/// the `spend_auth_sig` check. A spend that still carries a `dummy_sk` means the
/// IO finalizer has not run; signers must reject such PCZTs.
pub fn pending_spends(pczt: &Pczt) -> Result<Vec<PendingSpend>, VaultError> {
    // The raw `pczt::orchard::Spend` keeps `alpha` private; the Verifier role
    // parses each bundle into `orchard::pczt::Bundle`, whose actions expose it.
    type RawSpend = (
        ValuePool,
        usize,
        bool,
        Option<pallas::Scalar>,
        redpallas::VerificationKey<SpendAuth>,
        Option<u64>,
    );
    fn collect(pool: ValuePool, bundle: &orchard::pczt::Bundle, raw: &mut Vec<RawSpend>) {
        for (index, action) in bundle.actions().iter().enumerate() {
            let spend = action.spend();
            if spend.spend_auth_sig().is_some() {
                continue;
            }
            raw.push((
                pool,
                index,
                spend.dummy_sk().is_some(),
                *spend.alpha(),
                spend.rk().clone(),
                spend.value().map(|v| v.inner()),
            ));
        }
    }
    let mut raw = Vec::new();
    Verifier::new(pczt.clone())
        .with_orchard::<Infallible, _>(|bundle| {
            collect(ValuePool::Orchard, bundle, &mut raw);
            Ok(())
        })
        .map_err(|e| VaultError::Pczt(format!("parse Orchard bundle: {e:?}")))?
        .with_ironwood::<Infallible, _>(|bundle| {
            collect(ValuePool::Ironwood, bundle, &mut raw);
            Ok(())
        })
        .map_err(|e| VaultError::Pczt(format!("parse Ironwood bundle: {e:?}")))?;

    let mut spends = Vec::with_capacity(raw.len());
    for (pool, index, has_dummy_sk, alpha, rk, value) in raw {
        let pool_label = || pool_name(pool).to_owned();
        if has_dummy_sk {
            return Err(VaultError::DummyKeyPresent {
                pool: pool_label(),
                index,
            });
        }
        let alpha = alpha.ok_or_else(|| VaultError::MissingAlpha {
            pool: pool_label(),
            index,
        })?;
        spends.push(PendingSpend {
            pool,
            index,
            alpha,
            rk: <[u8; 32]>::from(&rk),
            value,
        });
    }
    if spends.is_empty() {
        return Err(VaultError::NothingToSign);
    }
    Ok(spends)
}

/// FROST-signs every pending spend (Orchard and Ironwood bundles) in a proven
/// PCZT.
///
/// Returns the signed PCZT and the list of signed spends. Each signature is
/// verified twice: once against the randomized group key inside
/// [`frost::randomized_sign`], and once more by the PCZT signer role against
/// the action's `rk`. The `rk` comparison before applying also proves that the
/// spend is controlled by this vault's key: `rk` must equal the group key
/// randomized by the action's own `alpha`.
pub fn sign_pczt(
    pczt: Pczt,
    signers: &[&KeyPackage],
    public_key_package: &PublicKeyPackage,
) -> Result<(Pczt, Vec<SignedSpend>), VaultError> {
    let pending = pending_spends(&pczt)?;
    let mut signer =
        Signer::new(pczt).map_err(|e| VaultError::Pczt(format!("signer init: {e:?}")))?;
    let sighash = signer.shielded_sighash();

    let mut signed = Vec::with_capacity(pending.len());
    for spend in &pending {
        let produced =
            frost::randomized_sign(&sighash, &spend.alpha, signers, public_key_package, OsRng)?;
        if produced.randomized_verifying_key != spend.rk {
            return Err(VaultError::RkMismatch {
                pool: pool_name(spend.pool).to_owned(),
                index: spend.index,
            });
        }
        let signature = redpallas::Signature::<SpendAuth>::from(produced.signature);
        let applied = match spend.pool {
            ValuePool::Orchard => signer.apply_orchard_signature(spend.index, signature),
            ValuePool::Ironwood => signer.apply_ironwood_signature(spend.index, signature),
        };
        applied.map_err(|e| {
            VaultError::Pczt(format!(
                "apply signature to {} action {}: {e:?}",
                pool_name(spend.pool),
                spend.index
            ))
        })?;
        signed.push(SignedSpend {
            pool: spend.pool,
            index: spend.index,
            value: spend.value,
        });
    }

    Ok((signer.finish(), signed))
}

/// Builds the ZIP 317 change strategy used for every vault transaction.
///
/// Change from an Ironwood spend stays in Ironwood; the fallback pool only
/// applies to flows the strategy cannot classify.
fn change_strategy() -> SingleOutputChangeStrategy<zip317::FeeRule, Db> {
    SingleOutputChangeStrategy::new(
        zip317::FeeRule::standard(),
        None,
        ShieldedPool::Ironwood,
        DustOutputPolicy::default(),
    )
}

/// Explains an insufficient-funds failure using the wallet's per-pool state.
fn explain_insufficient(
    report: &wallet::BalanceReport,
    available: Zatoshis,
    required: Zatoshis,
) -> VaultError {
    let ironwood = report.ironwood;
    let orchard = report.orchard;
    if ironwood.total.into_u64() < required.into_u64() && orchard.total.into_u64() > 0 {
        return VaultError::FundsInClosedOrchardPool {
            orchard: orchard.total.into_u64(),
            ironwood: ironwood.total.into_u64(),
        };
    }
    if ironwood.pending.into_u64() > 0 {
        return VaultError::FundsNotYetSpendable {
            spendable: ironwood.spendable.into_u64(),
            pending: ironwood.pending.into_u64(),
            required: required.into_u64(),
            min_confirmations: report.min_confirmations,
            change_confirmations: report.change_confirmations,
        };
    }
    VaultError::insufficient(available, required)
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

    // Refuse early, with an explanation, when the funds cannot be spent at all.
    let report = wallet::balance(db, account)?;
    if report.ironwood.total.into_u64() == 0 && report.orchard.total.into_u64() > 0 {
        return Err(VaultError::FundsInClosedOrchardPool {
            orchard: report.orchard.total.into_u64(),
            ironwood: 0,
        });
    }

    // The proving key takes minutes to build and the transaction expires 20
    // blocks after its target height, so build it before the proposal fixes
    // that height. The circuit depends only on the consensus branch, which is
    // re-checked against the proposal's actual target height below.
    let expected_target = BlockHeight::from_u32(report.chain_tip) + 1;
    let circuit = circuit_version_for(expected_target)?;
    tracing::info!("building Orchard proving key for {circuit:?} (this takes a while)");
    let proving_key = ProvingKey::build(circuit);
    let verifying_key = VerifyingKey::build(circuit);

    let payment = Payment::new(destination, Some(amount), memo, None, None, vec![])
        .map_err(|e| VaultError::InvalidAddress(format!("payment: {e}")))?;
    let tx_request = TransactionRequest::new(vec![payment])
        .map_err(|e| VaultError::Build(format!("request: {e}")))?;

    tracing::info!(
        "proposing transfer of {} ZEC from Ironwood",
        format_zec(amount)
    );
    let proposal = propose_transfer::<_, _, _, _, <Db as WalletCommitmentTrees>::Error>(
        db,
        &NETWORK,
        account,
        &GreedyInputSelector::<Db>::new(),
        &change_strategy(),
        tx_request,
        ConfirmationsPolicy::default(),
        &SpendPolicy::shielded_pools([ShieldedPool::Ironwood]),
        None,
        // `None`: build at the version implied by the target height (V6 from NU6.3).
        None,
    )
    .map_err(|e| match e {
        WalletApiError::InsufficientFunds {
            available,
            required,
        } => explain_insufficient(&report, available, required),
        other => VaultError::Build(other.to_string()),
    })?;

    for step in proposal.steps() {
        for input in step
            .shielded_inputs()
            .map(|i| i.notes().iter())
            .into_iter()
            .flatten()
        {
            let pool = match input.note() {
                Note::Orchard { pool, .. } => pool_name(*pool),
                Note::Sapling(_) => "sapling",
            };
            if pool != pool_name(ValuePool::Ironwood) {
                return Err(VaultError::UnexpectedInputPool(pool.to_owned()));
            }
        }
    }

    let fee = proposal
        .steps()
        .iter()
        .map(|step| step.balance().fee_required())
        .try_fold(Zatoshis::ZERO, |acc, f| acc + f)
        .ok_or_else(|| VaultError::Build("fee overflow".into()))?;
    let target_height = BlockHeight::from(proposal.min_target_height());
    let required_circuit = circuit_version_for(target_height)?;
    if required_circuit != circuit {
        return Err(VaultError::CircuitMismatch {
            built: format!("{circuit:?}"),
            required: format!("{required_circuit:?}"),
        });
    }
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

    let pczt = prove(pczt, &proving_key)?;

    let (pczt, signed_spends) = sign_pczt(pczt, signers, public_key_package)?;
    for spend in &signed_spends {
        tracing::info!(
            "FROST-signed {} action {} (value {})",
            pool_name(spend.pool),
            spend.index,
            spend
                .value
                .map(|v| v.to_string())
                .unwrap_or_else(|| "redacted".into())
        );
    }

    // `with_orchard` verifies the proofs and signatures of both the Orchard and
    // the Ironwood bundle; post-NU6.3 they share the verifying key.
    let transaction = TransactionExtractor::new(pczt.clone())
        .with_orchard(&verifying_key)
        .extract()
        .map_err(|e| VaultError::Pczt(format!("extract: {e:?}")))?;
    check_version(&transaction, target_height)?;
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
        version: transaction.version(),
        branch: transaction.consensus_branch_id(),
        target_height: target_height.into(),
        signed_spends,
        fee,
    })
}

/// Checks that the extracted transaction has the version and consensus branch
/// the target height requires (V6 on the NU6.3 or NU7 branch on testnet).
pub fn check_version(
    transaction: &Transaction,
    target_height: BlockHeight,
) -> Result<(), VaultError> {
    let expected_branch = BranchId::for_height(&NETWORK, target_height);
    let expected_version = TxVersion::suggested_for_branch(expected_branch);
    if transaction.version() != expected_version
        || transaction.consensus_branch_id() != expected_branch
    {
        return Err(VaultError::WrongTransactionVersion {
            version: format!("{:?}", transaction.version()),
            branch: format!("{:?}", transaction.consensus_branch_id()),
            expected_version: format!("{expected_version:?}"),
            expected_branch: format!("{expected_branch:?}"),
            target_height: target_height.into(),
        });
    }
    Ok(())
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
        // Testnet activations: NU6.2 at 4_052_000, NU6.3 at 4_134_000, NU7 at 4_465_026.
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
        assert_eq!(
            circuit_version_for(BlockHeight::from_u32(4_500_000)).expect("v3 under NU7"),
            OrchardCircuitVersion::PostNu6_3
        );
    }

    #[test]
    fn testnet_is_on_nu7_with_the_real_branch_id() {
        // Guards the vendored zcash_protocol patch: without it NU7 is the
        // placeholder 0xffffffff and never activates on testnet.
        let branch = BranchId::for_height(&NETWORK, BlockHeight::from_u32(4_498_706));
        assert_eq!(u32::from(branch), 0x7719_0ad9);
        assert_eq!(BranchId::try_from(0x7719_0ad9u32).expect("known"), branch);
        assert_eq!(
            u32::from(BranchId::for_height(
                &NETWORK,
                BlockHeight::from_u32(4_465_025)
            )),
            0x37a5_165b
        );
        assert_eq!(TxVersion::suggested_for_branch(branch), TxVersion::V6);
        assert!(branch.has_orchard());
    }
}
