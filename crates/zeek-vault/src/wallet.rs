//! View-only wallet backed by `zcash_client_sqlite`, synced through
//! `zcash_client_backend::sync::run`.
//!
//! The vault's unified full viewing key is imported as a view-only account:
//! the wallet can find and track notes but holds no spending key. Spending
//! happens separately via FROST (see [`crate::send`]).
//!
//! Since NU6.3 (Ironwood) every payment to the vault's Orchard receiver lands
//! in the **Ironwood** pool: the backend trial-decrypts Ironwood outputs with
//! the same Orchard incoming viewing key but under the Ironwood note-encryption
//! domain (ZIP 2005, note plaintext lead byte `0x03`) and tracks them as a
//! separate pool with its own commitment tree. Balances and notes are therefore
//! reported per pool; the legacy Orchard pool only matters for notes received
//! before NU6.3.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use orchard::ValuePool;
use rand_core::OsRng;
use zcash_client_backend::data_api::chain::{error, BlockCache, BlockSource};
use zcash_client_backend::data_api::locking::LockFilter;
use zcash_client_backend::data_api::scanning::ScanRange;
use zcash_client_backend::data_api::wallet::ConfirmationsPolicy;
use zcash_client_backend::data_api::wallet::TargetHeight;
use zcash_client_backend::data_api::{
    Account, AccountBirthday, AccountPurpose, Balance, InputSource, MaxSpendMode, TargetValue,
    WalletRead, WalletWrite,
};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::wallet::ReceivedNote;
use zcash_client_sqlite::util::SystemClock;
use zcash_client_sqlite::wallet::init::init_wallet_db;
use zcash_client_sqlite::{AccountUuid, ReceivedNoteId, WalletDb};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::consensus::{BlockHeight, TestNetwork};
use zcash_protocol::value::Zatoshis;
use zcash_protocol::ShieldedPool;

use crate::error::VaultError;
use crate::lightwalletd::{self, Client};
use crate::NETWORK;

/// Concrete wallet database type.
pub type Db = WalletDb<rusqlite::Connection, TestNetwork, SystemClock, OsRng>;

/// File name of the wallet database inside the data directory.
pub const WALLET_DB_FILE: &str = "zeek-vault-testnet.sqlite";
/// Human-readable account name used on import.
pub const ACCOUNT_NAME: &str = "zeek-vault";
/// Blocks per download/scan batch.
pub const SYNC_BATCH_SIZE: u32 = 1_000;

/// Opens (or creates) the wallet database and applies migrations.
pub fn open(data_dir: &Path) -> Result<Db, VaultError> {
    fs::create_dir_all(data_dir).map_err(|e| VaultError::io(data_dir, e))?;
    let path = data_dir.join(WALLET_DB_FILE);
    let mut db = WalletDb::for_path(&path, NETWORK, SystemClock, OsRng)
        .map_err(|e| VaultError::Wallet(format!("open {}: {e}", path.display())))?;
    init_wallet_db(&mut db, None)
        .map_err(|e| VaultError::Wallet(format!("migrate {}: {e}", path.display())))?;
    Ok(db)
}

/// Returns the account that tracks `ufvk`, if it has been imported.
pub fn find_account(
    db: &Db,
    ufvk: &UnifiedFullViewingKey,
) -> Result<Option<AccountUuid>, VaultError> {
    let account = db.get_account_for_ufvk(ufvk).map_err(wallet_err)?;
    Ok(account.map(|a| a.id()))
}

/// Imports `ufvk` as a view-only account if it is not present yet.
///
/// The birthday defaults to the current chain tip so a fresh vault does not
/// scan history it cannot own. `birthday` overrides that for vaults whose
/// address was funded before the wallet database was created.
pub async fn ensure_account(
    db: &mut Db,
    client: &mut Client,
    ufvk: &UnifiedFullViewingKey,
    birthday: Option<BlockHeight>,
) -> Result<(AccountUuid, BlockHeight), VaultError> {
    if let Some(id) = find_account(db, ufvk)? {
        let account = db
            .get_account(id)
            .map_err(wallet_err)?
            .ok_or(VaultError::AccountMissing)?;
        return Ok((id, account.birthday_height()));
    }

    let tip = lightwalletd::latest_height(client).await?;
    let birthday_height = birthday.unwrap_or(tip);
    if birthday_height > tip {
        return Err(VaultError::BadResponse(format!(
            "birthday {birthday_height} is above the chain tip {tip}"
        )));
    }
    // The birthday tree state is the state *before* the birthday block.
    let prior = birthday_height.saturating_sub(1);
    let tree_state = lightwalletd::tree_state(client, prior).await?;
    let account_birthday = AccountBirthday::from_treestate(tree_state, None)
        .map_err(|e| VaultError::Wallet(format!("birthday tree state: {e:?}")))?;

    let account = db
        .import_account_ufvk(
            ACCOUNT_NAME,
            ufvk,
            &account_birthday,
            AccountPurpose::ViewOnly,
            Some("FROST 2-of-3 vault (view-only import)"),
        )
        .map_err(wallet_err)?;
    Ok((account.id(), birthday_height))
}

