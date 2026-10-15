//! `zeek-dig`: test players for Zeek on testnet.

/// `zeek-dig` plays on testnet.
const NETWORK: zeek_vault::Params = zeek_vault::Params::TestNetwork;

mod dig;
mod notes;
mod wallet;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use zeek_vault::config::Config;
use zeek_vault::store::SecretsDir;
use zeek_vault::wallet as vault_wallet;

use crate::dig::RoundChoice;
use crate::wallet::{balance_lines, Player};

#[derive(Parser)]
#[command(name = "zeek-dig", version, about = "Zeek test player (Zcash testnet)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new player wallet under .secrets/players/<name>/.
    Init { name: String },
    /// Print the player's unified address (Orchard receiver). With --fresh,
    /// a new diversified address the wallet records as exposed (for pasting
    /// as an any-wallet payout address).
    Address {
        name: String,
        #[arg(long)]
        fresh: bool,
    },
    /// Scan testnet for the player's notes.
    Sync { name: String },
    /// Show the player's Ironwood balance.
    Balance { name: String },
    /// List the player's unspent notes with their decrypted memos, or every
    /// output of one transaction that belongs to the player (--txid).
    Notes {
        name: String,
        #[arg(long)]
        txid: Option<String>,
    },
    /// Dig: pay the vault with a sealed dig memo.
    Dig(DigArgs),
    /// Send shielded funds to any address with an Orchard-protocol receiver.
    Send {
        name: String,
        #[arg(long)]
        to: String,
        /// Amount in ZEC, e.g. 0.01.
        #[arg(long)]
        amount: String,
        /// Optional text memo.
        #[arg(long)]
        memo: Option<String>,
    },
    /// Build a win proof (prove-a-win) for the treasure payout this player
    /// received in a round, print it, and optionally submit it to the engine.
    ProveWin {
        name: String,
        #[arg(long)]
        round: u32,
        #[arg(long, default_value = "http://127.0.0.1:8787")]
        engine: String,
        /// Submit the proof to the engine's /v1/proofs.
        #[arg(long)]
        submit: bool,
    },
    /// Pay a ZIP 321 payment request URI exactly as an external wallet
    /// would (every payment, amounts and memo bytes as given).
    PayUri { name: String, uri: String },
}

#[derive(Args)]
struct DigArgs {
    name: String,
    /// Tile to dig, 0..=24.
    #[arg(long, conflicts_with = "tiles")]
    tile: Option<u8>,
    /// Several tiles in one transaction (comma-separated), e.g. 0,1,2. The
    /// outputs share a txid, so they are linkable to each other by anyone
    /// with the public UIVK.
    #[arg(long, value_delimiter = ',', conflicts_with = "tile")]
    tiles: Vec<u8>,
    /// Amount in ZEC per tile, e.g. 0.005.
    #[arg(long)]
    amount: String,
    /// Round to name in the memo: `auto` (the board's current round) or a number.
    #[arg(long, default_value = "auto")]
    round: String,
    /// Engine base URL.
    #[arg(long, env = "ZEEK_ENGINE_URL", default_value = "http://127.0.0.1:8787")]
    engine: String,
}

