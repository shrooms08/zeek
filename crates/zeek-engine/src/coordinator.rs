//! The payout coordinator: turns a settlement plan into a transaction the
//! keyholders are asked to sign, runs the FROST signing session over HTTP,
//! verifies and applies the signatures, and broadcasts.
//!
//! Trust: the coordinator holds no key share. It can only propose; each
//! keyholder recomputes the settlement from its own view of the chain and
//! refuses anything that differs (see `zeek_keyholder::verify`). Every
//! aggregated signature is verified here against the vault's `ak` randomized
//! by the action's `alpha` before it is applied, and the PCZT signer role
//! verifies it once more against the action's `rk`.
//!
//! Retry safety: a payout record in state `signing` is written before the
//! keyholders are asked for shares, and a round with a record is never
//! rebuilt automatically; the operator inspects it. A transaction is
//! broadcast only after the record exists, so no round is ever paid twice. A
//! record left in `signing` after a broadcast (a crash between the two
//! steps) is completed by `zeek-engine payout-recover`, which finds the
//! transaction that spent the round's charged notes on the chain.

use std::collections::BTreeMap;
use std::env;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use orchard::circuit::{ProvingKey, VerifyingKey};
use orchard::keys::{FullViewingKey, Scope};
use orchard::primitives::redpallas::{self, SpendAuth};
use orchard::ValuePool;
use pczt::roles::signer::Signer;
use pczt::Pczt;
use rand_core::RngCore;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::consensus::BlockHeight;
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zeek_core::memo::DigMemo;
use zeek_core::params::GameParams;
use zeek_core::proof::UnclaimablePayout;
use zeek_core::seal::SealingPrivateKey;
use zeek_core::settle::{self, PayoutReason};
use zeek_vault::frost::{self, PublicKeyPackage, SigningCommitments, SigningPackage};
use zeek_vault::pool::Pool;
use zeek_vault::send::{self, NoteId, PendingSpend, PreparedPczt, ShapedOutput};
use zeek_vault::store::{SecretsDir, VaultFile};
use zeek_vault::wallet;

use crate::config::GameConfig;
use crate::db::{Db, PayoutRecord, PayoutState, PlanStatus};
use crate::error::EngineError;
use crate::identity::{self, Identity};
use crate::ledger::{self, Ledger};
use crate::processor::Processor;
use crate::protocol::{
    self, kind, Envelope, Expect, Refusal, SessionId, SignCommitments, SignPackages, SignRequest,
    SignShares,
};
use crate::store::Store;

/// Participant identifier of the coordinator in envelopes.
pub const COORDINATOR_ID: u16 = 0;

/// Environment variables.
pub const KEYHOLDERS_VAR: &str = "ZEEK_KEYHOLDERS";
pub const IDENTITY_VAR: &str = "ZEEK_COORDINATOR_IDENTITY";
pub const VAULT_SECRETS_VAR: &str = "ZEEK_VAULT_SECRETS_DIR";
pub const VAULT_DATA_VAR: &str = "ZEEK_VAULT_DATA_DIR";
pub const THRESHOLD_VAR: &str = "ZEEK_VAULT_THRESHOLD";
#[cfg(feature = "rogue-demo")]
pub const ROGUE_VAR: &str = "ZEEK_ROGUE_PAYOUT_ADDRESS";

/// A keyholder the coordinator talks to.
#[derive(Clone, Debug)]
pub struct Keyholder {
    pub identifier: u16,
    pub url: String,
    /// Pinned identity public key, hex.
    pub identity: String,
}

/// Coordinator settings from the environment.
#[derive(Clone, Debug)]
pub struct CoordinatorSettings {
    pub keyholders: Vec<Keyholder>,
    pub identity_path: PathBuf,
    /// Directory holding the vault's `vault.json` and `payout-sealing.json`.
    pub vault_secrets_dir: PathBuf,
    /// Directory holding the vault's wallet database.
    pub vault_data_dir: PathBuf,
    pub threshold: u16,
    #[cfg(feature = "rogue-demo")]
    pub rogue_address: Option<String>,
}

impl CoordinatorSettings {
    /// Reads the settings; `None` when no keyholders are configured.
    pub fn from_env() -> Result<Option<Self>, EngineError> {
        let keyholders = match env::var(KEYHOLDERS_VAR) {
            Ok(v) if !v.trim().is_empty() => v,
            _ => return Ok(None),
        };
        let mut parsed = Vec::new();
        for entry in keyholders.split(',') {
            let parts: Vec<&str> = entry.trim().splitn(3, '=').collect();
            let [id, url, key] = parts.as_slice() else {
                return Err(EngineError::Config(format!(
                    "{KEYHOLDERS_VAR} entries must be `id=url=identity_hex`, got `{entry}`"
                )));
            };
            let identifier: u16 = id
                .parse()
                .map_err(|_| EngineError::Config(format!("bad keyholder id `{id}`")))?;
            identity::decode_key(key).map_err(|e| {
                EngineError::Config(format!("keyholder {identifier} identity: {e}"))
            })?;
            parsed.push(Keyholder {
                identifier,
                url: url.trim_end_matches('/').to_owned(),
                identity: (*key).to_owned(),
            });
        }
        let var = |name: &str, default: &str| {
            env::var(name)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| default.to_owned())
        };
        let threshold: u16 = var(THRESHOLD_VAR, "2")
            .parse()
            .map_err(|_| EngineError::Config(format!("{THRESHOLD_VAR} must be a number")))?;
        Ok(Some(CoordinatorSettings {
            keyholders: parsed,
            identity_path: PathBuf::from(var(IDENTITY_VAR, ".secrets/coordinator/identity.json")),
            vault_secrets_dir: PathBuf::from(var(VAULT_SECRETS_VAR, ".secrets")),
            vault_data_dir: PathBuf::from(var(VAULT_DATA_VAR, "data")),
            threshold,
            #[cfg(feature = "rogue-demo")]
            rogue_address: env::var(ROGUE_VAR).ok().filter(|v| !v.trim().is_empty()),
        }))
    }
}

/// The coordinator.
pub struct Coordinator {
    settings: CoordinatorSettings,
    identity: Identity,
    vault: VaultFile,
    public_key_package: PublicKeyPackage,
    ufvk: UnifiedFullViewingKey,
    fvk: FullViewingKey,
    sealing: SealingPrivateKey,
    http: reqwest::Client,
    pool: Arc<Pool>,
    retry: RetryBackoff,
    /// The vault's consensus parameters (from its vault file).
    params: zeek_vault::Params,
}

