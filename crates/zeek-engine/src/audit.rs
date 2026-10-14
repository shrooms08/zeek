//! `zeek-engine audit`: rebuilds a vault's complete history from chain data
//! and reconciles it, line by line, against the engine's settlements, payout
//! records and ledger.
//!
//! The only inputs are compact blocks and full transactions from a
//! lightwalletd plus each vault's **public** viewing material (`vault.json`):
//! every output is trial-decrypted with the external and internal incoming
//! viewing keys (received notes, with memos), every action's nullifier is
//! matched against the nullifiers of the notes found so far (spends), every
//! transaction that spends a vault note has its fee read from the bundle
//! value balances and its non-vault outputs recovered with the outgoing
//! viewing keys (payouts). Nothing here needs a key share or a sealing key,
//! and no payout receiver is printed: recovered outputs are reported by
//! amount and memo only.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use orchard::keys::{FullViewingKey, PreparedIncomingViewingKey, Scope};
use orchard::note_encryption::{CompactAction, IronwoodDomain, OrchardDomain};
use zcash_client_backend::proto::compact_formats::{CompactOrchardAction, CompactTx};
use zcash_note_encryption::{
    try_compact_note_decryption, try_note_decryption, try_output_recovery_with_ovk,
};
use zcash_primitives::transaction::Transaction;
use zeek_core::memo::DigMemo;
use zeek_core::params::Network;
use zeek_vault::store::{SecretsDir, VaultFile};
use zeek_vault::wallet;

use crate::chain::{branch_for, ChainSource, BATCH_SIZE};
use crate::config::GameConfig;
use crate::db::{Db, PayoutState, Pool};
use crate::error::EngineError;
use crate::ledger;
use crate::processor::Processor;

/// One vault to audit and what to reconcile it against.
#[derive(Clone, Debug)]
pub struct VaultSpec {
    /// Label in the report.
    pub name: String,
    /// Directory holding the vault's public `vault.json`.
    pub secrets_dir: PathBuf,
    /// The engine database that settled this vault's rounds, if any.
    pub engine_db: Option<PathBuf>,
    /// The game config those rounds were played under.
    pub game_config: Option<PathBuf>,
    /// The vault wallet's data directory, to compare its balance.
    pub wallet_data_dir: Option<PathBuf>,
}

impl VaultSpec {
    /// Parses `name=<secrets dir>[,engine=<db>][,game=<config>][,wallet=<dir>]`.
    pub fn parse(text: &str) -> Result<Self, EngineError> {
        let bad = |m: String| EngineError::Config(format!("--vault `{text}`: {m}"));
        let mut parts = text.split(',');
        let first = parts.next().ok_or_else(|| bad("empty".into()))?;
        let (name, dir) = first
            .split_once('=')
            .ok_or_else(|| bad("expected name=<secrets dir>".into()))?;
        let mut spec = VaultSpec {
            name: name.to_owned(),
            secrets_dir: PathBuf::from(dir),
            engine_db: None,
            game_config: None,
            wallet_data_dir: None,
        };
        for part in parts {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| bad(format!("expected key=value, got `{part}`")))?;
            match key {
                "engine" => spec.engine_db = Some(PathBuf::from(value)),
                "game" => spec.game_config = Some(PathBuf::from(value)),
                "wallet" => spec.wallet_data_dir = Some(PathBuf::from(value)),
                other => return Err(bad(format!("unknown key `{other}`"))),
            }
        }
        Ok(spec)
    }
}

/// `(pool, display txid, action index)`.
type NoteKey = (Pool, String, u16);

/// A vault's view of one compact transaction: the vault's index, the outputs
/// that decrypt to it (pool, action index, scope) and the notes it spends.
type Touched = (usize, Vec<(Pool, u16, Scope)>, Vec<NoteKey>);

/// A note the vault received, as rebuilt from the chain.
#[derive(Clone, Debug)]
struct ChainNote {
    height: u32,
    value: u64,
    scope: Scope,
    /// What the memo says: a dig (round, tile), text, or nothing.
    kind: String,
    cmx: [u8; 32],
    spent_in: Option<(u32, String)>,
}

