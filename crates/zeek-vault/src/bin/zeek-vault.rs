//! `zeek-vault` command-line interface (Zcash testnet only).

use anyhow::{bail, Context};
use clap::{Args, Parser, Subcommand};
use rand_09::TryRngCore;
use rand_core::OsRng;
use zcash_protocol::consensus::BlockHeight;
use zeek_vault::config::Config;
use zeek_vault::send::{format_zec, SendRequest};
use zeek_vault::store::{KeyholderFile, SealingKeyFile, SecretsDir, VaultFile};
use zeek_vault::{frost, keys, lightwalletd, send, wallet, VaultError};

#[derive(Parser)]
#[command(
    name = "zeek-vault",
    version,
    about = "2-of-3 FROST vault for Zeek (Zcash testnet)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate FROST key shares and the vault's Orchard viewing key.
    Keygen(KeygenArgs),
    /// Print the vault's unified address and unified full viewing key.
    Address,
    /// Scan testnet via lightwalletd for the vault's notes.
    Sync(SyncArgs),
    /// Report the vault's Ironwood (and legacy Orchard) balance and notes.
    Balance,
    /// Spend Ironwood funds from the vault with a FROST threshold signature.
    Send(SendArgs),
    /// Generate the payout sealing key pair (HPKE) that diggers seal their
    /// payout receivers to; writes .secrets/payout-sealing.json.
    SealingKeygen(SealingKeygenArgs),
}

#[derive(Args)]
struct SealingKeygenArgs {
    /// Replace an existing sealing key file.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct KeygenArgs {
    /// Minimum number of keyholders needed to sign.
    #[arg(long, default_value_t = 2)]
    threshold: u16,
    /// Total number of keyholders.
    #[arg(long, default_value_t = 3)]
    signers: u16,
    /// Replace existing key files.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct SyncArgs {
    /// Block height to start scanning from when importing the account for the
    /// first time (defaults to the current chain tip).
    #[arg(long)]
    birthday: Option<u32>,
}

#[derive(Args)]
struct SendArgs {
    /// Destination testnet unified address (must have an Orchard receiver,
    /// which is how the Ironwood pool is addressed).
    #[arg(long)]
    to: String,
    /// Amount in ZEC, e.g. 0.001.
    #[arg(long)]
    amount: String,
    /// Comma-separated keyholder numbers that will sign, e.g. 1,3.
    #[arg(long, value_delimiter = ',')]
    signers: Vec<u16>,
    /// Optional UTF-8 memo.
    #[arg(long)]
    memo: Option<String>,
}

fn main() -> anyhow::Result<()> {
    // `.env` is optional; a missing file is not an error.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let config = Config::from_env()?;
    let secrets = SecretsDir::new(config.secrets_dir());

    match cli.command {
        Command::Keygen(args) => keygen(&secrets, args),
        Command::Address => address(&secrets),
        Command::Sync(args) => runtime()?.block_on(sync(&config, &secrets, args)),
        Command::Balance => balance(&config, &secrets),
        Command::Send(args) => runtime()?.block_on(send_cmd(&config, &secrets, args)),
        Command::SealingKeygen(args) => sealing_keygen(&secrets, args),
    }
}

fn sealing_keygen(secrets: &SecretsDir, args: SealingKeygenArgs) -> anyhow::Result<()> {
    // `OsRng` is fallible in rand 0.9; `unwrap_err` adapts it to the infallible
    // `CryptoRng` the sealing keygen needs (an OS RNG failure aborts keygen).
    let mut rng = rand_09::rngs::OsRng.unwrap_err();
    let (private_key, public_key) = zeek_core::seal::generate_sealing_keypair(&mut rng);
    let file = SealingKeyFile::new(&private_key);
    secrets.write_sealing_key(&file, args.force)?;
    println!(
        "Wrote payout sealing key to {}",
        secrets.sealing_path().display()
    );
    println!(
        "Sealing public key (public config): {}",
        hex::encode(public_key.to_bytes())
    );
    Ok(())
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting async runtime")
}

