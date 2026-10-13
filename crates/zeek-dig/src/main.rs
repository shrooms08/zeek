//! `zeek-dig`: test players for Zeek on testnet.

mod dig;
mod wallet;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use zeek_vault::config::Config;
use zeek_vault::store::SecretsDir;
use zeek_vault::{lightwalletd, wallet as vault_wallet};

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
    /// Print the player's unified address (Orchard receiver).
    Address { name: String },
    /// Scan testnet for the player's notes.
    Sync { name: String },
    /// Show the player's Ironwood balance.
    Balance { name: String },
    /// Dig: pay the vault with a sealed dig memo.
    Dig(DigArgs),
}

#[derive(Args)]
struct DigArgs {
    name: String,
    /// Tile to dig, 0..=24.
    #[arg(long)]
    tile: u8,
    /// Amount in ZEC, e.g. 0.005.
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
        Command::Address { name } => {
            let player = Player::new(secrets.path(), &name)?;
            println!("{}", player.address()?);
            Ok(())
        }
        Command::Sync { name } => runtime.block_on(async {
            let player = Player::new(secrets.path(), &name)?;
            let mut client = lightwalletd::connect(config.lightwalletd_url()?).await?;
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
        Command::Dig(args) => runtime.block_on(async {
            if args.tile >= zeek_core::TILE_COUNT {
                anyhow::bail!("tile must be 0..=24");
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
            let mut client = lightwalletd::connect(config.lightwalletd_url()?).await?;
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
                    tile: args.tile,
                    amount_zec: args.amount,
                },
            )
            .await?;
            println!(
                "Dug tile {} for round {} with {} zatoshis (fee {}), target height {}",
                outcome.tile,
                outcome.memo_round,
                outcome.amount_zat,
                outcome.fee_zat,
                outcome.target_height
            );
            println!("txid: {}", outcome.txid);
            Ok(())
        }),
    }
}
