//! The five round checks.
//!
//! 1. **Beacon**: the drand quicknet BLS signature verifies (drand
//!    `bls-unchained-g1-rfc9380`, see `zeek_core::beacon`), and the drand
//!    round is the one the rules bind from the time of the window's first
//!    block, read from the chain.
//! 2. **Digs**: every compact block of the window is trial-decrypted with
//!    the game's public UIVK; every note found is completed from its full
//!    transaction (value, memo); the set must equal the bundle's notes
//!    exactly, so no dig is hidden and none is invented.
//! 3. **Timing**: the median-time-past of the blocks after the window, read
//!    from the chain, is recomputed against the beacon's publication time.
//! 4. **Settlement**: recomputed with `zeek-core` from the bundle (whose
//!    inputs checks 1 to 3 tied to the chain) and compared byte for byte.
//! 5. **Payout**: the payout transaction is mined (its txid is in the
//!    compact block at the claimed height), its Ironwood bundle verifies
//!    (Halo2 proof with the Orchard-protocol verifying key, spend
//!    authorization and binding signatures over the transaction's sighash,
//!    ZIP 244 / V6), and every payout of the settlement is an output of it
//!    with the settled amount: each payout opening recomputes the output's
//!    `cmx` (protocol spec § 5.4.8.4 note commitments, § 4.16 extraction;
//!    `rho` is the action's nullifier, § 4.7.3).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use orchard::circuit::{OrchardCircuitVersion, VerifyingKey};
use orchard::keys::PreparedIncomingViewingKey;
use orchard::note::{ExtractedNoteCommitment, NoteVersion, RandomSeed, Rho};
use orchard::note_encryption::{CompactAction, IronwoodDomain, OrchardDomain};
use orchard::value::NoteValue;
use orchard::{Address, Note};
use zcash_client_backend::proto::compact_formats::CompactOrchardAction;
use zcash_keys::keys::UnifiedIncomingViewingKey;
use zcash_note_encryption::{try_compact_note_decryption, try_note_decryption};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{MAIN_NETWORK, TEST_NETWORK};
use zeek_core::beacon;
use zeek_core::digs::ReceivedNote;
use zeek_core::params::{GameParams, Network};
use zeek_core::proof::{self, PayoutOpening, ProofBundle};
use zeek_core::random;
use zeek_core::rounds;
use zeek_core::settle::RoundOutcome;
use zeek_core::timing;

use crate::chain::{parse_tx, txid_bytes, Blocks, MinedTx};
use crate::report::{CheckId, CheckResult, CheckStatus, RoundReport};

/// Errors that stop verification before any check can run.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("the public UIVK is not valid: {0}")]
    Uivk(String),
    #[error("the bundle's round window is invalid: {0}")]
    Window(String),
}

/// The game's public configuration as the verifier trusts it (from the
/// committed config file, or the engine's `/v1/config`).
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct PublicConfig {
    pub params: GameParams,
    pub vault_address: String,
    pub uivk: String,
}

impl PublicConfig {
    /// The vault's external Orchard-protocol incoming viewing key.
    pub fn ivk(&self) -> Result<PreparedIncomingViewingKey, VerifyError> {
        let uivk = match self.params.network {
            Network::Main => UnifiedIncomingViewingKey::decode(&MAIN_NETWORK, &self.uivk),
            Network::Test => UnifiedIncomingViewingKey::decode(&TEST_NETWORK, &self.uivk),
        }
        .map_err(VerifyError::Uivk)?;
        let ivk = uivk
            .orchard()
            .clone()
            .ok_or_else(|| VerifyError::Uivk("no Orchard component".into()))?;
        Ok(PreparedIncomingViewingKey::new(&ivk))
    }
}

/// The Orchard-protocol verifying key (built once, a few seconds).
pub fn verifying_key() -> &'static VerifyingKey {
    static KEY: OnceLock<VerifyingKey> = OnceLock::new();
    KEY.get_or_init(|| VerifyingKey::build(OrchardCircuitVersion::PostNu6_3))
}