fn keygen(secrets: &SecretsDir, args: KeygenArgs) -> anyhow::Result<()> {
    if !args.force {
        secrets.check_absent(args.signers)?;
    }

    let out = frost::dealer_keygen(args.threshold, args.signers, OsRng)?;
    let group_key = frost::group_key_bytes(&out.public_key_package)?;
    let fvk = keys::derive_full_viewing_key(&group_key, OsRng)?;
    let ufvk = keys::unified_full_viewing_key(&fvk)?;
    let address = keys::encode_address(&keys::orchard_only_address(&ufvk)?);

    let vault = VaultFile::new(
        args.threshold,
        args.signers,
        &out.public_key_package,
        &fvk,
        &ufvk,
        &address,
    )?;
    secrets.write_vault(&vault)?;
    for (id, key_package) in &out.key_packages {
        let n = frost::identifier_number(id)?;
        secrets.write_keyholder(n, &KeyholderFile::new(n, key_package)?)?;
    }

    println!(
        "Generated a {}-of-{} FROST vault (trusted dealer; production will use DKG).",
        args.threshold, args.signers
    );
    println!("Secrets directory: {}", secrets.path().display());
    for n in 1..=args.signers {
        println!("  keyholder {n}: {}", secrets.keyholder_path(n).display());
    }
    println!("  public vault:  {}", secrets.vault_path().display());
    println!("Group verifying key (ak): {}", hex::encode(group_key));
    println!("Address: {address}");
    Ok(())
}

fn address(secrets: &SecretsDir) -> anyhow::Result<()> {
    let vault = secrets.load_vault()?;
    // Re-derive from the stored viewing key so a tampered file is noticed.
    let fvk = vault.orchard_fvk()?;
    let ufvk = keys::unified_full_viewing_key(&fvk)?;
    let derived = keys::encode_address(&keys::orchard_only_address(&ufvk)?);
    if derived != vault.address {
        bail!(
            "vault.json address {} does not match the address derived from its viewing key {}",
            vault.address,
            derived
        );
    }
    println!("Network: testnet");
    println!("Unified address (Orchard receiver; payments land in the Ironwood pool):");
    println!("  {}", vault.address);
    println!("Unified full viewing key:");
    println!("  {}", keys::encode_ufvk(&ufvk));
    println!("Unified incoming viewing key (external scope; detects digs, never change):");
    println!("  {}", vault.uivk()?);
    Ok(())
}

async fn sync(config: &Config, secrets: &SecretsDir, args: SyncArgs) -> anyhow::Result<()> {
    let mut vault = secrets.load_vault()?;
    let ufvk = vault.ufvk()?;
    println!(
        "Connecting to {} ...",
        config.lightwalletd_servers().join(", ")
    );
    let mut client = config.connect().await?;
    let info = lightwalletd::lightd_info(&mut client).await?;
    println!(
        "Connected: chain={} lightwalletd={} zcashd/zebra={} height={}",
        info.chain_name, info.version, info.zcashd_build, info.block_height
    );

    let mut db = wallet::open(config.data_dir())?;
    let birthday = args.birthday.map(BlockHeight::from_u32);
    let had_account = wallet::find_account(&db, &ufvk)?.is_some();
    let (account, birthday_height) =
        wallet::ensure_account(&mut db, &mut client, &ufvk, birthday).await?;
    if !had_account {
        println!("Imported view-only account {account:?} with birthday {birthday_height}");
        vault.birthday_height = Some(birthday_height.into());
        secrets.write_vault(&vault)?;
    }

    println!("Syncing (this scans from height {birthday_height}) ...");
    wallet::sync(&mut db, &mut client).await?;
    let report = wallet::balance(&db, account)?;
    println!(
        "Synced. chain tip {} fully scanned {} | Ironwood spendable {} ZEC pending {} ZEC | legacy Orchard total {} ZEC",
        report.chain_tip,
        report.fully_scanned,
        format_zec(report.ironwood.spendable),
        format_zec(report.ironwood.pending),
        format_zec(report.orchard.total)
    );
    Ok(())
}