fn main() -> anyhow::Result<()> {
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
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?;

    match cli.command {
        Command::Init { name } => {
            let player = Player::new(secrets.path(), &name)?;
            player.init()?;
            println!("Created player `{name}`; address: {}", player.address()?);
            Ok(())
        }
        Command::Address { name, fresh: false } => {
            let player = Player::new(secrets.path(), &name)?;
            println!("{}", player.address()?);
            Ok(())
        }
        Command::Address { name, fresh: true } => {
            let player = Player::new(secrets.path(), &name)?;
            let mut db = player.open_db()?;
            let account = player
                .find_account(&db)?
                .with_context(|| format!("player `{name}` has not synced yet"))?;
            let (address, _) =
                zcash_client_backend::data_api::WalletWrite::get_next_available_address(
                    &mut db,
                    account,
                    zcash_keys::keys::UnifiedAddressRequest::ORCHARD,
                )
                .map_err(|e| anyhow::anyhow!("next address: {e}"))?
                .context("the wallet has no account for this player")?;
            println!("{}", address.encode(&NETWORK));
            Ok(())
        }
        Command::Sync { name } => runtime.block_on(async {
            let player = Player::new(secrets.path(), &name)?;
            let mut client = config.connect(zeek_core::params::Network::Test).await?;
            let mut db = player.open_db()?;
            let account = player.ensure_account(&mut db, &mut client).await?;
            vault_wallet::sync(&mut db, &mut client).await?;
            for line in balance_lines(&vault_wallet::balance(&db, account)?) {
                println!("{line}");
            }
            Ok(())
        }),
        Command::Balance { name } => {
            let player = Player::new(secrets.path(), &name)?;
            let db = player.open_db()?;
            let account = player
                .find_account(&db)?
                .with_context(|| format!("player `{name}` has not synced yet"))?;
            for line in balance_lines(&vault_wallet::balance(&db, account)?) {
                println!("{line}");
            }
            Ok(())
        }
        Command::Notes { name, txid } => runtime.block_on(async {
            let player = Player::new(secrets.path(), &name)?;
            let mut client = config.connect(zeek_core::params::Network::Test).await?;
            let usk = player.spending_key()?;
            let listed = match txid {
                Some(txid) => notes::notes_in_transaction(&mut client, &usk, &txid).await?,
                None => {
                    let mut db = player.open_db()?;
                    let account = player.ensure_account(&mut db, &mut client).await?;
                    vault_wallet::sync(&mut db, &mut client).await?;
                    let report = vault_wallet::balance(&db, account)?;
                    notes::notes_with_memos(&mut client, &usk, &report.notes).await?
                }
            };
            for note in listed {
                println!(
                    "{}:{} {} zat at {} [{}] {}",
                    note.txid,
                    note.output_index,
                    note.value,
                    note.height
                        .map(|h| h.to_string())
                        .unwrap_or_else(|| "unmined".into()),
                    note.scope,
                    note.memo
                );
            }
            Ok(())
        }),
        Command::Send {
            name,
            to,
            amount,
            memo,
        } => runtime.block_on(async {
            let player = Player::new(secrets.path(), &name)?;
            let destination = zeek_vault::send::parse_destination(&to, &NETWORK)?;
            let amount = zeek_vault::send::parse_zec(&amount)?;
            let memo = zeek_vault::send::parse_memo(memo.as_deref())?;
            let payment = zip321::Payment::new(destination, Some(amount), memo, None, None, vec![])
                .map_err(|e| anyhow::anyhow!("payment: {e}"))?;
            let request = zip321::TransactionRequest::new(vec![payment])
                .map_err(|e| anyhow::anyhow!("payment request: {e}"))?;
            let mut client = config.connect(zeek_core::params::Network::Test).await?;
            let mut db = player.open_db()?;
            player.ensure_account(&mut db, &mut client).await?;
            let outcome = dig::pay_request(&player, &mut db, &mut client, request).await?;
            println!(
                "Sent {} zatoshis (fee {})",
                outcome.total_zat, outcome.fee_zat
            );
            println!("txid: {}", outcome.txid);
            Ok(())
        }),
        Command::ProveWin {
            name,
            round,
            engine,
            submit,
        } => runtime.block_on(async {
            use base64::Engine;
            let player = Player::new(secrets.path(), &name)?;
            let usk = player.spending_key()?;
            let sk = usk.orchard();
            let fvk = orchard::keys::FullViewingKey::from(sk);
            let ask = orchard::keys::SpendAuthorizingKey::from(sk);
            let url = format!("{}/v1/rounds/{round}/proof", engine.trim_end_matches('/'));
            let bundle: zeek_core::proof::ProofBundle =
                reqwest::get(&url).await?.error_for_status()?.json().await?;
            let openings = bundle.payout_openings.as_ref().context(
                "the round's bundle has no payout openings yet (not mined, or an old engine)",
            )?;
            let index = openings
                .outputs
                .iter()
                .find(|o| {
                    hex::decode(&o.recipient)
                        .ok()
                        .and_then(|b| <[u8; 43]>::try_from(b).ok())
                        .and_then(|b| Option::from(orchard::Address::from_raw_address_bytes(&b)))
                        .is_some_and(|a: orchard::Address| fvk.scope_for_address(&a).is_some())
                })
                .map(|o| o.payout_index)
                .context("none of this round's payouts went to this player")?;
            eprintln!("building the proving key and proving (a few seconds)...");
            let pk = orchard::circuit::ProvingKey::build(
                orchard::circuit::OrchardCircuitVersion::PostNu6_3,
            );
            let proof =
                zeek_verify::win::create(&bundle, index, &fvk, &ask, &pk, &mut rand_core::OsRng)?;
            let encoded = proof.encode()?;
            let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&encoded);
            println!(
                "round {round}, payout {index}, id {}",
                zeek_verify::win::WinProof::id(&encoded)
            );
            println!("{text}");
            if submit {
                let response = reqwest::Client::new()
                    .post(format!("{}/v1/proofs", engine.trim_end_matches('/')))
                    .json(&serde_json::json!({ "proof": text }))
                    .send()
                    .await?;
                let status = response.status();
                println!("submitted: {status} {}", response.text().await?);
            }
            Ok(())
        }),
        Command::PayUri { name, uri } => runtime.block_on(async {
            let player = Player::new(secrets.path(), &name)?;
            let request = zip321::TransactionRequest::from_uri(uri.trim())
                .map_err(|e| anyhow::anyhow!("not a valid ZIP 321 request: {e:?}"))?;
            let mut client = config.connect(zeek_core::params::Network::Test).await?;
            let mut db = player.open_db()?;
            player.ensure_account(&mut db, &mut client).await?;
            let outcome = dig::pay_request(&player, &mut db, &mut client, request).await?;
            println!(
                "Paid {} payment(s) totalling {} zatoshis (fee {})",
                outcome.payments, outcome.total_zat, outcome.fee_zat
            );
            println!("txid: {}", outcome.txid);
            Ok(())
        }),
        Command::Dig(args) => runtime.block_on(async {
            let tiles: Vec<u8> = match (args.tile, args.tiles.is_empty()) {
                (Some(t), _) => vec![t],
                (None, false) => args.tiles.clone(),
                (None, true) => anyhow::bail!("pass --tile N or --tiles a,b,c"),
            };
            if tiles.iter().any(|t| *t >= zeek_core::TILE_COUNT) {
                anyhow::bail!("tiles must be 0..=24");
            }
            let mut unique = tiles.clone();
            unique.sort_unstable();
            unique.dedup();
            if unique.len() != tiles.len() {
                anyhow::bail!("tiles must be distinct");
            }
            let choice = if args.round == "auto" {
                RoundChoice::Auto
            } else {
                RoundChoice::Explicit(
                    args.round
                        .parse()
                        .context("--round must be `auto` or a number")?,
                )
            };
            let player = Player::new(secrets.path(), &args.name)?;
            let (engine_config, board) = dig::fetch_engine(&args.engine).await?;
            println!(
                "Board: round {} ({}) tip {} blocks remaining {}",
                board.round, board.status, board.tip_height, board.blocks_remaining
            );
            let mut client = config.connect(zeek_core::params::Network::Test).await?;
            let mut db = player.open_db()?;
            player.ensure_account(&mut db, &mut client).await?;
            let outcome = dig::dig(
                &player,
                &mut db,
                &mut client,
                &engine_config,
                &board,
                &dig::DigRequest {
                    choice,
                    tiles,
                    amount_zec: args.amount,
                },
            )
            .await?;
            println!(
                "Dug tiles {:?} for round {} with {} zatoshis each (fee {}), target height {}",
                outcome.tiles,
                outcome.memo_round,
                outcome.amount_zat,
                outcome.fee_zat,
                outcome.target_height
            );
            println!("txid: {}", outcome.txid);
            println!(
                "Sealed {} distinct fresh payout receivers (diversifier indices {:?})",
                outcome.diversifier_indices.len(),
                outcome.diversifier_indices
            );
            Ok(())
        }),
    }
}