/// A transaction that touched the vault.
#[derive(Clone, Debug)]
struct ChainTx {
    height: u32,
    txid: String,
    received: Vec<NoteKey>,
    spent: Vec<NoteKey>,
    /// Outputs to other recipients, recovered with the vault's outgoing
    /// viewing key: value and memo description.
    paid: Vec<(u64, String)>,
    /// Shielded value balance of the transaction: the fee for a fully
    /// shielded transaction.
    fee: i64,
}

/// Everything rebuilt for one vault.
struct VaultHistory {
    fvk: FullViewingKey,
    address: String,
    notes: BTreeMap<NoteKey, ChainNote>,
    nullifiers: HashMap<[u8; 32], NoteKey>,
    txs: Vec<ChainTx>,
}

impl VaultHistory {
    fn new(vault: &VaultFile) -> Result<Self, EngineError> {
        Ok(VaultHistory {
            fvk: vault.orchard_fvk()?,
            address: vault.address.clone(),
            notes: BTreeMap::new(),
            nullifiers: HashMap::new(),
            txs: Vec::new(),
        })
    }

    fn balance(&self) -> Result<u64, EngineError> {
        self.notes
            .values()
            .filter(|n| n.spent_in.is_none())
            .try_fold(0u64, |acc, n| acc.checked_add(n.value))
            .ok_or(EngineError::Overflow("audit balance"))
    }
}

/// Outcome of the audit: the report text and how many lines disagreed.
pub struct AuditOutcome {
    pub report: String,
    pub mismatches: usize,
}

