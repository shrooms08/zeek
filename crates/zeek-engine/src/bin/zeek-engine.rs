//! `zeek-engine` binary: follows testnet, settles rounds, serves the API.
//! `zeek-engine audit` instead rebuilds the vaults' history from chain data
//! and reconciles it against the engine's databases.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use tokio::sync::broadcast;
use zeek_engine::api::{router, AppState};
use zeek_engine::audit::{self, VaultSpec};
use zeek_engine::beacon::RelayFetcher;
use zeek_engine::chain::{self, PoolSource};
use zeek_engine::config::{EngineSettings, GameConfig};
use zeek_engine::coordinator::{Coordinator, CoordinatorSettings};
use zeek_engine::db::Db;
use zeek_engine::decrypt::DigScanner;
use zeek_engine::edge;
use zeek_engine::mempool::{self, Landing, LightwalletdMempool};
use zeek_engine::processor::{Event, Processor};
use zeek_engine::relay::{LightwalletdBackend, RelayState};
use zeek_engine::store::Store;

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
    let network = config.params.network;
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
    let mut source = PoolSource::new(Arc::new(settings.pool(network)?));
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
    // The audited vaults are testnet vaults.
    let mut source = PoolSource::new(Arc::new(settings.pool(zeek_core::params::Network::Test)?));
    let outcome = audit::run(&mut source, args.from, &specs).await?;
    print!("{}", outcome.report);
    if outcome.mismatches > 0 {
        anyhow::bail!("audit found {} mismatch(es)", outcome.mismatches);
    }
    Ok(())
}

/// How often every lightwalletd server is health-checked.
const PROBE_EVERY: Duration = Duration::from_secs(60);

/// Longest an engine pass may run (a payout proof takes about a minute).
const PASS_TIMEOUT: Duration = Duration::from_secs(600);

async fn run(settings: EngineSettings, config: GameConfig) -> anyhow::Result<()> {
    let scanner = DigScanner::new(config.incoming_viewing_key()?);
    let store = Arc::new(Store::open(&settings.db_path)?);
    // One pool for the chain loop, the relay, the mempool watcher and the
    // coordinator: one health view, one failover order.
    let pool = Arc::new(settings.pool(config.params.network)?);
    pool.probe().await;
    tracing::info!(servers = ?settings.servers(config.params.network), "lightwalletd pool");
    let source = PoolSource::new(pool.clone());
    let fetcher = RelayFetcher::new(settings.relays.clone())?;
    let (events, _) = broadcast::channel::<Event>(256);
    tracing::info!(
        genesis = config.genesis_height,
        blocks_per_round = config.params.blocks_per_round,
        db = %settings.db_path.display(),
        "engine starting"
    );

    let coordinator = match CoordinatorSettings::from_env()? {
        Some(settings) => {
            let coordinator = Coordinator::new(settings, pool.clone())?;
            tracing::info!(
                vault = coordinator.vault_address(),
                "payout coordinator enabled"
            );
            Some(coordinator)
        }
        None => None,
    };
    let (keyholder_ids, threshold) = match &coordinator {
        Some(c) => c.keyholder_ids(),
        None => (Vec::new(), 0),
    };
    let state = Arc::new(AppState {
        keyholder_ids,
        threshold,
        share_cache: zeek_engine::share::ShareCache::default(),
        app_url: std::env::var("ZEEK_APP_URL")
            .unwrap_or_else(|_| "http://localhost:5174".to_owned()),
        public_url: std::env::var("ZEEK_PUBLIC_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8787".to_owned()),
        processor: Processor::new(config.clone()),
        store: store.clone(),
        events: events.clone(),
        started: Instant::now(),
        ledger: coordinator.as_ref().map(Coordinator::ledger_view),
        chain_servers: Some(pool.clone()),
        stream_slots: Arc::new(tokio::sync::Semaphore::new(zeek_engine::api::MAX_STREAMS)),
        relay: Some(Arc::new(RelayState::new(
            Box::new(LightwalletdBackend::new(pool.clone())),
            RelayState::rate_from_env(),
        ))),
        campfire_filter: zeek_engine::campfire::Filter::from_env()?,
        proof_limiter: zeek_engine::relay::RateLimiter::new(
            zeek_engine::api::PROOF_SUBMISSIONS_PER_MIN,
        ),
        landing: Arc::new(Landing::new(events.clone())),
    });

    // Mempool watcher: its own stream and task; it never touches the
    // database and survives any server error (each session picks a server).
    {
        let pool = pool.clone();
        tokio::spawn(mempool::watch(
            move || {
                let pool = pool.clone();
                async move { LightwalletdMempool::connect(&pool).await }
            },
            DigScanner::new(config.incoming_viewing_key()?),
            config.params.clone(),
            state.landing.clone(),
        ));
    }

    let listener = tokio::net::TcpListener::bind(&settings.bind)
        .await
        .with_context(|| format!("binding {}", settings.bind))?;
    tracing::info!(bind = %settings.bind, "local API listening");
    let app = router(state.clone(), &settings.cors_origins);
    let local = edge::local(app.clone());
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move {
        axum::serve(
            listener,
            local.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    });
    if let Some((bind, secret_file)) = &settings.public {
        let secret = std::fs::read_to_string(secret_file)
            .with_context(|| format!("reading the proxy secret {}", secret_file.display()))?;
        let public = edge::public(app, edge::ProxySecret::new(&secret)?);
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .with_context(|| format!("binding {bind}"))?;
        tracing::info!(bind = %bind, "public API listening (proxy secret required)");
        servers.spawn(async move {
            axum::serve(
                listener,
                public.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
        });
    }
    let server = async move {
        while let Some(result) = servers.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(anyhow::Error::from(e)),
                Err(e) => return Err(anyhow::Error::from(e)),
            }
        }
        Ok(())
    };

    // The chain loop runs as its own task: the store's guards are not
    // `Send`, so the compiler rejects any database access held across an
    // `.await` (network, beacon, proving, signing) inside it.
    let chain_loop = tokio::spawn(chain_loop(
        state.clone(),
        source,
        fetcher,
        coordinator,
        scanner,
        config,
        settings.poll_secs,
    ));
    tokio::select! {
        result = server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "API server stopped");
            }
        }
        result = chain_loop => {
            if let Err(e) = result {
                tracing::error!(error = %e, "chain loop stopped");
            }
        }
    }
    tracing::info!("engine stopping");
    Ok(())
}