/// The address a settled payout is paid to, or why it cannot be paid.
///
/// The coordinator and every keyholder apply this same rule with the same
/// sealing key, so they agree on which payouts a transaction must contain:
/// a payout whose sealed receiver does not open, or opens to bytes that are
/// not a valid Orchard address, is unclaimable (its digger sent a malformed
/// memo) and its value stays in the vault instead of blocking the round.
pub fn payable_receiver(
    network: zeek_core::params::Network,
    sealing: &SealingPrivateKey,
    payout: &settle::Payout,
) -> Result<orchard::Address, String> {
    let memo = DigMemo {
        round: payout.memo_round,
        tile: payout.memo_tile,
        sealed_receiver: payout.sealed_receiver.clone(),
        message: None,
    };
    let receiver = memo
        .open(network, sealing)
        .map_err(|e| format!("the sealed receiver does not open ({e})"))?;
    Option::from(orchard::Address::from_raw_address_bytes(&receiver.orchard))
        .ok_or_else(|| "the opened receiver is not a valid Orchard address".to_owned())
}

/// How long the API reuses a computed ledger (it opens the wallet database).
const LEDGER_CACHE: Duration = Duration::from_secs(15);

/// Reads the vault ledger without the coordinator (see
/// [`Coordinator::ledger_view`]).
#[derive(Clone)]
pub struct LedgerView {
    data_dir: PathBuf,
    params: zeek_vault::Params,
    ufvk: UnifiedFullViewingKey,
    cached: Arc<std::sync::Mutex<Option<(std::time::Instant, Ledger)>>>,
}

impl LedgerView {
    fn open(&self) -> Result<(wallet::Db, zcash_client_sqlite::AccountUuid), EngineError> {
        let db = wallet::open(&self.data_dir, self.params)?;
        let account = wallet::find_account(&db, &self.ufvk)?
            .ok_or_else(|| EngineError::Config("the vault wallet has no account for this vault; run `zeek-vault sync` with the vault's secrets first".into()))?;
        Ok((db, account))
    }

    /// The ledger from the vault wallet (without syncing), reused for
    /// [`LEDGER_CACHE`].
    pub fn ledger(&self, store: &Store, processor: &Processor) -> Result<Ledger, EngineError> {
        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((at, ledger)) = cached.as_ref() {
            if at.elapsed() < LEDGER_CACHE {
                return Ok(ledger.clone());
            }
        }
        let (wallet_db, account) = self.open()?;
        let report = wallet::balance(&wallet_db, account)?;
        let ledger = store.read(|db| {
            ledger::compute(
                db,
                processor,
                report.ironwood.total.into_u64(),
                report.chain_tip,
            )
        })?;
        *cached = Some((std::time::Instant::now(), ledger.clone()));
        Ok(ledger)
    }
}

/// What one payout attempt did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayoutReport {
    pub round: u32,
    pub txid: String,
    pub fee: u64,
    pub signers: Vec<u16>,
    pub outputs: usize,
}