/// Heights of the compact blocks the checks need (inclusive): the window
/// and the `mtp_window` blocks after it.
pub fn block_range(bundle: &ProofBundle) -> (u32, u32) {
    let end = bundle
        .window
        .end
        .saturating_add(bundle.params.mtp_window)
        .saturating_sub(1);
    (bundle.window.start, end)
}

/// A note the public UIVK finds in a compact block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Display-order hex.
    pub txid: String,
    pub height: u32,
    pub ironwood: bool,
    pub action_index: u16,
    pub value: u64,
}

fn scan_actions(
    ivk: &PreparedIncomingViewingKey,
    height: u32,
    txid: &str,
    ironwood: bool,
    actions: &[CompactOrchardAction],
    hits: &mut Vec<Hit>,
) {
    for (index, action) in actions.iter().enumerate() {
        let Ok(compact) = CompactAction::try_from(action) else {
            continue;
        };
        let decrypted = if ironwood {
            try_compact_note_decryption(
                &IronwoodDomain::for_compact_action(&compact),
                ivk,
                &compact,
            )
        } else {
            try_compact_note_decryption(&OrchardDomain::for_compact_action(&compact), ivk, &compact)
        };
        if let (Some((note, _)), Ok(action_index)) = (decrypted, u16::try_from(index)) {
            hits.push(Hit {
                txid: txid.to_owned(),
                height,
                ironwood,
                action_index,
                value: note.value().inner(),
            });
        }
    }
}

/// Phase 1: trial-decrypts every compact block of the window with the
/// public UIVK.
pub fn scan_window(
    bundle: &ProofBundle,
    ivk: &PreparedIncomingViewingKey,
    blocks: &Blocks,
) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (height, block) in blocks.iter() {
        if !bundle.window.contains(*height) {
            continue;
        }
        for tx in &block.vtx {
            let mut display = tx.txid.clone();
            display.reverse();
            let txid = hex::encode(display);
            scan_actions(ivk, *height, &txid, true, &tx.ironwood_actions, &mut hits);
            scan_actions(ivk, *height, &txid, false, &tx.actions, &mut hits);
        }
    }
    hits
}

/// Phase 2 input: the full transactions to fetch (display hex): every
/// transaction with a hit, and the payout transaction when openings exist.
pub fn transactions_to_fetch(bundle: &ProofBundle, hits: &[Hit]) -> Vec<String> {
    let mut txids: BTreeSet<String> = hits.iter().map(|h| h.txid.clone()).collect();
    if let Some(openings) = &bundle.payout_openings {
        txids.insert(openings.txid.clone());
    }
    txids.into_iter().collect()
}

/// Height of the block that must hold the payout transaction, if any.
pub fn payout_height(bundle: &ProofBundle) -> Option<u32> {
    bundle.payout_openings.as_ref().map(|o| o.mined_height)
}

/// Everything the checks read.
pub struct RoundInput<'a> {
    pub config: &'a PublicConfig,
    pub bundle: &'a ProofBundle,
    pub blocks: &'a Blocks,
    /// Full transactions by display txid.
    pub txs: &'a BTreeMap<String, MinedTx>,
    pub ivk: &'a PreparedIncomingViewingKey,
}

/// Runs the five checks.
pub fn verify_round(input: &RoundInput<'_>) -> RoundReport {
    let hits = scan_window(input.bundle, input.ivk, input.blocks);
    RoundReport {
        round: input.bundle.round,
        bundle_version: input.bundle.version,
        rules_version: input.bundle.claimed_rules_version(),
        checks: vec![
            check_beacon(input),
            check_digs(input, &hits),
            check_timing(input),
            check_settlement(input.bundle),
            check_payout(input),
        ],
    }
}

fn fail(id: CheckId, summary: impl Into<String>) -> CheckResult {
    CheckResult::new(id, CheckStatus::Fail, summary)
}

