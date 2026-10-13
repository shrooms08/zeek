//! View-only wallet backed by `zcash_client_sqlite`, synced through
//! `zcash_client_backend::sync::run`.
//!
//! The vault's unified full viewing key is imported as a view-only account:
//! the wallet can find and track notes but holds no spending key. Spending
//! happens separately via FROST (see [`crate::send`]).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use rand_core::OsRng;
use zcash_client_backend::data_api::chain::{error, BlockCache, BlockSource};
use zcash_client_backend::data_api::locking::LockFilter;
use zcash_client_backend::data_api::scanning::ScanRange;
use zcash_client_backend::data_api::wallet::ConfirmationsPolicy;
use zcash_client_backend::data_api::wallet::TargetHeight;
use zcash_client_backend::data_api::{
    Account, AccountBirthday, AccountPurpose, InputSource, MaxSpendMode, TargetValue, WalletRead,
    WalletWrite,
};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_sqlite::util::SystemClock;
use zcash_client_sqlite::wallet::init::init_wallet_db;
use zcash_client_sqlite::{AccountUuid, WalletDb};
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

/// One Orchard note known to the wallet.
#[derive(Clone, Debug)]
pub struct NoteReport {
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

/// Balance summary for the vault account.
#[derive(Clone, Debug)]
pub struct BalanceReport {
    /// Chain tip as seen by the wallet.
    pub chain_tip: u32,
    /// Highest fully scanned height.
    pub fully_scanned: u32,
    /// Orchard value spendable under the default confirmations policy.
    pub spendable: Zatoshis,
    /// Orchard value waiting for more confirmations.
    pub pending: Zatoshis,
    /// Total Orchard value including pending.
    pub total: Zatoshis,
    /// Orchard notes with at least one confirmation.
    pub notes: Vec<NoteReport>,
}

/// Reads the account's Orchard balance and note list from the wallet.
pub fn balance(db: &Db, account: AccountUuid) -> Result<BalanceReport, VaultError> {
    let summary = db
        .get_wallet_summary(ConfirmationsPolicy::default())
        .map_err(wallet_err)?
        .ok_or_else(|| {
            VaultError::Wallet("wallet has not been synced yet; run `zeek-vault sync`".into())
        })?;
    let chain_tip = summary.chain_tip_height();
    let (spendable, pending, total) = match summary.account_balances().get(&account) {
        Some(balance) => {
            let orchard = balance.orchard_balance();
            let pending = (orchard.change_pending_confirmation()
                + orchard.value_pending_spendability())
            .unwrap_or(Zatoshis::ZERO);
            (orchard.spendable_value(), pending, orchard.total())
        }
        None => (Zatoshis::ZERO, Zatoshis::ZERO, Zatoshis::ZERO),
    };

    let target: TargetHeight = (chain_tip + 1).into();
    let spendable_now = select_notes(db, account, target, ConfirmationsPolicy::default())?;
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
        spendable,
        pending,
        total,
        notes,
    })
}

fn select_notes(
    db: &Db,
    account: AccountUuid,
    target: TargetHeight,
    policy: ConfirmationsPolicy,
) -> Result<BTreeMap<(String, u16), NoteReport>, VaultError> {
    let tip = target.saturating_sub(1);
    let notes = db
        .select_spendable_notes(
            account,
            TargetValue::AllFunds(MaxSpendMode::MaxSpendable),
            &[ShieldedPool::Orchard],
            target,
            policy,
            &[],
            LockFilter::Unfiltered,
        )
        .map_err(wallet_err)?;
    Ok(notes
        .take_orchard()
        .into_iter()
        .map(|n| {
            let txid = n.txid().to_string();
            let mined_height = n.mined_height().map(u32::from);
            let confirmations = mined_height.map(|h| u32::from(tip).saturating_sub(h) + 1);
            (
                (txid.clone(), n.output_index()),
                NoteReport {
                    txid,
                    output_index: n.output_index(),
                    value: Zatoshis::from_u64(n.note().value().inner()).unwrap_or(Zatoshis::ZERO),
                    mined_height,
                    confirmations,
                    spendable: false,
                },
            )
        })
        .collect())
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
