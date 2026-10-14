//! `zeek-engine` binary: follows testnet, settles rounds, serves the API.
//! `zeek-engine audit` instead rebuilds the vaults' history from chain data
//! and reconciles it against the engine's databases.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use tokio::sync::{broadcast, Mutex};
use zeek_engine::api::{router, AppState};
use zeek_engine::audit::{self, VaultSpec};
use zeek_engine::beacon::RelayFetcher;
use zeek_engine::chain::{self, LightwalletdSource};
use zeek_engine::config::{EngineSettings, GameConfig};
use zeek_engine::coordinator::{Coordinator, CoordinatorSettings};
use zeek_engine::db::Db;
use zeek_engine::decrypt::DigScanner;
use zeek_engine::processor::{Event, Processor};

#[derive(Parser)]
#[command(
    name = "zeek-engine",
    version,
    about = "Zeek round engine (Zcash testnet)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Rebuild every vault's history from chain data and reconcile it against
    /// the engine's settlements, payouts and ledger. Prints the report.
    Audit(AuditArgs),
    /// Complete a payout record stuck in the `signing` state: finds, on the
    /// chain, the transaction that spent the round's charged dig notes and
    /// records its txid. Refuses if the notes are unspent or the fee differs
    /// from the record.
    PayoutRecover(RecoverArgs),
}

#[derive(Args)]
struct RecoverArgs {
    /// The round whose payout record is stuck.
    #[arg(long)]
    round: u32,
    /// Keyholders that released shares (from their logs), comma-separated;
    /// required when the record does not name them yet.
    #[arg(long, value_delimiter = ',')]
    signers: Vec<u16>,
    /// First height to scan (default: the vault's birthday).
    #[arg(long)]
    from: Option<u32>,
}

#[derive(Args)]
struct AuditArgs {
    /// Vaults to audit: `name=<secrets dir>[,engine=<db>][,game=<config>][,wallet=<dir>]`.
    /// Default: the DKG vault and the old dealer vault of this repository.
    #[arg(long = "vault")]
    vaults: Vec<String>,
    /// First height to scan (default: the earliest vault birthday).
    #[arg(long)]
    from: Option<u32>,
}

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
    let settings = EngineSettings::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?;
    match cli.command {
        Some(Command::Audit(args)) => runtime.block_on(run_audit(settings, args)),
        Some(Command::PayoutRecover(args)) => {
            let config = GameConfig::load(&settings.game_config_path)
                .with_context(|| format!("loading {}", settings.game_config_path.display()))?;
            runtime.block_on(run_recover(settings, config, args))
        }
        None => {
            let config = GameConfig::load(&settings.game_config_path)
                .with_context(|| format!("loading {}", settings.game_config_path.display()))?;
            runtime.block_on(run(settings, config))
        }
    }
}

async fn run_recover(
    settings: EngineSettings,
    config: GameConfig,
    args: RecoverArgs,
) -> anyhow::Result<()> {
    let coordinator_settings = CoordinatorSettings::from_env()?.context(
        "the coordinator environment (ZEEK_KEYHOLDERS, ZEEK_VAULT_SECRETS_DIR) is required",
    )?;
    let db = Db::open(&settings.db_path)?;
    let processor = Processor::new(config);
    let record = db
        .payout(args.round)?
        .with_context(|| format!("round {} has no payout record", args.round))?;
    anyhow::ensure!(
        record.state == zeek_engine::db::PayoutState::Signing,
        "round {} is already {:?}",
        args.round,
        record.state
    );
    let signers = if record.signers.is_empty() {
        anyhow::ensure!(
            !args.signers.is_empty(),
            "the record names no signers; pass --signers"
        );
        args.signers.clone()
    } else {
        record.signers.clone()
    };
    let row = db
        .settlement(args.round)?
        .with_context(|| format!("round {} is not settled", args.round))?;
    let notes: Vec<(String, u16)> = processor
        .payout_inputs(&db, &row)?
        .into_iter()
        .map(|n| (n.txid, n.output_index))
        .collect();
    anyhow::ensure!(
        !notes.is_empty(),
        "round {} has no charged notes",
        args.round
    );
    let vault =
        zeek_vault::store::SecretsDir::new(&coordinator_settings.vault_secrets_dir).load_vault()?;
    let from = args
        .from
        .or(vault.birthday_height)
        .context("no --from height and the vault file records no birthday")?;
    let mut source = LightwalletdSource::connect(&settings.lightwalletd_url).await?;
    let spent = audit::find_spending_tx(
        &mut source,
        from,
        &coordinator_settings.vault_secrets_dir,
        &notes,
    )
    .await?
    .context("the round's charged notes are not spent on the chain; nothing to recover")?;
    anyhow::ensure!(
        i128::from(spent.fee) == i128::from(record.fee),
        "the spending transaction's fee {} differs from the recorded fee {}",
        spent.fee,
        record.fee
    );
    db.mark_payout_broadcast(args.round, &spent.txid, spent.height, &signers)?;
    db.set_plan_status(args.round, zeek_engine::db::PlanStatus::Paid)?;
    println!(
        "round {}: payout {} mined at {} (fee {}) recorded as broadcast, signers {:?}",
        args.round, spent.txid, spent.height, spent.fee, signers
    );
    Ok(())
}