/// The parameters that must agree between the bundle and the public config
/// (the bundle may legitimately omit later rules' fields for older rounds).
fn params_agree(bundle: &GameParams, public: &GameParams) -> Result<(), String> {
    let pairs = [
        (
            "network",
            format!("{:?}", bundle.network),
            format!("{:?}", public.network),
        ),
        (
            "genesis_height",
            bundle.genesis_height.to_string(),
            public.genesis_height.to_string(),
        ),
        (
            "blocks_per_round",
            bundle.blocks_per_round.to_string(),
            public.blocks_per_round.to_string(),
        ),
        (
            "rake_bps",
            bundle.rake_bps.to_string(),
            public.rake_bps.to_string(),
        ),
        (
            "cache_odds",
            bundle.cache_odds.to_string(),
            public.cache_odds.to_string(),
        ),
        (
            "min_dig_zat",
            bundle.min_dig_zat.to_string(),
            public.min_dig_zat.to_string(),
        ),
        (
            "marginal_fee_zat",
            bundle.marginal_fee_zat.to_string(),
            public.marginal_fee_zat.to_string(),
        ),
        (
            "drand public key",
            bundle.drand.public_key.clone(),
            public.drand.public_key.clone(),
        ),
        (
            "drand genesis",
            bundle.drand.genesis_time.to_string(),
            public.drand.genesis_time.to_string(),
        ),
        (
            "drand period",
            bundle.drand.period_secs.to_string(),
            public.drand.period_secs.to_string(),
        ),
        (
            "reveal_delay_secs",
            bundle.reveal_delay_secs.to_string(),
            public.reveal_delay_secs.to_string(),
        ),
        (
            "mtp_window",
            bundle.mtp_window.to_string(),
            public.mtp_window.to_string(),
        ),
    ];
    for (name, b, p) in pairs {
        if b != p {
            return Err(format!("{name}: bundle {b}, public config {p}"));
        }
    }
    Ok(())
}

fn check_beacon(input: &RoundInput<'_>) -> CheckResult {
    let id = CheckId::Beacon;
    let bundle = input.bundle;
    let params = &bundle.params;
    if let Err(e) = params_agree(params, &input.config.params) {
        return fail(
            id,
            "The bundle's game rules differ from the published configuration.",
        )
        .with(e);
    }
    let start_time = match input.blocks.time(bundle.window.start) {
        Ok(t) => t,
        Err(e) => {
            return fail(
                id,
                "The round's first block is missing from the chain data.",
            )
            .with(e.to_string())
        }
    };
    if start_time != bundle.start_block_time {
        return fail(id, "The bundle misstates when the round started.").with(format!(
            "block {} nTime: chain {start_time}, bundle {}",
            bundle.window.start, bundle.start_block_time
        ));
    }
    let drand_round = match beacon::beacon_round_for(params, start_time) {
        Ok(r) => r,
        Err(e) => {
            return fail(id, "The bound beacon round cannot be computed.").with(e.to_string())
        }
    };
    if drand_round != bundle.drand_round {
        return fail(
            id,
            "The bundle uses a different beacon round than the rules bind.",
        )
        .with(format!(
            "rules: drand round {drand_round} (from start time {start_time}); bundle: {}",
            bundle.drand_round
        ));
    }
    match bundle.outcome {
        RoundOutcome::Voided => CheckResult::new(
            id,
            CheckStatus::Pass,
            "The round was voided by the timing rule, so no beacon decides it (check 3 confirms the voiding).",
        )
        .with(format!("bound drand round {drand_round} (not used)")),
        RoundOutcome::Drawn { treasure_tile, cache_triggered } => {
            let Some(b) = &bundle.beacon else {
                return fail(id, "A drawn round must include its beacon.");
            };
            if b.round != drand_round {
                return fail(id, "The beacon is for the wrong drand round.")
                    .with(format!("beacon round {}, bound round {drand_round}", b.round));
            }
            let randomness = match beacon::verify_beacon(&params.drand, b) {
                Ok(r) => r,
                Err(e) => {
                    return fail(id, "The beacon's BLS signature does not verify against drand quicknet.")
                        .with(e.to_string())
                }
            };
            if bundle.randomness != Some(randomness) {
                return fail(id, "The bundle's randomness does not come from this beacon.");
            }
            let tile = random::treasure_tile(bundle.round, &randomness);
            let cache = random::cache_triggers(params, bundle.round, &randomness);
            match (tile, cache) {
                (Ok(tile), Ok(cache)) if tile == treasure_tile && cache == cache_triggered => {
                    CheckResult::new(
                        id,
                        CheckStatus::Pass,
                        format!("drand quicknet round {drand_round} is genuine and is the one the rules bind; it puts the treasure on tile {tile}."),
                    )
                    .with(format!("round start block {} nTime {start_time}", bundle.window.start))
                    .with(format!("BLS signature verified against the quicknet public key; cache triggered: {cache}"))
                }
                (Ok(tile), Ok(cache)) => fail(id, "The bundle's draw does not follow from the beacon.").with(format!(
                    "recomputed tile {tile} cache {cache}; bundle tile {treasure_tile} cache {cache_triggered}"
                )),
                (Err(e), _) | (_, Err(e)) => fail(id, "The draw cannot be recomputed.").with(e.to_string()),
            }
        }
    }
}