fn describe_memo(memo: &[u8; 512]) -> String {
    if let Ok(dig) = DigMemo::decode(memo) {
        return format!("dig round {} tile {}", dig.round, dig.tile);
    }
    let end = memo
        .iter()
        .rposition(|b| *b != 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    let trimmed = &memo[..end];
    match trimmed.first() {
        None => "empty memo".to_owned(),
        Some(0xF6) if trimmed.len() == 1 => "empty memo".to_owned(),
        Some(b) if *b <= 0xF4 => match std::str::from_utf8(trimmed) {
            Ok(text) => format!("memo \"{text}\""),
            Err(_) => format!("non-UTF-8 text memo ({} bytes)", trimmed.len()),
        },
        Some(_) => format!("non-text memo ({} bytes)", trimmed.len()),
    }
}

fn txid_hex(raw: &[u8]) -> String {
    let mut bytes = raw.to_vec();
    bytes.reverse();
    hex::encode(bytes)
}

/// Scans `from..=tip` once for every vault.
async fn rebuild(
    source: &mut dyn ChainSource,
    from: u32,
    network: Network,
    vaults: &mut [VaultHistory],
) -> Result<u32, EngineError> {
    let tip = source.latest_height().await?;
    let keys: Vec<(PreparedIncomingViewingKey, PreparedIncomingViewingKey)> = vaults
        .iter()
        .map(|v| {
            (
                PreparedIncomingViewingKey::new(&v.fvk.to_ivk(Scope::External)),
                PreparedIncomingViewingKey::new(&v.fvk.to_ivk(Scope::Internal)),
            )
        })
        .collect();
    let mut next = from;
    while next <= tip {
        let end = next.saturating_add(BATCH_SIZE - 1).min(tip);
        let blocks = source.block_range(next, end).await?;
        if blocks.is_empty() {
            break;
        }
        for block in &blocks {
            let height = u32::try_from(block.height).map_err(|_| EngineError::UnexpectedBlock {
                expected: next,
                got: block.height,
            })?;
            for tx in &block.vtx {
                audit_tx(source, network, height, tx, vaults, &keys).await?;
            }
            next = height + 1;
        }
    }
    Ok(tip)
}

/// Compact pass over one transaction: hits and spends per vault, then the
/// full transaction for every vault it touches.
async fn audit_tx(
    source: &mut dyn ChainSource,
    network: Network,
    height: u32,
    tx: &CompactTx,
    vaults: &mut [VaultHistory],
    keys: &[(PreparedIncomingViewingKey, PreparedIncomingViewingKey)],
) -> Result<(), EngineError> {
    let actions: Vec<(Pool, usize, &CompactOrchardAction)> = tx
        .ironwood_actions
        .iter()
        .enumerate()
        .map(|(i, a)| (Pool::Ironwood, i, a))
        .chain(
            tx.actions
                .iter()
                .enumerate()
                .map(|(i, a)| (Pool::Orchard, i, a)),
        )
        .collect();
    if actions.is_empty() {
        return Ok(());
    }
    let txid = txid_hex(&tx.txid);
    let malformed = |reason: String| EngineError::MalformedBlock { height, reason };

    // Which vaults does this transaction touch, and how?
    let mut touched: Vec<Touched> = Vec::new();
    for (vi, vault) in vaults.iter().enumerate() {
        let mut hits = Vec::new();
        let mut spends = Vec::new();
        for (pool, index, action) in &actions {
            let compact = CompactAction::try_from(*action)
                .map_err(|e| malformed(format!("compact action {index}: {e:?}")))?;
            let index16 =
                u16::try_from(*index).map_err(|_| malformed("too many actions".into()))?;
            let nullifier = <[u8; 32]>::try_from(action.nullifier.as_slice())
                .map_err(|_| malformed("nullifier is not 32 bytes".into()))?;
            if let Some(key) = vault.nullifiers.get(&nullifier) {
                spends.push(key.clone());
            }
            for (scope, ivk) in [
                (Scope::External, &keys[vi].0),
                (Scope::Internal, &keys[vi].1),
            ] {
                let decrypted = match pool {
                    Pool::Ironwood => try_compact_note_decryption(
                        &IronwoodDomain::for_compact_action(&compact),
                        ivk,
                        &compact,
                    ),
                    Pool::Orchard => try_compact_note_decryption(
                        &OrchardDomain::for_compact_action(&compact),
                        ivk,
                        &compact,
                    ),
                };
                if decrypted.is_some() {
                    hits.push((*pool, index16, scope));
                    break;
                }
            }
        }
        if !hits.is_empty() || !spends.is_empty() {
            touched.push((vi, hits, spends));
        }
    }
    if touched.is_empty() {
        return Ok(());
    }

    let raw = source
        .transaction(
            &<[u8; 32]>::try_from(tx.txid.as_slice())
                .map_err(|_| malformed("txid is not 32 bytes".into()))?,
        )
        .await?;
    let parsed = Transaction::read(&raw[..], branch_for(network, height)).map_err(|e| {
        EngineError::MalformedTransaction {
            txid: txid.clone(),
            reason: e.to_string(),
        }
    })?;
    let fee = parsed
        .orchard_bundle()
        .map_or(0i64, |b| i64::from(*b.value_balance()))
        .checked_add(
            parsed
                .ironwood_bundle()
                .map_or(0i64, |b| i64::from(*b.value_balance())),
        )
        .ok_or(EngineError::Overflow("value balance"))?;

    for (vi, hits, spends) in touched {
        let vault = &mut vaults[vi];
        let mut record = ChainTx {
            height,
            txid: txid.clone(),
            received: Vec::new(),
            spent: spends.clone(),
            paid: Vec::new(),
            fee,
        };
        for key in &spends {
            if let Some(note) = vault.notes.get_mut(key) {
                note.spent_in = Some((height, txid.clone()));
            }
        }
        let ovks = [
            vault.fvk.to_ovk(Scope::External),
            vault.fvk.to_ovk(Scope::Internal),
        ];
        for (pool, bundle) in [
            (Pool::Ironwood, parsed.ironwood_bundle()),
            (Pool::Orchard, parsed.orchard_bundle()),
        ] {
            let Some(bundle) = bundle else { continue };
            for (index, action) in bundle.actions().iter().enumerate() {
                let index16 =
                    u16::try_from(index).map_err(|_| malformed("too many actions".into()))?;
                let hit = hits.iter().find(|(p, i, _)| *p == pool && *i == index16);
                if let Some((_, _, scope)) = hit {
                    let ivk = match scope {
                        Scope::External => &keys[vi].0,
                        Scope::Internal => &keys[vi].1,
                    };
                    let decrypted = match pool {
                        Pool::Ironwood => {
                            try_note_decryption(&IronwoodDomain::for_action(action), ivk, action)
                        }
                        Pool::Orchard => {
                            try_note_decryption(&OrchardDomain::for_action(action), ivk, action)
                        }
                    };
                    let (note, _, memo) =
                        decrypted.ok_or_else(|| EngineError::MalformedTransaction {
                            txid: txid.clone(),
                            reason: format!("action {index} decrypts compactly but not in full"),
                        })?;
                    let key: NoteKey = (pool, txid.clone(), index16);
                    vault
                        .nullifiers
                        .insert(note.nullifier(&vault.fvk).to_bytes(), key.clone());
                    vault.notes.insert(
                        key.clone(),
                        ChainNote {
                            height,
                            value: note.value().inner(),
                            scope: *scope,
                            kind: describe_memo(&memo),
                            cmx: action.cmx().to_bytes(),
                            spent_in: None,
                        },
                    );
                    record.received.push(key);
                    continue;
                }
                if spends.is_empty() {
                    continue;
                }
                // An output of a transaction the vault funded: recover it.
                for ovk in &ovks {
                    let recovered = match pool {
                        Pool::Ironwood => try_output_recovery_with_ovk(
                            &IronwoodDomain::for_action(action),
                            ovk,
                            action,
                            action.cv_net(),
                            &action.encrypted_note().out_ciphertext,
                        ),
                        Pool::Orchard => try_output_recovery_with_ovk(
                            &OrchardDomain::for_action(action),
                            ovk,
                            action,
                            action.cv_net(),
                            &action.encrypted_note().out_ciphertext,
                        ),
                    };
                    if let Some((note, _, memo)) = recovered {
                        if note.value().inner() > 0 {
                            record
                                .paid
                                .push((note.value().inner(), describe_memo(&memo)));
                        }
                        break;
                    }
                }
            }
        }
        vault.txs.push(record);
    }
    Ok(())
}

/// Runs the audit and renders the report.
pub async fn run(
    source: &mut dyn ChainSource,
    from: Option<u32>,
    specs: &[VaultSpec],
) -> Result<AuditOutcome, EngineError> {
    let mut report = String::new();
    let mut mismatches = 0usize;
    let mut vault_files = Vec::with_capacity(specs.len());
    let mut histories = Vec::with_capacity(specs.len());
    let mut ledgers: Vec<Option<ledger::Ledger>> = Vec::with_capacity(specs.len());
    for spec in specs {
        let vault = SecretsDir::new(&spec.secrets_dir).load_vault()?;
        histories.push(VaultHistory::new(&vault)?);
        vault_files.push(vault);
    }
    let from = match from {
        Some(h) => h,
        None => vault_files
            .iter()
            .filter_map(|v| v.birthday_height)
            .min()
            .ok_or_else(|| {
                EngineError::Config(
                    "no --from height and no vault file records a birthday height".into(),
                )
            })?,
    };
    let _ = writeln!(report, "zeek-engine audit: chain data from height {from}");
    let tip = rebuild(source, from, Network::Test, &mut histories).await?;
    let _ = writeln!(report, "chain tip {tip}");

    for (spec, history) in specs.iter().zip(&histories) {
        let _ = writeln!(report);
        let _ = writeln!(report, "== vault {} ({})", spec.name, history.address);
        let mut balance = 0i128;
        for tx in &history.txs {
            let _ = writeln!(report, "h={} tx {} fee={}", tx.height, tx.txid, tx.fee);
            for key in &tx.received {
                let note = &history.notes[key];
                let _ = writeln!(
                    report,
                    "  + {:>12}  {}:{} {:?} {} ({})",
                    note.value,
                    pool_name(key.0),
                    key.2,
                    note.scope,
                    note.kind,
                    hex::encode(note.cmx)
                );
                balance += i128::from(note.value);
            }
            for key in &tx.spent {
                let note = &history.notes[key];
                let _ = writeln!(
                    report,
                    "  - {:>12}  spent {}:{} (received at h={}, {})",
                    note.value,
                    &key.1[..16],
                    key.2,
                    note.height,
                    note.kind
                );
                balance -= i128::from(note.value);
            }
            for (value, memo) in &tx.paid {
                let _ = writeln!(report, "  > {value:>12}  paid out, {memo}");
            }
            let _ = writeln!(report, "  = {balance:>12}  balance after this transaction");
        }
        let unspent: Vec<(&NoteKey, &ChainNote)> = history
            .notes
            .iter()
            .filter(|(_, n)| n.spent_in.is_none())
            .collect();
        let chain_balance = history.balance()?;
        let _ = writeln!(
            report,
            "unspent notes: {} totalling {}",
            unspent.len(),
            chain_balance
        );
        for (key, note) in &unspent {
            let _ = writeln!(
                report,
                "  {:>12}  {}:{} h={} {:?} {}",
                note.value,
                &key.1[..16],
                key.2,
                note.height,
                note.scope,
                note.kind
            );
        }
        if i128::from(chain_balance) != balance {
            mismatches += 1;
            let _ = writeln!(
                report,
                "MISMATCH running balance {balance} != unspent total {chain_balance}"
            );
        }

        // Reconciliation against the engine.
        if let (Some(db_path), Some(game_path)) = (&spec.engine_db, &spec.game_config) {
            let (found, ledger) =
                reconcile(&mut report, history, chain_balance, tip, db_path, game_path)?;
            mismatches += found;
            ledgers.push(Some(ledger));
        } else {
            ledgers.push(None);
        }
        if let Some(dir) = &spec.wallet_data_dir {
            mismatches += reconcile_wallet(&mut report, history, chain_balance, dir)?;
        }
    }

    // A vault that was emptied into another audited vault has a shortfall of
    // exactly the Cache it carried over: the receiving game's
    // `initial_cache_zat` must equal it, and the migration transaction must
    // appear as a payment in one history and a received note in the other.
    for (i, (spec, history)) in specs.iter().zip(&histories).enumerate() {
        let Some(Some(ledger)) = ledgers.get(i) else {
            continue;
        };
        if ledger.shortfall == 0 || ledger.vault_balance != 0 || ledger.shortfall != ledger.cache {
            continue;
        }
        for (j, (other_spec, other)) in specs.iter().zip(&histories).enumerate() {
            if i == j {
                continue;
            }
            let Some(game_path) = &other_spec.game_config else {
                continue;
            };
            let other_config = GameConfig::load(game_path)?;
            if other_config.initial_cache_zat != ledger.cache {
                continue;
            }
            let migration = history.txs.iter().find(|tx| {
                !tx.paid.is_empty() && other.notes.keys().any(|(_, txid, _)| *txid == tx.txid)
            });
            if let Some(tx) = migration {
                let _ = writeln!(
                    report,
                    "-- vault {}: its Cache of {} was carried over to vault {} (initial_cache_zat {}) by tx {} at h={}; the shortfall above is that carry-over, not a loss",
                    spec.name,
                    ledger.cache,
                    other_spec.name,
                    other_config.initial_cache_zat,
                    tx.txid,
                    tx.height
                );
                mismatches = mismatches.saturating_sub(1);
                break;
            }
        }
    }
    let _ = writeln!(report);
    let _ = writeln!(report, "mismatches: {mismatches}");
    Ok(AuditOutcome { report, mismatches })
}

/// What the chain says about the transaction that spent a set of notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpendingTx {
    pub txid: String,
    pub height: u32,
    pub fee: i64,
}

