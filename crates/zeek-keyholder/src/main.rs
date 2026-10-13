//! `zeek-keyholder` binary.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use tokio::sync::{Mutex, RwLock};
use zeek_engine::config::{GameConfig, DEFAULT_RELAYS};
use zeek_engine::db::Db;
use zeek_keyholder::api::{router, AppState};
use zeek_keyholder::config::KeyholderConfig;
use zeek_keyholder::db::NodeDb;
use zeek_keyholder::dkg::{self, DkgState, KeyMaterial};
use zeek_keyholder::follower::Follower;
use zeek_keyholder::identity::Identity;
use zeek_keyholder::signing::SignerState;

#[derive(Parser)]
#[command(
    name = "zeek-keyholder",
    version,
    about = "Zeek keyholder node (FROST DKG participant and payout verifier)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate an ed25519 identity key under a secrets directory and print
    /// its public key (used for keyholders and for the coordinator).
    IdentityKeygen(KeygenArgs),
    /// Print this node's identity public key.
    Identity(ConfigArgs),
    /// Print the vault address and UIVK this node derived in the DKG ceremony.
    Address(ConfigArgs),
    /// Serve the API, follow the chain, and (with --dkg) run the ceremony first.
    Serve(ServeArgs),
}

#[derive(Args)]
struct ConfigArgs {
    /// Node config file (public, committed).
    #[arg(long, env = "ZEEK_KEYHOLDER_CONFIG")]
    config: PathBuf,
}

#[derive(Args)]
struct KeygenArgs {
    /// Secrets directory to write identity.json into (created 0700).
    #[arg(long)]
    secrets_dir: PathBuf,
    /// Replace an existing identity file.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct ServeArgs {
    /// Node config file (public, committed).
    #[arg(long, env = "ZEEK_KEYHOLDER_CONFIG")]
    config: PathBuf,
    /// Run the DKG ceremony with this 32-byte hex session id (same on all nodes).
    #[arg(long)]
    dkg: Option<String>,
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
    let load = |path: &PathBuf| {
        KeyholderConfig::load(path).with_context(|| format!("loading {}", path.display()))
    };
    match cli.command {
        Command::IdentityKeygen(args) => {
            let identity = Identity::generate();
            identity.save(&args.secrets_dir.join("identity.json"), args.force)?;
            println!("{}", identity.public_key_hex());
            Ok(())
        }
        Command::Identity(args) => {
            let config = load(&args.config)?;
            println!(
                "{}",
                Identity::load(&config.identity_path())?.public_key_hex()
            );
            Ok(())
        }
        Command::Address(args) => {
            let config = load(&args.config)?;
            let keys = KeyMaterial::load(&config)?;
            println!("{}", keys.vault.address);
            println!("{}", keys.vault.uivk()?);
            Ok(())
        }
        Command::Serve(args) => {
            let config = load(&args.config)?;
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("starting async runtime")?
                .block_on(serve(config, args))
        }
    }
}

async fn serve(config: KeyholderConfig, args: ServeArgs) -> anyhow::Result<()> {
    let identity = Identity::load(&config.identity_path())
        .context("loading the identity key (run `identity-keygen` first)")?;
    let game = GameConfig::load(&config.game_config)?;
    let chain_db = Db::open(&config.engine_db_path())?;
    let node_db = NodeDb::open(&config.node_db_path())?;
    let keys = match KeyMaterial::load(&config) {
        Ok(k) => Some(Arc::new(k)),
        Err(zeek_keyholder::KeyholderError::NoKeyShare) => None,
        Err(e) => return Err(e.into()),
    };
    tracing::info!(
        identifier = config.identifier,
        has_key_share = keys.is_some(),
        bind = %config.bind,
        "keyholder starting"
    );
    let state = Arc::new(AppState {
        config: config.clone(),
        game: game.clone(),
        identity,
        dkg: DkgState::default(),
        signer: SignerState::default(),
        keys: RwLock::new(keys),
        chain_db: Mutex::new(chain_db),
        node_db: Mutex::new(node_db),
        endpoint: Mutex::new(None),
    });

    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .with_context(|| format!("binding {}", config.bind))?;
    let app = router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    });

    if let Some(session_hex) = args.dkg {
        if state.keys.read().await.is_some() {
            tracing::info!("key share already present; skipping the DKG ceremony");
        } else {
            let session = hex::decode(&session_hex)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                .context("--dkg must be 32 hex bytes")?;
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()?;
            let output =
                dkg::run_ceremony(&config, &state.identity, &state.dkg, session, &client).await?;
            println!("DKG complete. Vault address: {}", output.address);
            if let Some(pk) = &output.sealing_public_key {
                println!("Payout sealing public key: {}", hex::encode(pk.to_bytes()));
                println!(
                    "Sealing key written to {}; copy it to the other keyholders and the engine (local-file distribution).",
                    config.secrets_dir.join("payout-sealing.json").display()
                );
            } else {
                println!(
                    "Waiting for the lead keyholder's payout-sealing.json to be copied into {}",
                    config.secrets_dir.display()
                );
            }
            match KeyMaterial::load(&config) {
                Ok(k) => *state.keys.write().await = Some(Arc::new(k)),
                Err(e) => {
                    tracing::warn!(error = %e, "key material not loadable yet (sealing key missing?); restart after copying it")
                }
            }
        }
    }

    let relays: Vec<String> = DEFAULT_RELAYS.split(',').map(str::to_owned).collect();
    let mut follower = Follower::new(game, config.lightwalletd.clone(), relays)?;
    loop {
        if server.is_finished() {
            break;
        }
        let pass = {
            let mut db = state.chain_db.lock().await;
            follower.pass(&mut db).await
        };
        *state.endpoint.lock().await = follower.endpoint().map(str::to_owned);
        if let Err(e) = pass {
            tracing::error!(error = %e, "follower pass failed; retrying after the poll interval");
        }
        if state.keys.read().await.is_none() {
            if let Ok(k) = KeyMaterial::load(&config) {
                tracing::info!("key material loaded");
                *state.keys.write().await = Some(Arc::new(k));
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(config.poll_secs)) => {}
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    tracing::info!("keyholder stopping");
    Ok(())
}