fn note_key(n: &ReceivedNote) -> (String, u16) {
    (n.txid.clone(), n.output_index)
}

fn check_digs(input: &RoundInput<'_>, hits: &[Hit]) -> CheckResult {
    let id = CheckId::Digs;
    let bundle = input.bundle;
    let window = bundle.window;
    for height in window.start..window.end {
        if let Err(e) = input.blocks.get(height) {
            return fail(id, "The chain data does not cover the whole round window.")
                .with(e.to_string());
        }
    }
    if let Err(e) = input.blocks.linked() {
        return fail(id, "The window's blocks do not form a chain.").with(e);
    }
    // Complete every hit from its full transaction.
    let mut found: BTreeMap<(String, u16), ReceivedNote> = BTreeMap::new();
    for hit in hits {
        let Some(mined) = input.txs.get(&hit.txid) else {
            return fail(id, "A transaction paying the vault could not be fetched.")
                .with(hit.txid.clone());
        };
        let tx = match parse_tx(bundle.params.network, &hit.txid, mined) {
            Ok(tx) => tx,
            Err(e) => {
                return fail(id, "A transaction paying the vault does not parse.")
                    .with(e.to_string())
            }
        };
        let bundle_ref = if hit.ironwood {
            tx.ironwood_bundle()
        } else {
            tx.orchard_bundle()
        };
        let Some(action) = bundle_ref.and_then(|b| b.actions().get(usize::from(hit.action_index)))
        else {
            return fail(id, "A compact action is missing from its full transaction.")
                .with(format!("{}:{}", hit.txid, hit.action_index));
        };
        let decrypted = if hit.ironwood {
            try_note_decryption(&IronwoodDomain::for_action(action), input.ivk, action)
        } else {
            try_note_decryption(&OrchardDomain::for_action(action), input.ivk, action)
        };
        let Some((note, _, memo)) = decrypted else {
            return fail(id, "A detected output does not decrypt in full.")
                .with(format!("{}:{}", hit.txid, hit.action_index));
        };
        found.insert(
            (hit.txid.clone(), hit.action_index),
            ReceivedNote {
                txid: hit.txid.clone(),
                output_index: hit.action_index,
                height: hit.height,
                value: note.value().inner(),
                memo: memo.to_vec(),
            },
        );
    }
    let claimed: BTreeMap<(String, u16), &ReceivedNote> =
        bundle.notes.iter().map(|n| (note_key(n), n)).collect();
    let hidden: Vec<String> = found
        .keys()
        .filter(|k| !claimed.contains_key(*k))
        .map(|(t, i)| format!("{t}:{i}"))
        .collect();
    let invented: Vec<String> = claimed
        .keys()
        .filter(|k| !found.contains_key(*k))
        .map(|(t, i)| format!("{t}:{i}"))
        .collect();
    let altered: Vec<String> = found
        .iter()
        .filter_map(|(k, n)| {
            claimed
                .get(k)
                .filter(|c| ***c != *n)
                .map(|_| format!("{}:{}", k.0, k.1))
        })
        .collect();
    if !hidden.is_empty() || !invented.is_empty() || !altered.is_empty() {
        let mut result = fail(
            id,
            "The bundle's digs do not match what the vault received on chain.",
        );
        if !hidden.is_empty() {
            result = result.with(format!(
                "on chain but missing from the bundle: {}",
                hidden.join(", ")
            ));
        }
        if !invented.is_empty() {
            result = result.with(format!(
                "in the bundle but not on chain: {}",
                invented.join(", ")
            ));
        }
        if !altered.is_empty() {
            result = result.with(format!(
                "value, height or memo differ: {}",
                altered.join(", ")
            ));
        }
        return result;
    }
    let total: u64 = found.values().map(|n| n.value).sum();
    CheckResult::new(
        id,
        CheckStatus::Pass,
        format!(
            "All {} payments the vault received in blocks {}..{} are in the bundle, with the same amounts and memos; none is missing or made up.",
            found.len(),
            window.start,
            window.last()
        ),
    )
    .with(format!("{} blocks trial-decrypted with the public UIVK; {} zatoshis in total", window.end - window.start, total))
}