/// Finds the transaction that spent **all** of `notes` (display txid, action
/// index) of the vault in `secrets_dir`, scanning chain data from `from`.
/// Returns `None` if none of them is spent yet; errors if they were spent by
/// different transactions.
pub async fn find_spending_tx(
    source: &mut dyn ChainSource,
    from: u32,
    secrets_dir: &Path,
    notes: &[(String, u16)],
) -> Result<Option<SpendingTx>, EngineError> {
    let vault = SecretsDir::new(secrets_dir).load_vault()?;
    let mut histories = vec![VaultHistory::new(&vault)?];
    rebuild(source, from, Network::Test, &mut histories).await?;
    let history = &histories[0];
    let mut found: Option<SpendingTx> = None;
    for (txid, index) in notes {
        let key: NoteKey = (Pool::Ironwood, txid.clone(), *index);
        let note = history.notes.get(&key).ok_or_else(|| {
            EngineError::Corrupt(format!("note {txid}:{index} is not a note of this vault"))
        })?;
        let Some((height, spender)) = &note.spent_in else {
            return Ok(None);
        };
        let tx = history
            .txs
            .iter()
            .find(|t| &t.txid == spender)
            .ok_or_else(|| EngineError::Corrupt(format!("spending tx {spender} vanished")))?;
        let this = SpendingTx {
            txid: spender.clone(),
            height: *height,
            fee: tx.fee,
        };
        match &found {
            None => found = Some(this),
            Some(f) if *f == this => {}
            Some(f) => {
                return Err(EngineError::Corrupt(format!(
                    "notes were spent by different transactions: {} and {}",
                    f.txid, this.txid
                )))
            }
        }
    }
    Ok(found)
}