/// Downloads and scans blocks until the wallet is at the chain tip.
pub async fn sync(db: &mut Db, client: &mut Client) -> Result<(), VaultError> {
    let cache = MemoryBlockCache::default();
    zcash_client_backend::sync::run(client, &NETWORK, &cache, db, SYNC_BATCH_SIZE)
        .await
        .map_err(|e| VaultError::Sync(e.to_string()))
}

/// One shielded note known to the wallet.
#[derive(Clone, Debug)]
pub struct NoteReport {
    /// Pool the note lives in: Ironwood (active) or legacy Orchard (closed to
    /// deposits since NU6.3).
    pub pool: ValuePool,
    /// Transaction that created the note.
    pub txid: String,
    /// Action index within that transaction.
    pub output_index: u16,
    /// Note value.
    pub value: Zatoshis,
    /// Height the note was mined at, if mined.
    pub mined_height: Option<u32>,
    /// Confirmations relative to the wallet's chain tip, if mined.
    pub confirmations: Option<u32>,
    /// Whether the default confirmations policy considers it spendable now.
    pub spendable: bool,
}

/// Value held in one pool, split by spendability.
#[derive(Clone, Copy, Debug)]
pub struct PoolBalance {
    /// Value spendable under the default confirmations policy.
    pub spendable: Zatoshis,
    /// Value waiting for more confirmations (received notes and change).
    pub pending: Zatoshis,
    /// Value in notes worth no more than the ZIP 317 marginal fee, which the
    /// wallet never selects because spending one costs at least what it
    /// holds. `zcash_client_backend`'s `Balance::total()` leaves this out;
    /// it is still the vault's money and counts in [`PoolBalance::total`].
    pub uneconomic: Zatoshis,
    /// Total value including pending and uneconomic notes.
    pub total: Zatoshis,
}

impl PoolBalance {
    /// An empty pool.
    pub const ZERO: Self = PoolBalance {
        spendable: Zatoshis::ZERO,
        pending: Zatoshis::ZERO,
        uneconomic: Zatoshis::ZERO,
        total: Zatoshis::ZERO,
    };

    fn from_balance(balance: &Balance) -> Result<Self, VaultError> {
        let pending = (balance.change_pending_confirmation()
            + balance.value_pending_spendability())
        .ok_or_else(|| VaultError::Wallet("pending balance overflow".into()))?;
        let total = (balance.total() + balance.uneconomic_value())
            .ok_or_else(|| VaultError::Wallet("total balance overflow".into()))?;
        Ok(PoolBalance {
            spendable: balance.spendable_value(),
            pending,
            uneconomic: balance.uneconomic_value(),
            total,
        })
    }
}

/// Balance summary for the vault account.
#[derive(Clone, Debug)]
pub struct BalanceReport {
    /// Chain tip as seen by the wallet.
    pub chain_tip: u32,
    /// Highest fully scanned height.
    pub fully_scanned: u32,
    /// Ironwood pool balance: the pool every payment to the vault lands in
    /// after NU6.3 and the only pool `send` spends from.
    pub ironwood: PoolBalance,
    /// Legacy Orchard pool balance. Non-zero only for notes received before
    /// NU6.3; such funds cannot be spent by this vault (see
    /// [`VaultError::FundsInClosedOrchardPool`]).
    pub orchard: PoolBalance,
    /// Mined notes in either pool with their confirmation depth.
    pub notes: Vec<NoteReport>,
    /// Confirmations the default policy requires before a received (external)
    /// note is spendable.
    pub min_confirmations: u32,
    /// Confirmations the default policy requires before change is spendable.
    pub change_confirmations: u32,
}

/// Reads the account's per-pool balance and note list from the wallet.
pub fn balance(db: &Db, account: AccountUuid) -> Result<BalanceReport, VaultError> {
    let policy = ConfirmationsPolicy::default();
    let summary = db
        .get_wallet_summary(policy)
        .map_err(wallet_err)?
        .ok_or_else(|| {
            VaultError::Wallet("wallet has not been synced yet; run `zeek-vault sync`".into())
        })?;
    let chain_tip = summary.chain_tip_height();
    let (ironwood, orchard) = match summary.account_balances().get(&account) {
        Some(balance) => (
            PoolBalance::from_balance(balance.ironwood_balance())?,
            PoolBalance::from_balance(balance.orchard_balance())?,
        ),
        None => (PoolBalance::ZERO, PoolBalance::ZERO),
    };

    let target: TargetHeight = (chain_tip + 1).into();
    let spendable_now = select_notes(db, account, target, policy)?;
    let all_mined = select_notes(db, account, target, ConfirmationsPolicy::MIN)?;

    let notes = all_mined
        .into_iter()
        .map(|(key, mut note)| {
            note.spendable = spendable_now.contains_key(&key);
            note
        })
        .collect();

    Ok(BalanceReport {
        chain_tip: chain_tip.into(),
        fully_scanned: summary.fully_scanned_height().into(),
        ironwood,
        orchard,
        notes,
        min_confirmations: policy.untrusted().get(),
        change_confirmations: policy.trusted().get(),
    })
}