fn check_timing(input: &RoundInput<'_>) -> CheckResult {
    let id = CheckId::Timing;
    let bundle = input.bundle;
    let params = &bundle.params;
    let mut times = Vec::with_capacity(bundle.post_close_block_times.len());
    for offset in 0..params.mtp_window {
        let height = bundle.window.end + offset;
        match input.blocks.time(height) {
            Ok(t) => times.push(t),
            Err(e) => {
                return fail(
                    id,
                    "The blocks after the round are missing from the chain data.",
                )
                .with(e.to_string())
            }
        }
    }
    if times != bundle.post_close_block_times {
        return fail(
            id,
            "The bundle misstates the times of the blocks after the round.",
        )
        .with(format!(
            "chain {times:?}, bundle {:?}",
            bundle.post_close_block_times
        ));
    }
    let beacon_time = match beacon::beacon_time(params, bundle.drand_round) {
        Ok(t) => t,
        Err(e) => return fail(id, "The beacon time cannot be computed.").with(e.to_string()),
    };
    match timing::check_window_close(params, beacon_time, &times) {
        Ok(verdict) if verdict == bundle.timing => {
            let mtp = timing::median_time_past(&times).unwrap_or(0);
            let safe = matches!(verdict, timing::TimingVerdict::Safe { .. });
            CheckResult::new(
                id,
                CheckStatus::Pass,
                if safe {
                    "The round closed safely before the beacon was published, so nobody could know the treasure tile while digging."
                } else {
                    "The round closed too late relative to the beacon, so it was correctly voided and refunded."
                },
            )
            .with(format!("median time past of blocks {}..{}: {mtp}; beacon published at {beacon_time}; clock skew allowance {} s", bundle.window.end, bundle.window.end + params.mtp_window - 1, params.honest_clock_skew_secs))
        }
        Ok(verdict) => fail(id, "The timing verdict in the bundle is wrong.").with(format!(
            "recomputed {verdict:?}, bundle {:?}",
            bundle.timing
        )),
        Err(e) => fail(id, "The timing rule cannot be evaluated.").with(e.to_string()),
    }
}

fn check_settlement(bundle: &ProofBundle) -> CheckResult {
    let id = CheckId::Settlement;
    match proof::recompute(bundle) {
        Ok(recomputed) => {
            let ours = serde_json::to_vec(&recomputed).unwrap_or_default();
            let theirs = serde_json::to_vec(&bundle.settlement).unwrap_or_default();
            if recomputed == bundle.settlement && !ours.is_empty() && ours == theirs {
                let s = &recomputed;
                CheckResult::new(
                    id,
                    CheckStatus::Pass,
                    format!(
                        "Recomputing the round from its digs gives exactly the published result: {} paid out to {} winner(s), Deep Cache {} → {}.",
                        s.total_paid, s.winners, s.previous_cache, s.new_cache
                    ),
                )
                .with(format!("pot {}, rake {}, fees {}, forfeited {}, shouts {} (reserved {})", s.pot, s.rake, s.total_fees, s.forfeited, s.shouts, s.shout_fees))
                .with(format!("byte-for-byte equal ({} bytes of JSON)", ours.len()))
            } else {
                fail(
                    id,
                    "The published settlement differs from the recomputed one.",
                )
                .with(format!("recomputed {}", String::from_utf8_lossy(&ours)))
            }
        }
        Err(e) => fail(id, "The bundle does not recompute.").with(e.to_string()),
    }
}