/// Follows the chain, settles rounds and pays them, until Ctrl-C.
async fn chain_loop(
    state: Arc<AppState>,
    mut source: PoolSource,
    mut fetcher: RelayFetcher,
    mut coordinator: Option<Coordinator>,
    scanner: DigScanner,
    config: GameConfig,
    poll_secs: u64,
) {
    let store = state.store.clone();
    let mut processor = Processor::new(config.clone());
    let mut last_board = None;
    // Campfire events announce messages mined after startup.
    let mut last_campfire_height = match store.read(|db| db.tip()) {
        Ok(tip) => tip.map_or(0, |b| b.height),
        Err(e) => {
            tracing::error!(error = %e, "could not read the stored tip");
            return;
        }
    };
    let mut last_probe = Instant::now();
    loop {
        // Health check of every server (tips, latency) for the status page;
        // failures found by ordinary calls are recorded as they happen.
        if last_probe.elapsed() >= PROBE_EVERY {
            source.pool().probe().await;
            last_probe = Instant::now();
        }
        let pass = async {
            let report = chain::ingest(&mut source, &store, &scanner, &config).await?;
            // Landing digs whose transaction is now stored are de-duplicated:
            // from here on the board counts them.
            let now_secs = unix_now();
            if let Err(e) = state
                .landing
                .drop_mined(now_secs, |txid| store.read(|db| db.has_note_from_tx(txid)))
            {
                tracing::warn!(error = %e, "could not check landing digs against stored notes");
            }
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
            let before = store.read(|db| processor.pending_rounds(db))?;
            let mut events_out = processor.process(&store, &mut fetcher, unix_now()).await?;
            let after = store.read(|db| processor.pending_rounds(db))?;
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
            if let Some(coordinator) = coordinator.as_mut() {
                match coordinator.pay_next(&store, &processor, &config).await {
                    Ok(Some(report)) => events_out.push(Event::Paid {
                        round: report.round,
                        txid: report.txid,
                        signers: report.signers,
                    }),
                    Ok(None) => {}
                    Err(e) => tracing::error!(error = %e, "payout attempt failed"),
                }
                if let Err(e) = coordinator.probe_keyholders(&store).await {
                    tracing::warn!(error = %e, "keyholder health probe failed");
                }
                if let Err(e) = coordinator
                    .record_openings(&store, &mut source, &config)
                    .await
                {
                    tracing::warn!(error = %e, "recording payout openings failed; retrying next pass");
                }
            }
            if report.new_notes > 0 {
                let mut fresh: Vec<_> = store
                    .read(|db| {
                        zeek_engine::campfire::messages(
                            db,
                            &config.params,
                            &state.campfire_filter,
                            None,
                            zeek_engine::campfire::MAX_LIMIT,
                            false,
                        )
                    })?
                    .into_iter()
                    .filter(|m| m.height > last_campfire_height)
                    .collect();
                fresh.reverse();
                if let Some(newest) = fresh.last() {
                    last_campfire_height = newest.height;
                }
                events_out.extend(fresh.into_iter().map(Event::Campfire));
            }
            let board = store.read(|db| processor.board(db))?;
            if last_board.as_ref() != Some(&board) {
                events_out.push(Event::Board(board.clone()));
                last_board = Some(board);
            }
            Ok::<_, anyhow::Error>(events_out)
        };
        // A backstop against a call that never returns. No database
        // connection is held across a network call, so the API keeps
        // answering either way; every step resumes from the database.
        let pass = tokio::time::timeout(PASS_TIMEOUT, pass)
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "engine pass timed out after {PASS_TIMEOUT:?}"
                ))
            });
        match pass {
            Ok(events_out) => {
                for event in events_out {
                    let _ = state.events.send(event);
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "engine pass failed; retrying after the poll interval")
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(poll_secs)) => {}
            _ = tokio::signal::ctrl_c() => break,
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