/// Key identifying a note across pools: `(pool, txid, action index)`.
type NoteKey = (ValuePool, String, u16);

fn select_notes(
    db: &Db,
    account: AccountUuid,
    target: TargetHeight,
    policy: ConfirmationsPolicy,
) -> Result<BTreeMap<NoteKey, NoteReport>, VaultError> {
    let tip = target.saturating_sub(1);
    let notes = db
        .select_spendable_notes(
            account,
            TargetValue::AllFunds(MaxSpendMode::MaxSpendable),
            &[ShieldedPool::Ironwood, ShieldedPool::Orchard],
            target,
            policy,
            &[],
            LockFilter::Unfiltered,
        )
        .map_err(wallet_err)?;
    let orchard = notes.orchard().to_vec();
    let ironwood = notes.take_ironwood();
    let report = |pool: ValuePool, n: ReceivedNote<ReceivedNoteId, orchard::Note>| {
        let txid = n.txid().to_string();
        let mined_height = n.mined_height().map(u32::from);
        let confirmations = mined_height.map(|h| u32::from(tip).saturating_sub(h) + 1);
        (
            (pool, txid.clone(), n.output_index()),
            NoteReport {
                pool,
                txid,
                output_index: n.output_index(),
                value: Zatoshis::from_u64(n.note().value().inner()).unwrap_or(Zatoshis::ZERO),
                mined_height,
                confirmations,
                spendable: false,
            },
        )
    };
    Ok(ironwood
        .into_iter()
        .map(|n| report(ValuePool::Ironwood, n))
        .chain(orchard.into_iter().map(|n| report(ValuePool::Orchard, n)))
        .collect())
}

/// Human-readable pool name for CLI output.
pub fn pool_name(pool: ValuePool) -> &'static str {
    match pool {
        ValuePool::Ironwood => "ironwood",
        ValuePool::Orchard => "orchard",
    }
}

/// Converts any wallet error into the crate's error type.
pub fn wallet_err<E: std::fmt::Display>(e: E) -> VaultError {
    VaultError::Wallet(e.to_string())
}

/// In-memory compact block cache for `sync::run`.
///
/// `sync::run` deletes each range right after scanning it, so nothing is
/// gained by persisting blocks; keeping them in memory avoids a second
/// database and still bounds memory by the batch size.
#[derive(Default)]
pub struct MemoryBlockCache {
    blocks: Mutex<BTreeMap<u32, CompactBlock>>,
}

/// Error type of [`MemoryBlockCache`]; the cache itself cannot fail, but the
/// scanning callbacks it drives can.
#[derive(Debug, thiserror::Error)]
#[error("block cache lock poisoned")]
pub struct CacheError;

impl MemoryBlockCache {
    fn with_blocks<T>(
        &self,
        f: impl FnOnce(&mut BTreeMap<u32, CompactBlock>) -> T,
    ) -> Result<T, CacheError> {
        let mut guard = self.blocks.lock().map_err(|_| CacheError)?;
        Ok(f(&mut guard))
    }

    fn height_of(block: &CompactBlock) -> u32 {
        u32::try_from(block.height).unwrap_or(u32::MAX)
    }
}

impl BlockSource for MemoryBlockCache {
    type Error = CacheError;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), error::Error<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), error::Error<WalletErrT, Self::Error>>,
    {
        let start = from_height.map(u32::from).unwrap_or(0);
        let selected: Vec<CompactBlock> = self
            .with_blocks(|blocks| {
                blocks
                    .range(start..)
                    .take(limit.unwrap_or(usize::MAX))
                    .map(|(_, b)| b.clone())
                    .collect()
            })
            .map_err(error::Error::BlockSource)?;
        for block in selected {
            with_block(block)?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockCache for MemoryBlockCache {
    fn get_tip_height(
        &self,
        range: Option<&ScanRange>,
    ) -> Result<Option<BlockHeight>, Self::Error> {
        self.with_blocks(|blocks| {
            blocks
                .keys()
                .filter(|h| {
                    range.is_none_or(|r| r.block_range().contains(&BlockHeight::from_u32(**h)))
                })
                .max()
                .map(|h| BlockHeight::from_u32(*h))
        })
    }

    async fn read(&self, range: &ScanRange) -> Result<Vec<CompactBlock>, Self::Error> {
        self.with_blocks(|blocks| {
            blocks
                .range(u32::from(range.block_range().start)..u32::from(range.block_range().end))
                .map(|(_, b)| b.clone())
                .collect()
        })
    }

    async fn insert(&self, compact_blocks: Vec<CompactBlock>) -> Result<(), Self::Error> {
        self.with_blocks(|blocks| {
            for block in compact_blocks {
                blocks.insert(Self::height_of(&block), block);
            }
        })
    }

    async fn delete(&self, range: ScanRange) -> Result<(), Self::Error> {
        self.with_blocks(|blocks| {
            blocks.retain(|h, _| !range.block_range().contains(&BlockHeight::from_u32(*h)));
        })
    }
}