fn pool_name(pool: Pool) -> &'static str {
    match pool {
        Pool::Ironwood => "ironwood",
        Pool::Orchard => "orchard",
    }
}

/// Compares the chain history with the engine database: notes per settled
/// round, payout transactions (inputs, outputs, fee), and the ledger.
/// Returns the mismatches found and the ledger computed from the chain balance.
fn reconcile(
    report: &mut String,
    history: &VaultHistory,
    chain_balance: u64,
    tip: u32,
    db_path: &Path,
    game_path: &Path,
) -> Result<(usize, ledger::Ledger), EngineError> {
    let mut mismatches = 0usize;
    let config = GameConfig::load(game_path)?;
    let processor = Processor::new(config.clone());
    let db = Db::open(db_path)?;
    let _ = writeln!(report, "-- reconciliation against {}", db_path.display());
    let dig_notes: Vec<(&NoteKey, &ChainNote)> = history
        .notes
        .iter()
        .filter(|(k, n)| k.0 == Pool::Ironwood && n.scope == Scope::External)
        .collect();
    let mut rounds_checked = 0usize;
    for round in db.settled_rounds()? {
        let row = db
            .settlement(round)?
            .ok_or_else(|| EngineError::Corrupt(format!("settlement {round} vanished")))?;
        let (window, _) = processor.round_context(&db, round)?;
        let engine_notes = db.notes_in_range(window.start, window.end)?;
        let mut engine_set: Vec<(String, u16, u64)> = engine_notes
            .iter()
            .map(|n| (n.txid_hex(), n.action_index, n.value))
            .collect();
        engine_set.sort();
        let mut chain_set: Vec<(String, u16, u64)> = dig_notes
            .iter()
            .filter(|(_, n)| window.contains(n.height))
            .map(|(k, n)| (k.1.clone(), k.2, n.value))
            .collect();
        chain_set.sort();
        let notes_ok = engine_set == chain_set;
        if !notes_ok {
            mismatches += 1;
        }
        let _ = writeln!(
            report,
            "round {round}: {} notes in window [{}, {}) {} (rules v{}, pot {}, fees {}, paid {}, cache {} -> {})",
            chain_set.len(),
            window.start,
            window.end,
            if notes_ok { "match the engine" } else { "MISMATCH with the engine" },
            row.rules_version,
            row.settlement.pot,
            row.settlement.total_fees,
            row.settlement.total_paid,
            row.settlement.previous_cache,
            row.settlement.new_cache
        );
        rounds_checked += 1;

        let Some(payout) = db.payout(round)? else {
            continue;
        };
        if payout.state != PayoutState::Broadcast {
            let _ = writeln!(report, "  payout record in state {:?}", payout.state);
            continue;
        }
        let Some(txid) = &payout.txid else {
            mismatches += 1;
            let _ = writeln!(report, "  MISMATCH payout marked broadcast without a txid");
            continue;
        };
        let Some(tx) = history.txs.iter().find(|t| &t.txid == txid) else {
            mismatches += 1;
            let _ = writeln!(report, "  MISMATCH payout tx {txid} not found on chain");
            continue;
        };
        let fee_ok = i128::from(tx.fee) == i128::from(payout.fee);
        if !fee_ok {
            mismatches += 1;
        }
        let _ = writeln!(
            report,
            "  payout tx {} at h={}: chain fee {} vs recorded {} {}",
            txid,
            tx.height,
            tx.fee,
            payout.fee,
            if fee_ok { "ok" } else { "MISMATCH" }
        );
        let collected = row.settlement.total_fees;
        let deficit = u64::try_from(tx.fee).unwrap_or(0).saturating_sub(collected);
        let _ = writeln!(
            report,
            "  fee coverage: collected {collected}, fee {}, deficit {deficit}{}",
            tx.fee,
            if deficit > 0 {
                " (paid from the operator float)"
            } else {
                ""
            }
        );
        let mut paid: Vec<u64> = tx.paid.iter().map(|(v, _)| *v).collect();
        paid.sort_unstable();
        let mut expected: Vec<u64> = row.settlement.payouts.iter().map(|p| p.net).collect();
        expected.sort_unstable();
        let paid_ok = paid == expected;
        if !paid_ok {
            mismatches += 1;
        }
        let _ = writeln!(
            report,
            "  outputs recovered with the outgoing viewing key {paid:?} vs settlement nets {expected:?} {}",
            if paid_ok { "ok" } else { "MISMATCH" }
        );
        let spent_total: u64 = tx.spent.iter().map(|k| history.notes[k].value).sum();
        let change: Vec<u64> = tx.received.iter().map(|k| history.notes[k].value).collect();
        let _ = writeln!(
            report,
            "  inputs {} notes totalling {spent_total}; change {change:?}",
            tx.spent.len()
        );
        if row.rules_version >= 2 {
            let mut expected_inputs: Vec<(String, u16)> = processor
                .payout_inputs(&db, &row)?
                .into_iter()
                .map(|n| (n.txid, n.output_index))
                .collect();
            expected_inputs.sort();
            let mut actual: Vec<(String, u16)> =
                tx.spent.iter().map(|k| (k.1.clone(), k.2)).collect();
            actual.sort();
            let floats: Vec<(String, u16)> = actual
                .iter()
                .filter(|a| !expected_inputs.contains(a))
                .cloned()
                .collect();
            let all_digs_spent = expected_inputs.iter().all(|e| actual.contains(e));
            if !all_digs_spent {
                mismatches += 1;
            }
            let _ = writeln!(
                report,
                "  rules v2 inputs: {} charged dig notes {}; {} float notes",
                expected_inputs.len(),
                if all_digs_spent {
                    "all spent"
                } else {
                    "MISMATCH not all spent"
                },
                floats.len()
            );
        }
    }
    let _ = writeln!(report, "{rounds_checked} settled rounds checked");

    let ledger = ledger::compute(&db, &processor, chain_balance, tip)?;
    let _ = writeln!(
        report,
        "ledger from the chain balance: vault {} = cache {} + obligations {} + reserved fees {} + float {} (shortfall {})",
        ledger.vault_balance,
        ledger.cache,
        ledger.unpaid_obligations,
        ledger.reserved_fees,
        ledger.operator_float,
        ledger.shortfall
    );
    if ledger.shortfall > 0 {
        mismatches += 1;
        let _ = writeln!(report, "MISMATCH the vault does not cover its obligations");
    }
    Ok((mismatches, ledger))
}