fn balance(config: &Config, secrets: &SecretsDir) -> anyhow::Result<()> {
    let vault = secrets.load_vault()?;
    let ufvk = vault.ufvk()?;
    let db = wallet::open(config.data_dir())?;
    let account = wallet::find_account(&db, &ufvk)?.ok_or(VaultError::AccountMissing)?;
    let report = wallet::balance(&db, account)?;

    println!("Vault address: {}", vault.address);
    println!("Chain tip (wallet view): {}", report.chain_tip);
    println!("Fully scanned height:    {}", report.fully_scanned);
    println!("Ironwood pool (where payments to the vault land since NU6.3):");
    println!(
        "  spendable:             {} ZEC",
        format_zec(report.ironwood.spendable)
    );
    println!(
        "  pending confirmations: {} ZEC",
        format_zec(report.ironwood.pending)
    );
    println!(
        "  total:                 {} ZEC",
        format_zec(report.ironwood.total)
    );
    if report.orchard.total.into_u64() > 0 {
        println!("Legacy Orchard pool (closed to ordinary payments since NU6.3; not spendable by this vault):");
        println!(
            "  total:                 {} ZEC",
            format_zec(report.orchard.total)
        );
    } else {
        println!("Legacy Orchard pool:     none");
    }
    println!(
        "Notes: {} (received notes need {} confirmations, change needs {})",
        report.notes.len(),
        report.min_confirmations,
        report.change_confirmations
    );
    for note in &report.notes {
        let depth = note
            .confirmations
            .map(|c| format!("{c} confirmations"))
            .unwrap_or_else(|| "unmined".to_owned());
        println!(
            "  [{}] {}:{}  {} ZEC  mined at {}  {}  {}",
            wallet::pool_name(note.pool),
            note.txid,
            note.output_index,
            format_zec(note.value),
            note.mined_height
                .map(|h| h.to_string())
                .unwrap_or_else(|| "-".into()),
            depth,
            match (note.pool, note.spendable) {
                (orchard::ValuePool::Orchard, _) => "legacy pool, not spendable here",
                (_, true) => "spendable",
                (_, false) => "not yet spendable",
            }
        );
    }
    Ok(())
}

async fn send_cmd(config: &Config, secrets: &SecretsDir, args: SendArgs) -> anyhow::Result<()> {
    let vault = secrets.load_vault()?;
    let public_key_package = vault.public_key_package()?;

    if args.signers.len() < usize::from(vault.threshold) {
        bail!(
            "{}",
            VaultError::NotEnoughSigners {
                given: args.signers.len(),
                threshold: vault.threshold
            }
        );
    }
    let mut key_packages = Vec::with_capacity(args.signers.len());
    for n in &args.signers {
        key_packages.push(secrets.load_keyholder(*n, &vault)?);
    }
    let signers: Vec<&frost::KeyPackage> = key_packages.iter().collect();

    let ufvk = vault.ufvk()?;
    let mut client = config.connect().await?;
    let mut db = wallet::open(config.data_dir())?;
    let account = wallet::find_account(&db, &ufvk)?.ok_or(VaultError::AccountMissing)?;

    println!("Refreshing wallet state before spending ...");
    wallet::sync(&mut db, &mut client).await?;

    let request = SendRequest {
        to: args.to,
        amount_zec: args.amount,
        memo: args.memo,
    };
    let outcome = send::send(
        &mut db,
        &mut client,
        account,
        &public_key_package,
        &signers,
        &request,
    )
    .await?;

    println!("Broadcast txid: {}", outcome.txid);
    println!(
        "Transaction: {:?} on consensus branch {:?} (target height {})",
        outcome.version, outcome.branch, outcome.target_height
    );
    println!("FROST-signed spends: {}", outcome.signed_spends.len());
    for spend in &outcome.signed_spends {
        println!(
            "  {} action {}  value {}",
            wallet::pool_name(spend.pool),
            spend.index,
            spend
                .value
                .map(|v| format_zec(zcash_protocol::value::Zatoshis::const_from_u64(v)))
                .unwrap_or_else(|| "redacted".into())
        );
    }
    println!("Fee paid: {} ZEC", format_zec(outcome.fee));
    println!("Signers: {:?}", args.signers);
    Ok(())
}
