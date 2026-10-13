//! `zeek-engine` binary: follows testnet, settles rounds, serves the API.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use tokio::sync::{broadcast, Mutex};
use zeek_engine::api::{router, AppState};
use zeek_engine::beacon::RelayFetcher;
use zeek_engine::chain::{self, LightwalletdSource};
use zeek_engine::config::{EngineSettings, GameConfig};
use zeek_engine::coordinator::{Coordinator, CoordinatorSettings};
use zeek_engine::db::Db;
use zeek_engine::decrypt::DigScanner;
use zeek_engine::processor::{Event, Processor};

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
    let settings = EngineSettings::from_env()?;
    let config = GameConfig::load(&settings.game_config_path)
        .with_context(|| format!("loading {}", settings.game_config_path.display()))?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?
        .block_on(run(settings, config))
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