/// The `cmx` of the note an opening describes, with `rho` taken from the
/// action's nullifier.
pub fn opening_cmx(opening: &PayoutOpening, rho_bytes: [u8; 32]) -> Result<[u8; 32], String> {
    let recipient: [u8; 43] = hex::decode(&opening.recipient)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("recipient is not 43 bytes of hex")?;
    let recipient = Option::<Address>::from(Address::from_raw_address_bytes(&recipient))
        .ok_or("recipient is not an Orchard-protocol address")?;
    let rseed: [u8; 32] = hex::decode(&opening.rseed)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("rseed is not 32 bytes of hex")?;
    let rho = Option::<Rho>::from(Rho::from_bytes(&rho_bytes)).ok_or("rho is not canonical")?;
    let rseed = Option::<RandomSeed>::from(RandomSeed::from_bytes(rseed, &rho))
        .ok_or("rseed is invalid for rho")?;
    let version = match opening.note_version {
        2 => NoteVersion::V2,
        3 => NoteVersion::V3,
        v => return Err(format!("unknown note version {v}")),
    };
    let note = Option::<Note>::from(Note::from_parts(
        recipient,
        NoteValue::from_raw(opening.value),
        rho,
        rseed,
        version,
    ))
    .ok_or("the opening is not a valid note")?;
    Ok(ExtractedNoteCommitment::from(note.commitment()).to_bytes())
}

/// Verifies the Ironwood bundle of a mined transaction: Halo2 proof,
/// spend authorization signatures and binding signature over its sighash.
///
/// For a transaction without a transparent component the shielded
/// signature digest equals the txid digest (ZIP 244 § S.2: the transparent
/// part of the signature digest is the transparent txid digest when no
/// transparent input is being signed; `sighash_v6::v6_signature_hash` differs
/// from the txid only there), so the message is the txid. Transactions with
/// a transparent component are refused rather than guessed at.
pub fn verify_ironwood_bundle(tx: &Transaction) -> Result<usize, String> {
    let bundle = tx.ironwood_bundle().ok_or("no Ironwood bundle")?;
    if tx.transparent_bundle().is_some() {
        return Err(
            "the transaction has a transparent component; payout transactions never do".into(),
        );
    }
    bundle
        .verify_proof(verifying_key())
        .map_err(|e| format!("Halo2 proof does not verify: {e:?}"))?;
    let txid = tx.txid();
    let msg: &[u8; 32] = txid.as_ref();
    for (i, action) in bundle.actions().iter().enumerate() {
        action
            .rk()
            .verify(msg, action.authorization())
            .map_err(|_| format!("spend authorization signature of action {i} is invalid"))?;
    }
    bundle
        .binding_validating_key()
        .verify(msg, bundle.authorization().binding_signature())
        .map_err(|_| "binding signature is invalid".to_owned())?;
    Ok(bundle.actions().len())
}

