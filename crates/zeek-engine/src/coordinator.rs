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
//! broadcast only after the record exists, so no round is ever paid twice.

use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use orchard::circuit::{ProvingKey, VerifyingKey};
use orchard::primitives::redpallas::{self, SpendAuth};
use orchard::ValuePool;
use pczt::roles::signer::Signer;
use pczt::Pczt;
use rand_core::RngCore;
use zcash_address::ZcashAddress;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::consensus::BlockHeight;
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zeek_core::memo::DigMemo;
use zeek_core::seal::SealingPrivateKey;
use zeek_core::settle::PayoutReason;
use zeek_vault::frost::{self, PublicKeyPackage, SigningCommitments, SigningPackage};
use zeek_vault::lightwalletd::{self, Client};
use zeek_vault::send::{self, PendingSpend, PreparedPczt};
use zeek_vault::store::{SecretsDir, VaultFile};
use zeek_vault::wallet;
use zip321::{Payment, TransactionRequest};

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
    sealing: SealingPrivateKey,
    http: reqwest::Client,
    lightwalletd_url: String,
    client: Option<Client>,
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
    pub fn new(settings: CoordinatorSettings, lightwalletd_url: &str) -> Result<Self, EngineError> {
        let identity = Identity::load(&settings.identity_path)?;
        let secrets = SecretsDir::new(&settings.vault_secrets_dir);
        let vault = secrets.load_vault()?;
        let public_key_package = vault.public_key_package()?;
        let ufvk = vault.ufvk()?;
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
            sealing,
            http,
            lightwalletd_url: lightwalletd_url.to_owned(),
            client: None,
        })
    }

    /// The vault's address.
    pub fn vault_address(&self) -> &str {
        &self.vault.address
    }

    async fn client(&mut self) -> Result<&mut Client, EngineError> {
        if self.client.is_none() {
            self.client = Some(lightwalletd::connect(&self.lightwalletd_url).await?);
        }
        self.client
            .as_mut()
            .ok_or_else(|| EngineError::Server("no lightwalletd client".into()))
    }

    /// Opens the vault wallet and returns its account.
    fn vault_wallet(&self) -> Result<(wallet::Db, zcash_client_sqlite::AccountUuid), EngineError> {
        let db = wallet::open(&self.settings.vault_data_dir)?;
        let account = wallet::find_account(&db, &self.ufvk)?
            .ok_or_else(|| EngineError::Config("the vault wallet has no account for this vault; run `zeek-vault sync` with the vault's secrets first".into()))?;
        Ok((db, account))
    }

    /// Reads the ledger from the vault wallet (without syncing).
    pub fn ledger(&self, db: &Db, processor: &Processor) -> Result<Ledger, EngineError> {
        let (wallet_db, account) = self.vault_wallet()?;
        let report = wallet::balance(&wallet_db, account)?;
        ledger::compute(
            db,
            processor,
            report.ironwood.total.into_u64(),
            report.chain_tip,
        )
    }

    /// Pays the earliest settlement that is still owed, if any.
    pub async fn pay_next(
        &mut self,
        db: &mut Db,
        processor: &Processor,
        game: &GameConfig,
    ) -> Result<Option<PayoutReport>, EngineError> {
        let Some(round) = next_payable_round(db)? else {
            return Ok(None);
        };
        let row = db
            .settlement(round)?
            .ok_or_else(|| EngineError::Corrupt(format!("settlement {round} vanished")))?;
        if row.settlement.payouts.is_empty() {
            db.set_plan_status(round, PlanStatus::NothingToPay)?;
            tracing::info!(round, "settlement has no payout; marked nothing_to_pay");
            return Ok(None);
        }
        if let Some(existing) = db.payout(round)? {
            return Err(EngineError::AlreadyPaid(vec![existing.round]));
        }

        // Outputs: one per payout, memo naming the round and reason.
        let mut payments = Vec::with_capacity(row.settlement.payouts.len());
        for payout in &row.settlement.payouts {
            let memo = DigMemo {
                round: payout.memo_round,
                tile: payout.memo_tile,
                sealed_receiver: payout.sealed_receiver.clone(),
            };
            // Opened only here, in memory; never stored or logged.
            let receiver = memo.open(game.params.network, &self.sealing).map_err(|e| {
                EngineError::Config(format!("round {round}: sealed receiver does not open: {e}"))
            })?;
            let address = ZcashAddress::try_from_encoded(
                &receiver
                    .to_unified_address()
                    .map_err(|e| EngineError::Config(e.to_string()))?,
            )
            .map_err(|e| EngineError::Config(format!("payout address: {e}")))?;
            let text = match payout.reason {
                PayoutReason::Win { .. } => format!("Zeek round {round}: treasure"),
                PayoutReason::Refund(_) => format!("Zeek round {round}: refund"),
            };
            let memo_bytes = MemoBytes::from_bytes(text.as_bytes())
                .map_err(|e| EngineError::Config(format!("memo: {e}")))?;
            let amount = Zatoshis::from_u64(payout.net)
                .map_err(|_| EngineError::Overflow("payout amount"))?;
            payments.push(
                Payment::new(address, Some(amount), Some(memo_bytes), None, None, vec![])
                    .map_err(|e| EngineError::Config(format!("payout payment: {e}")))?,
            );
        }
        #[cfg(feature = "rogue-demo")]
        self.rogue_tamper(&mut payments, round)?;
        let request = TransactionRequest::new(payments)
            .map_err(|e| EngineError::Config(format!("payment request: {e}")))?;

        // Vault wallet: sync, balance, ledger check, PCZT.
        let (mut wallet_db, account) = self.vault_wallet()?;
        {
            let client = self.client().await?;
            wallet::sync(&mut wallet_db, client).await?;
        }
        let report = wallet::balance(&wallet_db, account)?;
        let ledger_before = ledger::compute(
            db,
            processor,
            report.ironwood.total.into_u64(),
            report.chain_tip,
        )?;
        let expected_target = BlockHeight::from_u32(report.chain_tip) + 1;
        let circuit = send::circuit_version_for(expected_target)?;
        let proving_key = ProvingKey::build(circuit);
        let verifying_key = VerifyingKey::build(circuit);
        let PreparedPczt {
            pczt,
            target_height,
            fee,
        } = send::prepare_pczt(&mut wallet_db, account, request, &report)?;
        if send::circuit_version_for(target_height)? != circuit {
            return Err(zeek_vault::VaultError::CircuitMismatch {
                built: format!("{circuit:?}"),
                required: format!("{:?}", send::circuit_version_for(target_height)?),
            }
            .into());
        }
        let deficit = fee.into_u64().saturating_sub(row.settlement.total_fees);
        if deficit > ledger_before.operator_float {
            return Err(EngineError::OperatorFloatExhausted {
                rounds: vec![round],
                deficit,
                float: ledger_before.operator_float,
            });
        }
        tracing::info!(
            round,
            outputs = row.settlement.payouts.len(),
            fee = fee.into_u64(),
            collected = row.settlement.total_fees,
            deficit,
            float = ledger_before.operator_float,
            "payout transaction prepared"
        );
        let pczt = send::prove(pczt, &proving_key)?;

        // Signing session.
        let mut session = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut session);
        let pending = send::pending_spends(&pczt)?;
        let sighash = Signer::new(pczt.clone())
            .map_err(|e| EngineError::Server(format!("signer: {e:?}")))?
            .shielded_sighash();
        db.insert_payout(&PayoutRecord {
            round,
            state: PayoutState::Signing,
            session: hex::encode(session),
            txid: None,
            height: report.chain_tip,
            fee: fee.into_u64(),
            signers: vec![],
        })?;
        let signed = match self
            .sign_with_keyholders(&session, pczt, &pending, sighash, &[round])
            .await
        {
            Ok(s) => s,
            Err(e) => {
                // Nothing was broadcast: forget the attempt so it can be retried.
                db.delete_payout_if_signing(round)?;
                return Err(e);
            }
        };
        let (pczt, signers) = signed;
        let client = self.client().await?;
        let transaction = send::extract_broadcast_store(
            &mut wallet_db,
            client,
            pczt,
            &verifying_key,
            target_height,
        )
        .await?;
        let txid = transaction.txid().to_string();
        db.mark_payout_broadcast(round, &txid, report.chain_tip, &signers)?;
        db.set_plan_status(round, PlanStatus::Paid)?;
        tracing::info!(round, %txid, ?signers, "payout broadcast");
        Ok(Some(PayoutReport {
            round,
            txid,
            fee: fee.into_u64(),
            signers,
            outputs: row.settlement.payouts.len(),
        }))
    }

    #[cfg(feature = "rogue-demo")]
    fn rogue_tamper(&self, payments: &mut [Payment], round: u32) -> Result<(), EngineError> {
        let Some(address) = &self.settings.rogue_address else {
            return Ok(());
        };
        let Some(first) = payments.first_mut() else {
            return Ok(());
        };
        let rogue = ZcashAddress::try_from_encoded(address)
            .map_err(|e| EngineError::Config(format!("{ROGUE_VAR}: {e}")))?;
        let amount = first.amount();
        let memo = first.memo().cloned();
        *first = Payment::new(rogue, amount, memo, None, None, vec![])
            .map_err(|e| EngineError::Config(format!("rogue payment: {e}")))?;
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
    ) -> Result<(Pczt, Vec<u16>), EngineError> {
        let needed = usize::from(self.settings.threshold);
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
                                approvals.insert(keyholder.identifier, c);
                            }
                            Ok(c) => details.push(format!(
                                "keyholder {}: {} commitments for {} spends",
                                keyholder.identifier,
                                c.len(),
                                pending.len()
                            )),
                            Err(e) => details.push(format!(
                                "keyholder {}: bad commitments: {e}",
                                keyholder.identifier
                            )),
                        }
                    }
                    Err(e) => details.push(format!("keyholder {}: {e}", keyholder.identifier)),
                },
                Err(e) => {
                    tracing::warn!(keyholder = keyholder.identifier, error = %e, "keyholder did not approve");
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

/// The earliest settled round that is owed and has no payout record.
pub fn next_payable_round(db: &Db) -> Result<Option<u32>, EngineError> {
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
        return Ok(Some(round));
    }
    Ok(None)
}