impl Coordinator {
    /// Loads the coordinator's identity and the vault's public material.
    pub fn new(settings: CoordinatorSettings, pool: Arc<Pool>) -> Result<Self, EngineError> {
        let identity = Identity::load(&settings.identity_path)?;
        let secrets = SecretsDir::new(&settings.vault_secrets_dir);
        let vault = secrets.load_vault()?;
        if vault.network()? != pool.network() {
            return Err(EngineError::Config(format!(
                "the vault in {} is a `{}` vault but the game runs on {:?}",
                settings.vault_secrets_dir.display(),
                vault.network,
                pool.network()
            )));
        }
        let params = vault.params()?;
        let public_key_package = vault.public_key_package()?;
        let ufvk = vault.ufvk()?;
        let fvk = vault.orchard_fvk()?;
        let sealing = secrets.load_sealing_key()?.private_key()?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| EngineError::Relay(e.to_string()))?;
        Ok(Coordinator {
            settings,
            identity,
            vault,
            public_key_package,
            ufvk,
            fvk,
            sealing,
            http,
            pool,
            retry: RetryBackoff::default(),
            params,
        })
    }

    /// Records the payout openings (proof bundle format 4) of every round
    /// whose payout transaction is mined: each output is recovered with the
    /// vault's outgoing viewing keys and matched to its settlement payout by
    /// the opened receiver and the net amount. Returns the rounds recorded.
    pub async fn record_openings(
        &self,
        store: &Store,
        source: &mut dyn crate::chain::ChainSource,
        game: &GameConfig,
    ) -> Result<Vec<u32>, EngineError> {
        let mut recorded = Vec::new();
        for (round, txid) in store.read(|db| db.rounds_missing_openings())? {
            let Some(row) = store.read(|db| db.settlement(round))? else {
                continue;
            };
            let mut internal = hex::decode(&txid)
                .map_err(|_| EngineError::Corrupt(format!("payout txid {txid} is not hex")))?;
            internal.reverse();
            let internal: [u8; 32] = internal
                .try_into()
                .map_err(|_| EngineError::Corrupt(format!("payout txid {txid} is not 32 bytes")))?;
            let Some(height) = crate::chain::confirmed_height(source, &internal).await? else {
                continue;
            };
            let raw = source.transaction(&internal).await?;
            // One round that cannot be opened (for example a payout made by an
            // older vault) must not block the others.
            match self.openings_of(game, &row.settlement, &txid, height, &raw) {
                Ok(openings) => {
                    store.write(|db| db.set_payout_openings(round, &openings))?;
                    tracing::info!(round, %txid, height, outputs = openings.outputs.len(), "payout openings recorded");
                    recorded.push(round);
                }
                Err(e) => {
                    tracing::warn!(round, %txid, error = %e, "payout openings unavailable for this round")
                }
            }
        }
        Ok(recorded)
    }

    fn openings_of(
        &self,
        game: &GameConfig,
        settlement: &settle::Settlement,
        txid: &str,
        height: u32,
        raw: &[u8],
    ) -> Result<zeek_core::proof::PayoutOpenings, EngineError> {
        use orchard::note_encryption::IronwoodDomain;
        use zcash_note_encryption::try_output_recovery_with_ovk;
        let branch = crate::chain::branch_for(game.params.network, height);
        let tx = zcash_primitives::transaction::Transaction::read(raw, branch)
            .map_err(|e| EngineError::Corrupt(format!("payout transaction {txid}: {e}")))?;
        let bundle = tx.ironwood_bundle().ok_or_else(|| {
            EngineError::Corrupt(format!("payout transaction {txid} has no Ironwood bundle"))
        })?;
        let ovks = [
            self.fvk.to_ovk(Scope::External),
            self.fvk.to_ovk(Scope::Internal),
        ];
        // (action index, recipient, value, rseed, note version) of every
        // recoverable output.
        let mut recovered = Vec::new();
        for (index, action) in bundle.actions().iter().enumerate() {
            for ovk in &ovks {
                if let Some((note, _, _)) = try_output_recovery_with_ovk(
                    &IronwoodDomain::for_action(action),
                    ovk,
                    action,
                    action.cv_net(),
                    &action.encrypted_note().out_ciphertext,
                ) {
                    recovered.push((
                        index,
                        note.recipient().to_raw_address_bytes(),
                        note.value().inner(),
                        *note.rseed().as_bytes(),
                        match note.version() {
                            orchard::note::NoteVersion::V2 => 2u8,
                            orchard::note::NoteVersion::V3 => 3u8,
                        },
                    ));
                    break;
                }
            }
        }
        let mut outputs = Vec::with_capacity(settlement.payouts.len());
        let mut used = vec![false; recovered.len()];
        for (payout_index, payout) in settlement.payouts.iter().enumerate() {
            // Unclaimable payouts have no output (see `payable_receiver`).
            let Ok(address) = payable_receiver(game.params.network, &self.sealing, payout) else {
                continue;
            };
            let receiver = address.to_raw_address_bytes();
            let found = recovered
                .iter()
                .enumerate()
                .find(|(i, r)| !used[*i] && r.1 == receiver && r.2 == payout.net);
            let Some((i, (action_index, recipient, value, rseed, version))) = found else {
                return Err(EngineError::Corrupt(format!(
                    "payout {payout_index} of {txid} has no matching output"
                )));
            };
            used[i] = true;
            outputs.push(zeek_core::proof::PayoutOpening {
                payout_index: u32::try_from(payout_index)
                    .map_err(|_| EngineError::Overflow("payout index"))?,
                action_index: u32::try_from(*action_index)
                    .map_err(|_| EngineError::Overflow("action index"))?,
                recipient: hex::encode(recipient),
                value: *value,
                rseed: hex::encode(rseed),
                note_version: *version,
            });
        }
        Ok(zeek_core::proof::PayoutOpenings {
            txid: txid.to_owned(),
            mined_height: height,
            outputs,
        })
    }

    /// Probes every keyholder's health endpoint, then records the results.
    pub async fn probe_keyholders(&self, store: &Store) -> Result<(), EngineError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut results = Vec::with_capacity(self.settings.keyholders.len());
        for keyholder in &self.settings.keyholders {
            let url = format!("{}/v1/health", keyholder.url);
            let outcome = self
                .http
                .get(&url)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.error_for_status().map_err(|e| e.to_string()));
            results.push((keyholder.identifier, outcome.err()));
        }
        store.write(|db| {
            db.transaction(|tx| {
                for (identifier, error) in &results {
                    match error {
                        None => tx.set_keyholder_health(*identifier, now, true, "")?,
                        Some(e) => tx.set_keyholder_health(*identifier, now, false, e)?,
                    }
                }
                Ok(())
            })
        })
    }

    /// Identifiers of the configured keyholders and the signing threshold.
    pub fn keyholder_ids(&self) -> (Vec<u16>, u16) {
        (
            self.settings
                .keyholders
                .iter()
                .map(|k| k.identifier)
                .collect(),
            self.settings.threshold,
        )
    }

    /// Brings the vault wallet up to the chain tip, so the ledger
    /// (`GET /v1/vault/ledger`) counts every dig the Cache counts. Payouts
    /// sync it too; between them the engine calls this every few minutes.
    /// Returns the wallet's chain tip after the sync.
    pub async fn sync_vault_wallet(&self) -> Result<u32, EngineError> {
        let (mut wallet_db, account) = self.vault_wallet()?;
        let (index, mut client) = self.pool.client().await?;
        if let Err(e) = wallet::sync(&mut wallet_db, &mut client).await {
            self.pool.failed(index, &e);
            return Err(e.into());
        }
        Ok(wallet::balance(&wallet_db, account)?.chain_tip)
    }

    /// The vault's address.
    pub fn vault_address(&self) -> &str {
        &self.vault.address
    }

    /// Opens the vault wallet and returns its account.
    fn vault_wallet(&self) -> Result<(wallet::Db, zcash_client_sqlite::AccountUuid), EngineError> {
        self.ledger_view().open()
    }

    /// A read-only view of the vault wallet for the API's ledger endpoint,
    /// usable while a payout holds the coordinator.
    pub fn ledger_view(&self) -> LedgerView {
        LedgerView {
            data_dir: self.settings.vault_data_dir.clone(),
            params: self.params,
            ufvk: self.ufvk.clone(),
            cached: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Pays the earliest settlement that is still owed, if any.
    ///
    /// The transaction spends exactly the round's charged dig notes (see
    /// [`zeek_core::settle::payout_inputs`]) plus the fewest operator-float
    /// notes needed when those do not cover the payouts and the fee (a Cache
    /// payout), pays every payout output and returns one change output to the
    /// vault's internal address. See [`plan_inputs`] for the shape and the fee
    /// coverage argument.
    ///
    /// Wallet sync, proving, keyholder signing and broadcast all run with no
    /// engine database connection held; the payout record is written in
    /// short steps around them (only this loop writes payouts).
    pub async fn pay_next(
        &mut self,
        store: &Store,
        processor: &Processor,
        game: &GameConfig,
    ) -> Result<Option<PayoutReport>, EngineError> {
        let rounds = store.read(payable_rounds)?;
        let now = std::time::Instant::now();
        let Some(round) = self.retry.next(&rounds, now) else {
            return Ok(None);
        };
        match self.pay_round(round, store, processor, game).await {
            Ok(report) => {
                self.retry.succeeded(round);
                Ok(report)
            }
            Err(e) => {
                let wait = self.retry.failed(round, now);
                tracing::warn!(
                    round,
                    retry_in_secs = wait.as_secs(),
                    "payout attempt failed; later rounds are tried meanwhile"
                );
                Err(e)
            }
        }
    }

    async fn pay_round(
        &mut self,
        round: u32,
        store: &Store,
        processor: &Processor,
        game: &GameConfig,
    ) -> Result<Option<PayoutReport>, EngineError> {
        let row = store
            .read(|db| db.settlement(round))?
            .ok_or_else(|| EngineError::Corrupt(format!("settlement {round} vanished")))?;
        if row.settlement.payouts.is_empty() {
            store.write(|db| db.set_plan_status(round, PlanStatus::NothingToPay))?;
            tracing::info!(round, "settlement has no payout; marked nothing_to_pay");
            return Ok(None);
        }
        if let Some(existing) = store.read(|db| db.payout(round))? {
            return Err(EngineError::AlreadyPaid(vec![existing.round]));
        }

        // Outputs: one per payable payout, memo naming the round and reason.
        // A payout whose receiver cannot be paid is left in the vault (the
        // keyholders apply the same rule; see `payable_receiver`).
        let mut outputs = Vec::with_capacity(row.settlement.payouts.len() + 1);
        let mut unclaimable = Vec::new();
        for (index, payout) in row.settlement.payouts.iter().enumerate() {
            // Opened only here, in memory; never stored or logged.
            let recipient = match payable_receiver(game.params.network, &self.sealing, payout) {
                Ok(address) => address,
                Err(reason) => {
                    tracing::warn!(round, payout = index, %reason, net = payout.net, "payout has no payable receiver; its value stays in the vault");
                    unclaimable.push(UnclaimablePayout {
                        payout_index: u32::try_from(index)
                            .map_err(|_| EngineError::Overflow("payout index"))?,
                        net: payout.net,
                        reason,
                    });
                    continue;
                }
            };
            let text = match payout.reason {
                PayoutReason::Win { .. } => format!("Zeek round {round}: treasure"),
                PayoutReason::Refund(_) => format!("Zeek round {round}: refund"),
            };
            outputs.push(ShapedOutput {
                recipient,
                value: Zatoshis::from_u64(payout.net)
                    .map_err(|_| EngineError::Overflow("payout amount"))?,
                memo: MemoBytes::from_bytes(text.as_bytes())
                    .map_err(|e| EngineError::Config(format!("memo: {e}")))?,
                is_change: false,
            });
        }
        if outputs.is_empty() {
            store.write(|db| {
                db.transaction(|tx| {
                    tx.set_unclaimable(round, &unclaimable)?;
                    tx.set_plan_status(round, PlanStatus::NothingToPay)
                })
            })?;
            tracing::warn!(
                round,
                "no payout of this round is payable; marked nothing_to_pay"
            );
            return Ok(None);
        }
        #[cfg(feature = "rogue-demo")]
        self.rogue_tamper(&mut outputs, round)?;

        // Vault wallet: sync, balance, ledger.
        let (mut wallet_db, account) = self.vault_wallet()?;
        {
            let (index, mut client) = self.pool.client().await?;
            if let Err(e) = wallet::sync(&mut wallet_db, &mut client).await {
                self.pool.failed(index, &e);
                return Err(e.into());
            }
        }
        let report = wallet::balance(&wallet_db, account)?;
        let (ledger_before, charged) = store.read(|db| {
            Ok::<_, EngineError>((
                ledger::compute(
                    db,
                    processor,
                    report.ironwood.total.into_u64(),
                    report.chain_tip,
                )?,
                processor.payout_inputs(db, &row)?,
            ))
        })?;

        // Inputs: the round's charged notes, then the float notes the plan needs.
        let charged: Vec<PlannedNote> = charged
            .into_iter()
            .map(|note| PlannedNote {
                id: NoteId {
                    txid: note.txid,
                    output_index: note.output_index,
                },
                value: note.value,
            })
            .collect();
        let spendable = |n: &wallet::NoteReport| n.pool == ValuePool::Ironwood && n.spendable;
        for input in &charged {
            let known = report
                .notes
                .iter()
                .find(|n| n.txid == input.id.txid && n.output_index == input.id.output_index);
            let reason = match known {
                None => Some("the vault wallet does not hold it".to_owned()),
                Some(n) if !spendable(n) => Some(format!(
                    "not spendable yet ({} confirmations)",
                    n.confirmations.unwrap_or(0)
                )),
                Some(n) if n.value.into_u64() != input.value => Some(format!(
                    "the wallet reports {} zatoshis, the settlement {}",
                    n.value.into_u64(),
                    input.value
                )),
                Some(_) => None,
            };
            if let Some(reason) = reason {
                return Err(EngineError::InputUnspendable {
                    round,
                    txid: input.id.txid.clone(),
                    index: input.id.output_index,
                    reason,
                });
            }
        }
        // Float is never a note the game knows (another round's dig, a
        // shout, a forfeit): the keyholders refuse those (review N8).
        let known = store.read(|db| db.known_note_ids())?;
        let float_candidates: Vec<PlannedNote> = report
            .notes
            .iter()
            .filter(|n| spendable(n))
            .filter(|n| !known.contains(&(n.txid.clone(), n.output_index)))
            .map(|n| PlannedNote {
                id: NoteId {
                    txid: n.txid.clone(),
                    output_index: n.output_index,
                },
                value: n.value.into_u64(),
            })
            .collect();
        let nets: Vec<u64> = outputs.iter().map(|o| o.value.into_u64()).collect();
        let plan = plan_inputs(&game.params, &charged, &nets, &float_candidates)
            .map_err(|e| e.for_rounds(vec![round]))?;
        let collected = row.settlement.total_fees;
        let deficit = plan.fee.saturating_sub(collected);
        if deficit > ledger_before.operator_float {
            return Err(EngineError::OperatorFloatExhausted {
                rounds: vec![round],
                deficit,
                float: ledger_before.operator_float,
            });
        }
        outputs.push(ShapedOutput {
            recipient: self.fvk.address_at(0u32, Scope::Internal),
            value: Zatoshis::from_u64(plan.change)
                .map_err(|_| EngineError::Overflow("change amount"))?,
            memo: MemoBytes::empty(),
            is_change: true,
        });
        tracing::info!(
            round,
            payouts = nets.len(),
            charged_inputs = charged.len(),
            float_inputs = plan.float_count,
            fee = plan.fee,
            collected,
            deficit,
            change = plan.change,
            float = ledger_before.operator_float,
            "payout transaction planned"
        );

        // Build at the wallet's next height, anchored at its newest checkpoint
        // that gives every input the confirmations the policy requires.
        let expected_target = BlockHeight::from_u32(report.chain_tip) + 1;
        let circuit = send::circuit_version_for(&self.params, expected_target)?;
        let verifying_key = VerifyingKey::build(circuit);
        let min_confirmations = NonZeroU32::new(report.min_confirmations).ok_or_else(|| {
            EngineError::Config("confirmations policy requires at least 1".into())
        })?;
        let PreparedPczt {
            pczt,
            target_height,
            fee,
        } = send::build_pczt_from_notes(
            &mut wallet_db,
            &self.fvk,
            &plan.spends,
            &outputs,
            min_confirmations,
        )?;
        if send::circuit_version_for(&self.params, target_height)? != circuit {
            return Err(zeek_vault::VaultError::CircuitMismatch {
                built: format!("{circuit:?}"),
                required: format!(
                    "{:?}",
                    send::circuit_version_for(&self.params, target_height)?
                ),
            }
            .into());
        }
        if fee.into_u64() != plan.fee {
            return Err(EngineError::Server(format!(
                "builder fee {} differs from the planned fee {}",
                fee.into_u64(),
                plan.fee
            )));
        }
        // Proving takes about a minute of CPU: it runs on the blocking pool so
        // the API's workers stay free.
        let pczt =
            tokio::task::spawn_blocking(move || send::prove(pczt, &ProvingKey::build(circuit)))
                .await
                .map_err(|e| EngineError::Server(format!("proving task: {e}")))??;

        // Signing session.
        let mut session = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut session);
        let pending = send::pending_spends(&pczt)?;
        let sighash = Signer::new(pczt.clone())
            .map_err(|e| EngineError::Server(format!("signer: {e:?}")))?
            .shielded_sighash();
        store.write(|db| {
            db.insert_payout(&PayoutRecord {
                round,
                state: PayoutState::Signing,
                session: hex::encode(session),
                txid: None,
                height: report.chain_tip,
                fee: fee.into_u64(),
                signers: vec![],
            })
        })?;
        let mut events = Vec::new();
        let result = self
            .sign_with_keyholders(&session, pczt, &pending, sighash, &[round], &mut events)
            .await;
        store.write(|db| {
            db.transaction(|tx| {
                for event in &events {
                    tx.insert_keyholder_event(event)?;
                }
                Ok(())
            })
        })?;
        let signed = match result {
            Ok(s) => s,
            Err(e) => {
                // Nothing was broadcast: forget the attempt so it can be retried.
                store.write(|db| db.delete_payout_if_signing(round))?;
                return Err(e);
            }
        };
        let (pczt, signers) = signed;
        // The record names the signers before anything leaves this process,
        // and the broadcast is recorded before the wallet is told about it: a
        // wallet storage failure must never hide a transaction that is on
        // the network (the wallet learns about it from the chain anyway).
        store.write(|db| db.set_payout_signers(round, &signers))?;
        let (index, mut client) = self.pool.client().await?;
        let transaction = match send::extract_and_broadcast(
            &self.params,
            &mut client,
            pczt,
            &verifying_key,
            target_height,
        )
        .await
        {
            Ok(t) => t,
            Err(e) => {
                if !matches!(e, zeek_vault::VaultError::BroadcastRejected { .. }) {
                    self.pool.failed(index, &e);
                }
                return Err(e.into());
            }
        };
        let txid = transaction.txid().to_string();
        store.write(|db| {
            db.transaction(|tx| {
                tx.mark_payout_broadcast(round, &txid, report.chain_tip, &signers)?;
                tx.set_unclaimable(round, &unclaimable)?;
                tx.set_plan_status(round, PlanStatus::Paid)
            })
        })?;
        tracing::info!(round, %txid, ?signers, "payout broadcast");
        if let Err(e) = send::store_transaction(&mut wallet_db, &transaction) {
            tracing::warn!(round, %txid, error = %e, "the vault wallet could not record the payout yet; it will pick it up from the chain");
        }
        Ok(Some(PayoutReport {
            round,
            txid,
            fee: fee.into_u64(),
            signers,
            outputs: row.settlement.payouts.len(),
        }))
    }

    #[cfg(feature = "rogue-demo")]
    fn rogue_tamper(&self, outputs: &mut [ShapedOutput], round: u32) -> Result<(), EngineError> {
        let Some(address) = &self.settings.rogue_address else {
            return Ok(());
        };
        let Some(first) = outputs.first_mut() else {
            return Ok(());
        };
        let receiver = zeek_core::memo::PayoutReceiver::from_unified_address(
            zeek_core::params::Network::Test,
            address,
        )
        .map_err(|e| EngineError::Config(format!("{ROGUE_VAR}: {e}")))?;
        first.recipient = Option::from(orchard::Address::from_raw_address_bytes(&receiver.orchard))
            .ok_or_else(|| EngineError::Config(format!("{ROGUE_VAR}: not an Orchard address")))?;
        tracing::warn!(
            round,
            "ROGUE DEMO: redirected the first payout to the operator-controlled address"
        );
        Ok(())
    }

    /// Runs rounds 1 and 2 with the keyholders and applies the signatures.
    async fn sign_with_keyholders(
        &self,
        session: &SessionId,
        pczt: Pczt,
        pending: &[PendingSpend],
        sighash: [u8; 32],
        rounds: &[u32],
        events: &mut Vec<crate::db::KeyholderEvent>,
    ) -> Result<(Pczt, Vec<u16>), EngineError> {
        use crate::db::{KeyholderEvent, KeyholderOutcome};
        let needed = usize::from(self.settings.threshold);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut record = |keyholder: u16, outcome: KeyholderOutcome, reason: String| {
            events.push(KeyholderEvent {
                time: now,
                session: hex::encode(session),
                keyholder,
                rounds: rounds.to_vec(),
                outcome,
                reason,
            });
        };
        let request = protocol::seal(
            &self.identity,
            session,
            COORDINATOR_ID,
            kind::SIGN_REQUEST,
            &SignRequest {
                pczt: pczt
                    .clone()
                    .serialize()
                    .map_err(|e| EngineError::Server(format!("serialize PCZT: {e:?}")))?,
                rounds: rounds.to_vec(),
            },
        )?;

        // Round 1: ask everyone, keep the first `threshold` approvals.
        let mut approvals: BTreeMap<u16, Vec<SigningCommitments>> = BTreeMap::new();
        let mut details = Vec::new();
        let mut round1 = Vec::with_capacity(self.settings.keyholders.len());
        for keyholder in &self.settings.keyholders {
            round1.push(self.post(keyholder, "/v1/sign/request", &request).await);
        }
        for (keyholder, result) in self.settings.keyholders.iter().zip(round1) {
            match result {
                Ok(env) => match protocol::open::<SignCommitments>(
                    &env,
                    &Expect {
                        kind: kind::SIGN_COMMITMENTS,
                        sender: keyholder.identifier,
                        sender_key: &keyholder.identity,
                        session: Some(session),
                    },
                ) {
                    Ok((_, msg)) => {
                        let parsed: Result<Vec<SigningCommitments>, _> = msg
                            .commitments
                            .iter()
                            .map(|c| SigningCommitments::deserialize(c))
                            .collect();
                        match parsed {
                            Ok(c) if c.len() == pending.len() => {
                                tracing::info!(
                                    keyholder = keyholder.identifier,
                                    "approved the payout"
                                );
                                record(
                                    keyholder.identifier,
                                    KeyholderOutcome::Approved,
                                    String::new(),
                                );
                                approvals.insert(keyholder.identifier, c);
                            }
                            Ok(c) => {
                                let reason =
                                    format!("{} commitments for {} spends", c.len(), pending.len());
                                record(
                                    keyholder.identifier,
                                    KeyholderOutcome::Unreachable,
                                    reason.clone(),
                                );
                                details
                                    .push(format!("keyholder {}: {reason}", keyholder.identifier));
                            }
                            Err(e) => {
                                record(
                                    keyholder.identifier,
                                    KeyholderOutcome::Unreachable,
                                    format!("bad commitments: {e}"),
                                );
                                details.push(format!(
                                    "keyholder {}: bad commitments: {e}",
                                    keyholder.identifier
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        record(
                            keyholder.identifier,
                            KeyholderOutcome::Unreachable,
                            e.to_string(),
                        );
                        details.push(format!("keyholder {}: {e}", keyholder.identifier));
                    }
                },
                Err(e) => {
                    tracing::warn!(keyholder = keyholder.identifier, error = %e, "keyholder did not approve");
                    match &e {
                        EngineError::Refused { reason, .. } => {
                            record(
                                keyholder.identifier,
                                KeyholderOutcome::Refused,
                                reason.clone(),
                            );
                        }
                        other => record(
                            keyholder.identifier,
                            KeyholderOutcome::Unreachable,
                            other.to_string(),
                        ),
                    }
                    details.push(format!("keyholder {}: {e}", keyholder.identifier));
                }
            }
        }
        if approvals.len() < needed {
            return Err(EngineError::NotEnoughSigners {
                approved: approvals.len(),
                needed,
                details: details.join("; "),
            });
        }
        let chosen: Vec<u16> = approvals.keys().take(needed).copied().collect();

        // Round 2: one signing package per spend, built from the chosen signers.
        let mut packages = Vec::with_capacity(pending.len());
        for (i, _) in pending.iter().enumerate() {
            let mut commitments = BTreeMap::new();
            for id in &chosen {
                commitments.insert(frost::identifier(*id)?, approvals[id][i]);
            }
            packages.push(SigningPackage::new(commitments, &sighash));
        }
        let serialized: Result<Vec<Vec<u8>>, _> = packages.iter().map(|p| p.serialize()).collect();
        let message = protocol::seal(
            &self.identity,
            session,
            COORDINATOR_ID,
            kind::SIGN_PACKAGES,
            &SignPackages {
                packages: serialized.map_err(zeek_vault::VaultError::from)?,
            },
        )?;
        let mut shares: BTreeMap<u16, Vec<frost::SignatureShare>> = BTreeMap::new();
        for id in &chosen {
            let keyholder = self
                .settings
                .keyholders
                .iter()
                .find(|k| k.identifier == *id)
                .ok_or_else(|| EngineError::Config(format!("keyholder {id} vanished")))?;
            let env = self.post(keyholder, "/v1/sign/round2", &message).await?;
            let (_, msg) = protocol::open::<SignShares>(
                &env,
                &Expect {
                    kind: kind::SIGN_SHARES,
                    sender: *id,
                    sender_key: &keyholder.identity,
                    session: Some(session),
                },
            )?;
            let parsed: Result<Vec<frost::SignatureShare>, _> = msg
                .shares
                .iter()
                .map(|s| frost::SignatureShare::deserialize(s))
                .collect();
            let parsed = parsed.map_err(zeek_vault::VaultError::from)?;
            if parsed.len() != pending.len() {
                return Err(EngineError::Protocol(format!(
                    "keyholder {id} returned {} shares for {} spends",
                    parsed.len(),
                    pending.len()
                )));
            }
            shares.insert(*id, parsed);
        }

        // Aggregate per spend, verify against ak·alpha and the action's rk, apply.
        let mut signer =
            Signer::new(pczt).map_err(|e| EngineError::Server(format!("signer: {e:?}")))?;
        for (i, spend) in pending.iter().enumerate() {
            let mut spend_shares = BTreeMap::new();
            for id in &chosen {
                spend_shares.insert(frost::identifier(*id)?, shares[id][i]);
            }
            let produced = frost::aggregate(
                &packages[i],
                &spend_shares,
                &self.public_key_package,
                &spend.alpha,
            )?;
            if produced.randomized_verifying_key != spend.rk {
                return Err(zeek_vault::VaultError::RkMismatch {
                    pool: wallet::pool_name(spend.pool).to_owned(),
                    index: spend.index,
                }
                .into());
            }
            let signature = redpallas::Signature::<SpendAuth>::from(produced.signature);
            match spend.pool {
                ValuePool::Orchard => signer.apply_orchard_signature(spend.index, signature),
                ValuePool::Ironwood => signer.apply_ironwood_signature(spend.index, signature),
            }
            .map_err(|e| {
                EngineError::Server(format!("apply signature to action {}: {e:?}", spend.index))
            })?;
        }
        Ok((signer.finish(), chosen))
    }

    async fn post(
        &self,
        keyholder: &Keyholder,
        path: &str,
        envelope: &Envelope,
    ) -> Result<Envelope, EngineError> {
        let url = format!("{}{path}", keyholder.url);
        let response = self
            .http
            .post(&url)
            .json(envelope)
            .send()
            .await
            .map_err(|e| EngineError::Relay(format!("{url}: {e}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            let env: Envelope = response
                .json()
                .await
                .map_err(|e| EngineError::Relay(format!("{url}: {e}")))?;
            let (_, refusal) = protocol::open::<Refusal>(
                &env,
                &Expect {
                    kind: kind::REFUSAL,
                    sender: keyholder.identifier,
                    sender_key: &keyholder.identity,
                    session: None,
                },
            )?;
            return Err(EngineError::Refused {
                identifier: keyholder.identifier,
                reason: refusal.reason,
            });
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(EngineError::Relay(format!("{url}: HTTP {status} {body}")));
        }
        response
            .json()
            .await
            .map_err(|e| EngineError::Relay(format!("{url}: {e}")))
    }
}

/// A note the plan may spend, with its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedNote {
    pub id: NoteId,
    pub value: u64,
}

/// The shape of a payout transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputPlan {
    /// Every spend in order: the charged dig notes, then the float notes.
    pub spends: Vec<NoteId>,
    /// How many of `spends` are operator-float notes.
    pub float_count: usize,
    /// ZIP 317 fee of the shape (`spends` in, payouts plus change out).
    pub fee: u64,
    /// Value of the change output (always positive).
    pub change: u64,
}

/// Errors from planning, before the rounds are known.
#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    /// Even every float note together cannot pay the outputs and the fee.
    Insufficient {
        needed: u64,
        available: u64,
    },
    /// The inputs balance to the zatoshi and no float note is left to create
    /// a change output.
    ZeroChange,
    Overflow,
}

impl PlanError {
    fn for_rounds(self, rounds: Vec<u32>) -> EngineError {
        match self {
            PlanError::Insufficient { needed, available } => EngineError::InsufficientVaultFunds {
                rounds,
                needed,
                available,
            },
            PlanError::ZeroChange => EngineError::Server(format!(
                "the payout for rounds {rounds:?} balances with no change output and no float note is available to add one"
            )),
            PlanError::Overflow => EngineError::Overflow("payout plan"),
        }
    }
}

/// Decides which notes a payout transaction spends.
///
/// Inputs are the round's `charged` notes (every one of them, in the given
/// order) and, only if their value does not cover the payouts plus the ZIP 317
/// fee of the resulting shape, the fewest operator-float notes from
/// `float_candidates`, largest first. The transaction always ends with one
/// change output; when the inputs would balance to the zatoshi, the smallest
/// remaining float note is added so that a change output exists.
///
/// # Fee coverage
///
/// With `n` charged notes, `k` float notes and `p` payouts the fee is
/// `marginal · max(2, n + k, p + 1)` and the rules collected
/// `marginal · (n + p)` (a dig fee per charged note, an output fee per
/// payout; see [`zeek_core::settle`]). For `k = 0` the fee is always covered.
/// A float note is only needed when the payouts exceed the charged notes'
/// value, i.e. for a Cache payout; each float note raises the fee by one
/// marginal fee once `n + k` exceeds `p + 1`, so the fee stays covered
/// exactly while `k ≤ p` ([`zeek_core::settle::fees_cover`]). Since float
/// notes are taken largest first, `k` is the smallest count that funds the
/// outputs. Any fee beyond the collected fees (only possible for `k > p`) is
/// a deficit the caller charges to the operator float, never to the Cache or
/// to the players' obligations; `pay_next` refuses the transaction when the
/// float cannot cover it.
pub fn plan_inputs(
    params: &GameParams,
    charged: &[PlannedNote],
    payout_nets: &[u64],
    float_candidates: &[PlannedNote],
) -> Result<InputPlan, PlanError> {
    let sum = |values: &[u64]| -> Result<u64, PlanError> {
        values
            .iter()
            .try_fold(0u64, |acc, v| acc.checked_add(*v))
            .ok_or(PlanError::Overflow)
    };
    let charged_values: Vec<u64> = charged.iter().map(|n| n.value).collect();
    let mut inputs = sum(&charged_values)?;
    let outputs = sum(payout_nets)?;
    let payouts = u64::try_from(payout_nets.len()).map_err(|_| PlanError::Overflow)?;

    // Largest first; ties broken by note identity so every run agrees.
    let mut floats: Vec<&PlannedNote> = float_candidates.iter().collect();
    floats.sort_by(|a, b| {
        b.value
            .cmp(&a.value)
            .then_with(|| a.id.txid.cmp(&b.id.txid))
            .then_with(|| a.id.output_index.cmp(&b.id.output_index))
    });
    let available = inputs
        .checked_add(sum(&floats.iter().map(|n| n.value).collect::<Vec<_>>())?)
        .ok_or(PlanError::Overflow)?;

    let mut chosen: Vec<&PlannedNote> = Vec::new();
    loop {
        let spends =
            u64::try_from(charged.len() + chosen.len()).map_err(|_| PlanError::Overflow)?;
        let fee =
            settle::zip317_fee(params, spends, payouts + 1).map_err(|_| PlanError::Overflow)?;
        let needed = outputs.checked_add(fee).ok_or(PlanError::Overflow)?;
        if inputs > needed {
            let change = inputs - needed;
            let mut spends: Vec<NoteId> = charged.iter().map(|n| n.id.clone()).collect();
            spends.extend(chosen.iter().map(|n| n.id.clone()));
            return Ok(InputPlan {
                spends,
                float_count: chosen.len(),
                fee,
                change,
            });
        }
        // Short, or exactly balanced: take one more float note. Largest first
        // when short; the smallest when only a change output is missing.
        let next = if inputs == needed {
            floats.pop()
        } else if floats.is_empty() {
            None
        } else {
            Some(floats.remove(0))
        };
        match next {
            Some(note) => {
                inputs = inputs.checked_add(note.value).ok_or(PlanError::Overflow)?;
                chosen.push(note);
            }
            None if inputs == needed => return Err(PlanError::ZeroChange),
            None => return Err(PlanError::Insufficient { needed, available }),
        }
    }
}

/// Settled rounds that are owed and have no payout record, earliest first.
pub fn payable_rounds(db: &Db) -> Result<Vec<u32>, EngineError> {
    let mut rounds = Vec::new();
    for round in db.settled_rounds()? {
        let row = db
            .settlement(round)?
            .ok_or_else(|| EngineError::Corrupt(format!("settlement {round} vanished")))?;
        if row.status != PlanStatus::AwaitingSignatures {
            continue;
        }
        if db.payout(round)?.is_some() {
            // In progress or stuck: leave it to the operator.
            continue;
        }
        rounds.push(round);
    }
    Ok(rounds)
}

/// Retry schedule of rounds whose payout attempt failed, so that one round
/// that keeps failing never holds up the rounds after it.
#[derive(Default)]
pub struct RetryBackoff {
    failed: BTreeMap<u32, (u32, std::time::Instant)>,
}

impl RetryBackoff {
    /// First wait after a failure; it doubles per failure up to `MAX`.
    const FIRST: Duration = Duration::from_secs(60);
    const MAX: Duration = Duration::from_secs(30 * 60);

    /// The first of `rounds` that may be attempted at `now`.
    pub fn next(&self, rounds: &[u32], now: std::time::Instant) -> Option<u32> {
        rounds
            .iter()
            .copied()
            .find(|r| self.failed.get(r).is_none_or(|(_, at)| *at <= now))
    }

    /// Records a failed attempt of `round` at `now`.
    pub fn failed(&mut self, round: u32, now: std::time::Instant) -> Duration {
        let failures = self
            .failed
            .get(&round)
            .map_or(0, |(n, _)| *n)
            .saturating_add(1);
        let wait = Self::FIRST
            .saturating_mul(1u32 << failures.saturating_sub(1).min(5))
            .min(Self::MAX);
        self.failed.insert(round, (failures, now + wait));
        wait
    }

    /// Forgets `round` after a successful attempt.
    pub fn succeeded(&mut self, round: u32) {
        self.failed.remove(&round);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::collection::vec;
    use proptest::prelude::*;

    fn note(tag: u8, index: u16, value: u64) -> PlannedNote {
        PlannedNote {
            id: NoteId {
                txid: format!("{tag:02x}").repeat(32),
                output_index: index,
            },
            value,
        }
    }

    fn params() -> GameParams {
        GameParams::testnet(1)
    }

    #[test]
    fn spends_exactly_the_charged_notes_when_they_suffice() {
        // Round 7's shape: 25 digs of 100,000, one payout of 2,370,000.
        let charged: Vec<_> = (0..25u16).map(|i| note(0xaa, i, 100_000)).collect();
        let floats = vec![note(0xcc, 0, 2_250_000)];
        let plan = plan_inputs(&params(), &charged, &[2_370_000], &floats).expect("plan");
        assert_eq!(plan.float_count, 0);
        assert_eq!(plan.spends.len(), 25);
        assert_eq!(plan.fee, 125_000);
        assert_eq!(plan.change, 2_500_000 - 2_370_000 - 125_000);
        // Collected under rules v2: 25 dig fees + 1 output fee.
        assert!(plan.fee <= 26 * 5_000);
    }

    #[test]
    fn adds_the_largest_float_notes_only_when_short() {
        let charged = vec![note(0xaa, 0, 100_000)];
        let floats = vec![
            note(0xcc, 0, 50_000),
            note(0xdd, 0, 1_000_000),
            note(0xee, 0, 400_000),
        ];
        // A Cache payout of 1,000,000 to the single winner.
        let plan = plan_inputs(&params(), &charged, &[1_000_000], &floats).expect("plan");
        assert_eq!(plan.float_count, 1);
        assert_eq!(plan.spends[1].txid, "dd".repeat(32));
        assert_eq!(plan.fee, 10_000, "2 spends, 2 outputs");
        assert_eq!(plan.change, 1_100_000 - 1_000_000 - 10_000);
        assert!(settle::fees_cover(&params(), 1, 1, 1).expect("cover"));
    }

    #[test]
    fn exact_balance_pulls_the_smallest_float_note_for_change() {
        // 1 spend, 1 payout, 1 change: fee 10,000; inputs equal outputs + fee.
        let charged = vec![note(0xaa, 0, 110_000)];
        let floats = vec![note(0xcc, 0, 7_000), note(0xdd, 0, 900_000)];
        let plan = plan_inputs(&params(), &charged, &[100_000], &floats).expect("plan");
        assert_eq!(plan.float_count, 1);
        assert_eq!(plan.spends[1].txid, "cc".repeat(32));
        assert_eq!(plan.change, 7_000);
        assert_eq!(
            plan_inputs(&params(), &charged, &[100_000], &[]),
            Err(PlanError::ZeroChange)
        );
    }

    #[test]
    fn a_failing_round_backs_off_and_later_rounds_go_first() {
        let start = std::time::Instant::now();
        let mut retry = RetryBackoff::default();
        let rounds = [4, 5, 6];
        assert_eq!(retry.next(&rounds, start), Some(4));
        assert_eq!(retry.failed(4, start), Duration::from_secs(60));
        assert_eq!(retry.next(&rounds, start), Some(5));
        assert_eq!(
            retry.next(&rounds, start + Duration::from_secs(61)),
            Some(4)
        );
        assert_eq!(retry.failed(4, start), Duration::from_secs(120));
        for _ in 0..10 {
            retry.failed(4, start);
        }
        assert_eq!(retry.failed(4, start), Duration::from_secs(1_800), "capped");
        retry.succeeded(4);
        assert_eq!(retry.next(&rounds, start), Some(4));
    }

    #[test]
    fn refuses_when_even_the_float_cannot_pay() {
        let charged = vec![note(0xaa, 0, 100_000)];
        let floats = vec![note(0xcc, 0, 50_000)];
        assert_eq!(
            plan_inputs(&params(), &charged, &[1_000_000], &floats),
            Err(PlanError::Insufficient {
                needed: 1_010_000,
                available: 150_000
            })
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        /// Random shapes: the plan balances with positive change, is
        /// deterministic, uses float notes only when the charged notes are
        /// short (or balance exactly), takes them largest first, and keeps the
        /// fee within the collected fees whenever it uses at most one float
        /// note per payout.
        #[test]
        fn plans_balance_and_cover_fees(
            charged_values in vec(100_000u64..5_000_000, 1..30),
            payout_fractions in vec(1u64..=100, 1..12),
            float_values in vec(1_000u64..20_000_000, 0..6),
            cache_payout in any::<bool>(),
        ) {
            let p = params();
            let charged: Vec<PlannedNote> = charged_values
                .iter()
                .enumerate()
                .map(|(i, v)| note(0xaa, i as u16, *v))
                .collect();
            let total: u64 = charged_values.iter().sum();
            // Payouts: fractions of the charged value, or a Cache payout above it.
            let base = if cache_payout { total * 3 } else { total * 95 / 100 };
            let fraction_sum: u64 = payout_fractions.iter().sum();
            let nets: Vec<u64> = payout_fractions
                .iter()
                .map(|f| base * f / fraction_sum / (payout_fractions.len() as u64).max(1) + 1)
                .collect();
            let floats: Vec<PlannedNote> = float_values
                .iter()
                .enumerate()
                .map(|(i, v)| note(0xcc, i as u16, *v))
                .collect();
            let result = plan_inputs(&p, &charged, &nets, &floats);
            let again = plan_inputs(&p, &charged, &nets, &floats);
            prop_assert_eq!(&result, &again, "deterministic");
            let Ok(plan) = result else {
                // Only a genuine shortfall or an exact balance without floats fails.
                let fee0 = settle::zip317_fee(&p, charged.len() as u64, nets.len() as u64 + 1).expect("fee");
                let needed0 = nets.iter().sum::<u64>() + fee0;
                prop_assert!(total <= needed0);
                return Ok(());
            };
            let n = charged.len() as u64;
            let k = plan.float_count as u64;
            let outputs = nets.len() as u64 + 1;
            prop_assert_eq!(plan.fee, settle::zip317_fee(&p, n + k, outputs).expect("fee"));
            // Balance.
            let spent: u64 = plan.spends.iter().map(|id| {
                charged.iter().chain(floats.iter()).find(|c| c.id == *id).expect("known").value
            }).sum();
            prop_assert_eq!(spent, nets.iter().sum::<u64>() + plan.fee + plan.change);
            prop_assert!(plan.change > 0);
            // Charged notes first, all of them, in order.
            for (i, c) in charged.iter().enumerate() {
                prop_assert_eq!(&plan.spends[i], &c.id);
            }
            // Float notes only when the charged notes are short or exact.
            let fee_without = settle::zip317_fee(&p, n, outputs).expect("fee");
            if k > 0 {
                prop_assert!(total <= nets.iter().sum::<u64>() + fee_without);
            }
            // Fee coverage: no float notes, or at most one per payout.
            let collected = n * p.dig_fee_zat + nets.len() as u64 * p.marginal_fee_zat;
            if k <= nets.len() as u64 {
                prop_assert!(plan.fee <= collected, "fee {} > collected {}", plan.fee, collected);
            }
            prop_assert_eq!(settle::fees_cover(&p, n, k, nets.len() as u64).expect("cover"), k <= nets.len() as u64);
        }
    }
}