fn check_payout(input: &RoundInput<'_>) -> CheckResult {
    let id = CheckId::Payout;
    let bundle = input.bundle;
    let settlement = &bundle.settlement;
    let Some(openings) = &bundle.payout_openings else {
        let why = if bundle.version < 4 {
            format!("Not available for this bundle version ({}): payout openings were added in version 4.", bundle.version)
        } else if settlement.payouts.is_empty() {
            "Nothing to pay this round, so there is no payout transaction.".to_owned()
        } else {
            "The payout transaction is not mined yet (or not recorded); check again later."
                .to_owned()
        };
        return CheckResult::new(id, CheckStatus::NotAvailable, why);
    };
    if let Some(record) = &bundle.payout {
        if record.txid.as_deref() != Some(openings.txid.as_str()) {
            return fail(
                id,
                "The openings name a different transaction than the payout record.",
            );
        }
    }
    let Some(internal) = txid_bytes(&openings.txid) else {
        return fail(id, "The payout txid is malformed.");
    };
    if !input.blocks.contains_tx(openings.mined_height, &internal) {
        return fail(
            id,
            "The payout transaction is not in the block it claims to be mined in.",
        )
        .with(format!(
            "{} at height {}",
            openings.txid, openings.mined_height
        ));
    }
    let Some(mined) = input.txs.get(&openings.txid) else {
        return fail(id, "The payout transaction could not be fetched.");
    };
    let tx = match parse_tx(bundle.params.network, &openings.txid, mined) {
        Ok(tx) => tx,
        Err(e) => return fail(id, "The payout transaction does not parse.").with(e.to_string()),
    };
    if tx.txid().to_string() != openings.txid {
        return fail(id, "The fetched transaction has a different txid.");
    }
    let actions = match verify_ironwood_bundle(&tx) {
        Ok(n) => n,
        Err(e) => {
            return fail(
                id,
                "The payout transaction's proof or signatures do not verify.",
            )
            .with(e)
        }
    };
    let Some(ironwood) = tx.ironwood_bundle() else {
        return fail(id, "The payout transaction has no Ironwood bundle.");
    };
    let mut covered = BTreeSet::new();
    let mut lines = Vec::new();
    for opening in &openings.outputs {
        let Some(payout) = settlement.payouts.get(opening.payout_index as usize) else {
            return fail(
                id,
                "An opening names a payout the settlement does not have.",
            )
            .with(format!("payout index {}", opening.payout_index));
        };
        if !covered.insert(opening.payout_index) {
            return fail(id, "Two openings claim the same payout.");
        }
        if opening.value != payout.net {
            return fail(id, "An output's amount differs from the settled amount.").with(format!(
                "payout {}: opening {}, settlement {}",
                opening.payout_index, opening.value, payout.net
            ));
        }
        let Some(action) = ironwood.actions().get(opening.action_index as usize) else {
            return fail(
                id,
                "An opening points at an action the transaction does not have.",
            );
        };
        match opening_cmx(opening, action.nullifier().to_bytes()) {
            Ok(cmx) if cmx == action.cmx().to_bytes() => lines.push(format!(
                "payout {} ({} zatoshis) is action {}: cmx {} recomputed",
                opening.payout_index,
                opening.value,
                opening.action_index,
                hex::encode(cmx)
            )),
            Ok(_) => {
                return fail(
                    id,
                    "An opening does not match the output in the transaction.",
                )
                .with(format!(
                    "payout {} action {}",
                    opening.payout_index, opening.action_index
                ))
            }
            Err(e) => return fail(id, "An opening is malformed.").with(e),
        }
    }
    if covered.len() != settlement.payouts.len() {
        return fail(
            id,
            "Some settled payouts have no output in the transaction.",
        )
        .with(format!(
            "{} payouts settled, {} opened",
            settlement.payouts.len(),
            covered.len()
        ));
    }
    let mut result = CheckResult::new(
        id,
        CheckStatus::Pass,
        format!(
            "The payout transaction is mined at block {} and valid, and it pays every settled amount ({} output(s), {} zatoshis).",
            openings.mined_height,
            settlement.payouts.len(),
            settlement.total_paid
        ),
    )
    .with(format!("txid {}, {actions} actions; Halo2 proof, spend authorization and binding signatures verified", openings.txid));
    for line in lines {
        result = result.with(line);
    }
    result
}

/// Whether `bundle.round`'s window is consistent with its parameters (used
/// before fetching data).
pub fn check_window(bundle: &ProofBundle) -> Result<(), VerifyError> {
    let window = rounds::round_window(&bundle.params, bundle.round)
        .map_err(|e| VerifyError::Window(e.to_string()))?;
    if window != bundle.window {
        return Err(VerifyError::Window(format!(
            "expected {window:?}, bundle says {:?}",
            bundle.window
        )));
    }
    Ok(())
}