/// Compares the chain balance with what the vault wallet reports.
fn reconcile_wallet(
    report: &mut String,
    history: &VaultHistory,
    chain_balance: u64,
    data_dir: &Path,
) -> Result<usize, EngineError> {
    let db = wallet::open(data_dir)?;
    let ufvk = zeek_vault::keys::unified_full_viewing_key(&history.fvk)?;
    let Some(account) = wallet::find_account(&db, &ufvk)? else {
        let _ = writeln!(
            report,
            "-- wallet {}: no account for this vault",
            data_dir.display()
        );
        return Ok(0);
    };
    let balance = wallet::balance(&db, account)?;
    let total = balance.ironwood.total.into_u64();
    let ok = total == chain_balance;
    let _ = writeln!(
        report,
        "-- wallet {} at tip {}: total {} (spendable {}, pending {}, uneconomic {}) vs chain {} {}",
        data_dir.display(),
        balance.chain_tip,
        total,
        balance.ironwood.spendable.into_u64(),
        balance.ironwood.pending.into_u64(),
        balance.ironwood.uneconomic.into_u64(),
        chain_balance,
        if ok { "ok" } else { "MISMATCH" }
    );
    Ok(usize::from(!ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_spec_parses() {
        let spec =
            VaultSpec::parse("dkg=.secrets,engine=data/e.sqlite,game=config/g.json,wallet=data")
                .expect("spec");
        assert_eq!(spec.name, "dkg");
        assert_eq!(spec.secrets_dir, PathBuf::from(".secrets"));
        assert_eq!(spec.engine_db, Some(PathBuf::from("data/e.sqlite")));
        assert_eq!(spec.game_config, Some(PathBuf::from("config/g.json")));
        assert_eq!(spec.wallet_data_dir, Some(PathBuf::from("data")));
        assert!(VaultSpec::parse("nodir").is_err());
        assert!(VaultSpec::parse("a=b,bogus=c").is_err());
    }

    #[test]
    fn memo_descriptions() {
        let mut text = [0u8; 512];
        text[..5].copy_from_slice(b"hello");
        assert_eq!(describe_memo(&text), "memo \"hello\"");
        assert_eq!(describe_memo(&[0u8; 512]), "empty memo");
        let mut empty = [0u8; 512];
        empty[0] = 0xF6;
        assert_eq!(describe_memo(&empty), "empty memo");
    }
}