async fn run_audit(settings: EngineSettings, args: AuditArgs) -> anyhow::Result<()> {
    let specs: Vec<VaultSpec> = if args.vaults.is_empty() {
        vec![
            VaultSpec::parse(
                "dkg=.secrets,engine=data/zeek-engine.sqlite,game=config/game.testnet.json,wallet=data",
            )?,
            VaultSpec::parse(
                "old-dealer=.secrets/old-dealer,engine=data/old-vault/zeek-engine.sqlite,game=config/game.testnet.old-vault.json,wallet=data/old-vault",
            )?,
        ]
    } else {
        args.vaults
            .iter()
            .map(|v| VaultSpec::parse(v))
            .collect::<Result<_, _>>()?
    };
    let mut source = LightwalletdSource::connect(&settings.lightwalletd_url).await?;
    let outcome = audit::run(&mut source, args.from, &specs).await?;
    print!("{}", outcome.report);
    if outcome.mismatches > 0 {
        anyhow::bail!("audit found {} mismatch(es)", outcome.mismatches);
    }
    Ok(())
}

fn settings_url(settings: &EngineSettings) -> String {
    settings.lightwalletd_url.clone()
}

async fn run(settings: EngineSettings, config: GameConfig) -> anyhow::Result<()> {
    let settings_ref = settings.clone();
    let scanner = DigScanner::new(config.incoming_viewing_key()?);
    let db = Db::open(&settings.db_path)?;
    let mut source = LightwalletdSource::connect(&settings.lightwalletd_url).await?;
    let mut fetcher = RelayFetcher::new(settings.relays.clone())?;
    let (events, _) = broadcast::channel::<Event>(256);
    tracing::info!(
        genesis = config.genesis_height,
        blocks_per_round = config.params.blocks_per_round,
        db = %settings.db_path.display(),
        "engine starting"
    );

    let coordinator = match CoordinatorSettings::from_env()? {
        Some(settings) => {
            let coordinator = Coordinator::new(settings, &settings_url(&settings_ref))?;
            tracing::info!(
                vault = coordinator.vault_address(),
                "payout coordinator enabled"
            );
            Some(Arc::new(Mutex::new(coordinator)))
        }
        None => None,
    };
    let state = Arc::new(AppState {
        processor: Processor::new(config.clone()),
        db: Mutex::new(db),
        events: events.clone(),
        started: Instant::now(),
        coordinator: coordinator.clone(),
    });

    let listener = tokio::net::TcpListener::bind(&settings.bind)
        .await
        .with_context(|| format!("binding {}", settings.bind))?;
    tracing::info!(bind = %settings.bind, "API listening");
    let app = router(state.clone(), &settings.cors_origins);
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    });

    let mut processor = Processor::new(config.clone());
    let mut last_board = None;
    loop {
        if server.is_finished() {
            break;
        }
        let pass = async {
            let mut db = state.db.lock().await;
            let report = chain::ingest(&mut source, &mut db, &scanner, &config).await?;
            if report.new_blocks > 0 || report.new_notes > 0 || report.reorg_from.is_some() {
                tracing::info!(
                    tip = ?report.stored_tip,
                    chain_tip = report.chain_tip,
                    new_blocks = report.new_blocks,
                    new_notes = report.new_notes,
                    reorg_from = ?report.reorg_from,
                    "ingested"
                );
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let before = processor.pending_rounds(&db)?;
            let mut events_out = processor.process(&mut db, &mut fetcher, now).await?;
            let after = processor.pending_rounds(&db)?;
            for closed in after
                .iter()
                .filter(|p| !before.iter().any(|b| b.round == p.round))
            {
                events_out.push(Event::RoundClosed {
                    round: closed.round,
                    window: closed.window,
                    drand_round: closed.drand_round,
                    beacon_time: closed.beacon_time,
                });
            }
            if let Some(coordinator) = &coordinator {
                let mut coordinator = coordinator.lock().await;
                match coordinator.pay_next(&mut db, &processor, &config).await {
                    Ok(Some(report)) => events_out.push(Event::Paid {
                        round: report.round,
                        txid: report.txid,
                        signers: report.signers,
                    }),
                    Ok(None) => {}
                    Err(e) => tracing::error!(error = %e, "payout attempt failed"),
                }
            }
            let board = processor.board(&db)?;
            if last_board.as_ref() != Some(&board) {
                events_out.push(Event::Board(board.clone()));
                last_board = Some(board);
            }
            Ok::<_, anyhow::Error>(events_out)
        }
        .await;
        match pass {
            Ok(events_out) => {
                for event in events_out {
                    let _ = events.send(event);
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "engine pass failed; retrying after the poll interval")
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(settings.poll_secs)) => {}
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    tracing::info!("engine stopping");
    Ok(())
}
