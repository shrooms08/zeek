//! Instant dig feedback: digs to the vault seen in the mempool ("landing").
//!
//! A watcher task opens `lightwalletd`'s `GetMempoolStream` with its own
//! client, trial-decrypts every action of each mempool transaction with the
//! vault's external IVK and keeps the outputs whose memo is a dig that would
//! be valid if mined in the next block. They are published as
//! [`LandingDig`]s: the board's `landing` field and the WebSocket event
//! `{"event":"landing","digs":[...]}`.
//!
//! Landing digs are a hint for players, nothing more. They never enter the
//! database, the processor, the prize, depths, stages or settlement: a dig
//! counts only once its transaction is mined and ingested like any other
//! note.
//!
//! # Round assignment
//!
//! A dig counts in the round whose window contains the height it is mined
//! at, and only if its memo names that round (`zeek_core::digs::classify`).
//! A mempool transaction has no height yet, so the watcher assumes the next
//! block: height `chain tip + 1` (tip as reported by its own lightwalletd
//! connection when the stream session opened), round
//! `round_for_height(tip + 1)`. The note is classified with exactly the
//! processor's rules at that height (memo decoding, text memos and Campfire
//! activation, minimum value) and kept only if it would be a valid dig. A
//! dig naming the following round, sent before that round opens, is
//! therefore not shown until the chain reaches it.
//!
//! # Lifecycle
//!
//! `GetMempoolStream` re-sends the whole mempool on each call and closes
//! when a block is mined. Each completed session sweeps the entries it did
//! not see (dropped from the mempool, or mined). Independently, after each
//! engine pass the main loop drops every entry whose transaction is stored
//! in the notes table ([`Landing::drop_mined`]); the txid is remembered
//! until a later session completes so a stale mempool cannot bring it back.
//!
//! # Privacy
//!
//! Neither txids nor addresses are exposed: a landing dig carries an opaque
//! id, its round, tile, value, the SHA-256 of its memo field (the digest
//! `/v1/rounds/{n}/digests` serves) and when the engine first saw it.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tonic::codec::Streaming;
use zcash_client_backend::proto::service::{Empty, RawTransaction};
use zeek_core::digs::{self, Classified, ReceivedNote};
use zeek_core::params::GameParams;
use zeek_core::rounds;
use zeek_vault::lightwalletd::{self, Client};

use crate::chain::branch_for;
use crate::db::Pool;
use crate::decrypt::DigScanner;
use crate::error::EngineError;
use crate::processor::Event;

/// Longest wait for the next mempool transaction before the stream is
/// reopened. The mempool can be legitimately silent until the next block.
pub const MEMPOOL_STALL: Duration = Duration::from_secs(15 * 60);
/// Landing digs older than this are dropped even if no session completes
/// (a testnet transaction expires after 40 blocks, about 50 minutes).
pub const MAX_LANDING_AGE_SECS: u64 = 2 * 60 * 60;
/// First retry delay after an error; doubles up to [`RETRY_MAX`].
pub const RETRY_MIN: Duration = Duration::from_secs(5);
/// Longest retry delay after errors.
pub const RETRY_MAX: Duration = Duration::from_secs(5 * 60);
/// Retry delay when the server does not implement `GetMempoolStream`.
pub const UNSUPPORTED_RETRY: Duration = Duration::from_secs(30 * 60);
/// A session shorter than this is followed by a pause of the same length,
/// so a server that closes the stream at once cannot spin the watcher.
pub const MIN_SESSION: Duration = Duration::from_secs(5);

/// A dig to the vault seen in the mempool, not mined yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandingDig {
    /// Opaque id: hex of the first 8 bytes of
    /// `SHA-256(txid (32 bytes, internal order) || pool (0 Ironwood, 1 Orchard) || action index (u16 BE))`.
    pub id: String,
    /// Round the dig would count in if mined in the next block.
    pub round: u32,
    /// Tile dug, `0..=24`.
    pub tile: u8,
    /// Note value in zatoshis (before the dig fee).
    pub value_zat: u64,
    /// SHA-256 (hex) of the 512-byte memo field, as `/v1/rounds/{n}/digests`.
    pub memo_sha256: String,
    /// Unix seconds the engine first saw it.
    pub first_seen: u64,
}

/// Identifies one output of one transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LandingKey {
    pub txid: [u8; 32],
    pub pool: Pool,
    pub action_index: u16,
}

impl LandingKey {
    /// The public opaque id of this output.
    pub fn id(&self) -> String {
        let pool = match self.pool {
            Pool::Ironwood => 0u8,
            Pool::Orchard => 1u8,
        };
        let mut hasher = Sha256::new();
        hasher.update(self.txid);
        hasher.update([pool]);
        hasher.update(self.action_index.to_be_bytes());
        hex::encode(&hasher.finalize()[..8])
    }
}

/// A landing dig found in one transaction, before it is timestamped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub key: LandingKey,
    pub round: u32,
    pub tile: u8,
    pub value_zat: u64,
    pub memo_sha256: String,
}

/// The digs to the vault in the mempool transaction `raw`, classified as if
/// mined at `tip + 1` (see the module docs). Returns the txid too.
pub fn candidates(
    scanner: &DigScanner,
    params: &GameParams,
    raw: &[u8],
    tip: u32,
) -> Result<([u8; 32], Vec<Candidate>), EngineError> {
    let height = tip
        .checked_add(1)
        .ok_or(EngineError::Overflow("next block height"))?;
    let (txid, notes) = scanner.decrypt_all(raw, branch_for(params.network, height))?;
    let Some(round) = rounds::round_for_height(params, height) else {
        return Ok((txid, Vec::new()));
    };
    let mut out = Vec::new();
    for note in notes {
        let received = ReceivedNote {
            // Only used in reports; never leaves this function.
            txid: String::new(),
            output_index: note.action_index,
            height,
            value: note.value,
            memo: note.memo.to_vec(),
        };
        if let Classified::Valid(dig) = digs::classify(params, round, false, received)? {
            out.push(Candidate {
                key: LandingKey {
                    txid,
                    pool: note.pool,
                    action_index: note.action_index,
                },
                round,
                tile: dig.tile,
                value_zat: note.value,
                memo_sha256: hex::encode(Sha256::digest(note.memo)),
            });
        }
    }
    Ok((txid, out))
}

struct Entry {
    dig: LandingDig,
    /// Last session that saw it.
    session: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<LandingKey, Entry>,
    /// Mined txids, with the session current when they were marked.
    mined: HashMap<[u8; 32], u64>,
    /// Latest session begun.
    session: u64,
    /// What was last published (always the current set).
    published: Vec<LandingDig>,
}

/// The current landing digs, shared by the watcher, the engine loop and the
/// API. Every critical section is short and synchronous.
#[derive(Default)]
pub struct Landing {
    inner: Mutex<Inner>,
    events: Option<broadcast::Sender<Event>>,
}

impl Landing {
    /// An empty set that announces changes on `events`.
    pub fn new(events: broadcast::Sender<Event>) -> Self {
        Landing {
            inner: Mutex::default(),
            events: Some(events),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Every mutation leaves `Inner` consistent between statements, so a
        // panic elsewhere while holding the lock cannot corrupt it.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The current landing digs, oldest first.
    pub fn digs(&self) -> Vec<LandingDig> {
        self.lock().published.clone()
    }

    fn publish(&self, inner: &mut Inner) {
        let mut digs: Vec<LandingDig> = inner.entries.values().map(|e| e.dig.clone()).collect();
        digs.sort_by(|a, b| a.first_seen.cmp(&b.first_seen).then(a.id.cmp(&b.id)));
        if digs != inner.published {
            inner.published = digs.clone();
            if let Some(events) = &self.events {
                // No subscribers is fine.
                let _ = events.send(Event::Landing { digs });
            }
        }
    }

    /// Starts a stream session; returns its number.
    pub fn begin_session(&self) -> u64 {
        let mut inner = self.lock();
        inner.session += 1;
        inner.session
    }

    /// Records the candidates of transaction `txid` seen in `session` at
    /// Unix time `now`. Ignored if the transaction is already mined.
    pub fn observe(&self, session: u64, txid: [u8; 32], candidates: Vec<Candidate>, now: u64) {
        let mut inner = self.lock();
        if inner.mined.contains_key(&txid) {
            return;
        }
        for c in candidates {
            let entry = inner.entries.entry(c.key).or_insert_with(|| Entry {
                dig: LandingDig {
                    id: c.key.id(),
                    round: c.round,
                    tile: c.tile,
                    value_zat: c.value_zat,
                    memo_sha256: c.memo_sha256.clone(),
                    first_seen: now,
                },
                session,
            });
            entry.session = entry.session.max(session);
            entry.dig.round = c.round;
            entry.dig.tile = c.tile;
            entry.dig.value_zat = c.value_zat;
            entry.dig.memo_sha256 = c.memo_sha256;
        }
        self.publish(&mut inner);
    }

    /// Ends a session that ran to completion: drops every entry it did not
    /// see, entries older than [`MAX_LANDING_AGE_SECS`], and mined marks
    /// made before it began.
    pub fn end_session(&self, session: u64, now: u64) {
        let mut inner = self.lock();
        inner.entries.retain(|_, e| {
            e.session >= session && now.saturating_sub(e.dig.first_seen) <= MAX_LANDING_AGE_SECS
        });
        inner.mined.retain(|_, marked| *marked >= session);
        self.publish(&mut inner);
    }

    /// Drops the entries whose transaction `is_mined` says is stored, and
    /// remembers those txids. Call after each engine pass, with the
    /// database lock held (one indexed lookup per pending transaction).
    /// Also drops entries older than [`MAX_LANDING_AGE_SECS`].
    pub fn drop_mined(
        &self,
        now: u64,
        mut is_mined: impl FnMut(&[u8; 32]) -> Result<bool, EngineError>,
    ) -> Result<usize, EngineError> {
        let mut pending: Vec<[u8; 32]> = self.lock().entries.keys().map(|k| k.txid).collect();
        pending.sort_unstable();
        pending.dedup();
        let mut mined = Vec::new();
        for txid in pending {
            if is_mined(&txid)? {
                mined.push(txid);
            }
        }
        let mut inner = self.lock();
        let before = inner.entries.len();
        let session = inner.session;
        for txid in &mined {
            inner.mined.insert(*txid, session);
        }
        inner.entries.retain(|k, e| {
            !mined.contains(&k.txid) && now.saturating_sub(e.dig.first_seen) <= MAX_LANDING_AGE_SECS
        });
        let dropped = before - inner.entries.len();
        self.publish(&mut inner);
        Ok(dropped)
    }
}

/// Where mempool transactions come from.
#[async_trait]
pub trait MempoolSource: Send {
    /// Current chain tip height.
    async fn latest_height(&mut self) -> Result<u32, EngineError>;
    /// Opens a new session (dropping any open one): the server re-sends the
    /// whole mempool, then each new transaction.
    async fn open(&mut self) -> Result<(), EngineError>;
    /// The next raw transaction of the open session; `None` once the session
    /// is over (a block was mined) or none is open.
    async fn next(&mut self) -> Result<Option<Vec<u8>>, EngineError>;
}

/// `lightwalletd` as a [`MempoolSource`], with its own connection.
pub struct LightwalletdMempool {
    client: Client,
    stream: Option<Streaming<RawTransaction>>,
}

impl LightwalletdMempool {
    /// Connects to a testnet `lightwalletd`.
    pub async fn connect(url: &str) -> Result<Self, EngineError> {
        Ok(LightwalletdMempool {
            client: lightwalletd::connect(url).await?,
            stream: None,
        })
    }
}

#[async_trait]
impl MempoolSource for LightwalletdMempool {
    async fn latest_height(&mut self) -> Result<u32, EngineError> {
        Ok(lightwalletd::latest_height(&mut self.client).await?.into())
    }

    async fn open(&mut self) -> Result<(), EngineError> {
        self.stream = None;
        let stream = self
            .client
            .get_mempool_stream(Empty {})
            .await
            .map_err(zeek_vault::VaultError::from)?
            .into_inner();
        self.stream = Some(stream);
        Ok(())
    }

    async fn next(&mut self) -> Result<Option<Vec<u8>>, EngineError> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(None);
        };
        match stream.message().await {
            Ok(Some(tx)) => Ok(Some(tx.data)),
            Ok(None) => {
                self.stream = None;
                Ok(None)
            }
            Err(status) => {
                self.stream = None;
                Err(zeek_vault::VaultError::from(status).into())
            }
        }
    }
}

/// How a session ended without error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    /// The server closed the stream (a block was mined); entries the
    /// session did not see were swept.
    Closed { transactions: usize },
    /// No transaction for the stall timeout; nothing was swept.
    Stalled { transactions: usize },
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Runs one stream session: reads the tip, opens the stream and records
/// every landing dig until the stream closes, stalls for `stall`, or fails.
pub async fn run_session(
    source: &mut dyn MempoolSource,
    scanner: &DigScanner,
    params: &GameParams,
    landing: &Landing,
    stall: Duration,
) -> Result<SessionEnd, EngineError> {
    let tip = source.latest_height().await?;
    source.open().await?;
    let session = landing.begin_session();
    let mut transactions = 0usize;
    loop {
        let raw = match tokio::time::timeout(stall, source.next()).await {
            Err(_) => return Ok(SessionEnd::Stalled { transactions }),
            Ok(result) => result?,
        };
        let Some(raw) = raw else {
            landing.end_session(session, unix_now());
            return Ok(SessionEnd::Closed { transactions });
        };
        transactions += 1;
        match candidates(scanner, params, &raw, tip) {
            Ok((txid, found)) if !found.is_empty() => {
                tracing::info!(digs = found.len(), "landing dig seen in the mempool");
                landing.observe(session, txid, found, unix_now());
            }
            Ok(_) => {}
            // Not a transaction this build parses (or a malformed one): it
            // cannot be a dig the engine would count either.
            Err(e) => tracing::debug!(error = %e, "skipping unparsable mempool transaction"),
        }
    }
}

/// Whether `e` says the server does not implement the RPC.
pub fn is_unsupported(e: &EngineError) -> bool {
    matches!(
        e,
        EngineError::Lightwalletd(zeek_vault::VaultError::Rpc(status))
            if status.code() == tonic::Code::Unimplemented
    )
}

/// Watches the mempool forever: connects with `connect`, runs sessions back
/// to back, and reconnects with backoff after errors. Never returns and
/// never panics; it holds no engine lock.
pub async fn watch<C, F, S>(
    mut connect: C,
    scanner: DigScanner,
    params: GameParams,
    landing: Arc<Landing>,
) where
    C: FnMut() -> F + Send,
    F: Future<Output = Result<S, EngineError>> + Send,
    S: MempoolSource,
{
    let mut backoff = RETRY_MIN;
    let mut warned_unsupported = false;
    loop {
        let mut source = match connect().await {
            Ok(source) => source,
            Err(e) => {
                tracing::warn!(error = %e, retry_in = ?backoff, "mempool watcher: connect failed");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RETRY_MAX);
                continue;
            }
        };
        loop {
            let started = Instant::now();
            match run_session(&mut source, &scanner, &params, &landing, MEMPOOL_STALL).await {
                Ok(end) => {
                    backoff = RETRY_MIN;
                    if warned_unsupported {
                        tracing::info!("mempool watcher: GetMempoolStream is available now");
                        warned_unsupported = false;
                    }
                    tracing::debug!(?end, "mempool session ended");
                    let elapsed = started.elapsed();
                    if elapsed < MIN_SESSION {
                        tokio::time::sleep(MIN_SESSION - elapsed).await;
                    }
                }
                Err(e) if is_unsupported(&e) => {
                    if !warned_unsupported {
                        tracing::warn!(
                            retry_in = ?UNSUPPORTED_RETRY,
                            "mempool watcher: lightwalletd does not support GetMempoolStream; landing digs are off until it does"
                        );
                        warned_unsupported = true;
                    }
                    tokio::time::sleep(UNSUPPORTED_RETRY).await;
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, retry_in = ?backoff, "mempool watcher: session failed; reconnecting");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RETRY_MAX);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::chain::ingest;
    use crate::processor::Processor;
    use crate::store::Store;
    use crate::synthetic::{SyntheticChain, TestVault};

    const GENESIS: u32 = 1_000;

    /// Sessions of raw transactions, served in order; `None` in a session
    /// list means "stall here" (the stream stays open with nothing to say).
    #[derive(Default)]
    struct ScriptedMempool {
        tip: u32,
        sessions: VecDeque<Vec<Option<Vec<u8>>>>,
        current: VecDeque<Option<Vec<u8>>>,
        unsupported: bool,
    }

    #[async_trait]
    impl MempoolSource for ScriptedMempool {
        async fn latest_height(&mut self) -> Result<u32, EngineError> {
            Ok(self.tip)
        }

        async fn open(&mut self) -> Result<(), EngineError> {
            if self.unsupported {
                return Err(zeek_vault::VaultError::Rpc(tonic::Status::unimplemented(
                    "unknown method GetMempoolStream",
                ))
                .into());
            }
            self.current = self.sessions.pop_front().unwrap_or_default().into();
            Ok(())
        }

        async fn next(&mut self) -> Result<Option<Vec<u8>>, EngineError> {
            match self.current.pop_front() {
                Some(Some(raw)) => Ok(Some(raw)),
                Some(None) => std::future::pending().await,
                None => Ok(None),
            }
        }
    }

    struct Fixture {
        vault: TestVault,
        chain: SyntheticChain,
        store: Store,
        scanner: DigScanner,
        processor: Processor,
        landing: Landing,
        events: broadcast::Receiver<Event>,
    }

    /// A chain whose stored tip is two blocks into round 0.
    async fn fixture() -> Fixture {
        let vault = TestVault::new(55);
        let config = vault.config(GENESIS);
        let scanner = DigScanner::new(config.incoming_viewing_key().expect("ivk"));
        let mut chain = SyntheticChain::new(&vault, GENESIS, 1_700_000_000);
        chain.advance(2);
        let store = Store::open_in_memory().expect("db");
        ingest(&mut chain, &store, &scanner, &config)
            .await
            .expect("ingest");
        let (tx, events) = broadcast::channel(16);
        Fixture {
            processor: Processor::new(config),
            vault,
            chain,
            store,
            scanner,
            landing: Landing::new(tx),
            events,
        }
    }

    impl Fixture {
        fn tip(&self) -> u32 {
            self.chain.next_height() - 1
        }

        fn params(&self) -> GameParams {
            self.processor.config().params.clone()
        }

        async fn session(&self, txs: Vec<Vec<u8>>) -> SessionEnd {
            let mut source = ScriptedMempool {
                tip: self.tip(),
                sessions: VecDeque::from([txs.into_iter().map(Some).collect()]),
                ..ScriptedMempool::default()
            };
            run_session(
                &mut source,
                &self.scanner,
                &self.params(),
                &self.landing,
                MEMPOOL_STALL,
            )
            .await
            .expect("session")
        }

        fn last_landing_event(&mut self) -> Option<Vec<LandingDig>> {
            let mut last = None;
            while let Ok(event) = self.events.try_recv() {
                if let Event::Landing { digs } = event {
                    last = Some(digs);
                }
            }
            last
        }
    }

    #[tokio::test]
    async fn a_mempool_dig_lands_with_its_round_tile_value_and_digest() {
        let mut f = fixture().await;
        let memo = f.vault.dig_memo(0, 7, 1).encode();
        let dig = f
            .chain
            .unmined_transaction(&f.chain.payment(250_000, Pool::Ironwood, memo));
        let other = f.chain.unmined_transaction(&f.chain.payment_to(
            TestVault::new(99).address(),
            300_000,
            Pool::Ironwood,
            f.vault.dig_memo(0, 2, 2).encode(),
        ));
        let end = f.session(vec![other, dig]).await;
        assert_eq!(end, SessionEnd::Closed { transactions: 2 });

        let digs = f.landing.digs();
        assert_eq!(
            digs.len(),
            1,
            "the transaction to another address is ignored"
        );
        let d = &digs[0];
        assert_eq!((d.round, d.tile, d.value_zat), (0, 7, 250_000));
        assert_eq!(d.memo_sha256, hex::encode(Sha256::digest(memo)));
        assert_eq!(d.id.len(), 16);
        assert!(d.first_seen > 0);
        assert_eq!(f.last_landing_event(), Some(digs.clone()));

        // The serialized form names exactly the public fields.
        let json = serde_json::to_value(Event::Landing { digs }).expect("json");
        assert_eq!(json["event"], "landing");
        let keys: Vec<&String> = json["digs"][0]
            .as_object()
            .expect("object")
            .keys()
            .collect();
        assert_eq!(
            keys,
            [
                "first_seen",
                "id",
                "memo_sha256",
                "round",
                "tile",
                "value_zat"
            ]
        );
    }

    #[tokio::test]
    async fn non_digs_and_wrong_round_digs_do_not_land() {
        let mut f = fixture().await;
        let shout_like =
            f.chain
                .unmined_transaction(&f.chain.payment(250_000, Pool::Ironwood, [0xF6; 512]));
        let wrong_round = f.chain.unmined_transaction(&f.chain.payment(
            250_000,
            Pool::Ironwood,
            f.vault.dig_memo(3, 1, 1).encode(),
        ));
        let dust = f.chain.unmined_transaction(&f.chain.payment(
            1_000,
            Pool::Ironwood,
            f.vault.dig_memo(0, 1, 1).encode(),
        ));
        let change = f.chain.unmined_transaction(&f.chain.change(500_000));
        f.session(vec![shout_like, wrong_round, dust, change, vec![1, 2, 3]])
            .await;
        assert!(f.landing.digs().is_empty());
    }

    #[tokio::test]
    async fn a_mined_dig_leaves_landing_and_counts_once() {
        let mut f = fixture().await;
        let raw = f.chain.unmined_transaction(&f.chain.payment(
            400_000,
            Pool::Ironwood,
            f.vault.dig_memo(0, 12, 3).encode(),
        ));
        f.session(vec![raw.clone()]).await;
        assert_eq!(f.landing.digs().len(), 1);
        let board = f
            .processor
            .board(&f.store.reader().expect("reader"))
            .expect("board");
        assert_eq!(
            (board.digs, board.gross_dug_zat, board.prize_pool_zat),
            (0, 0, 0),
            "a landing dig is never on the board"
        );
        assert_eq!(board.depths, [0; 25]);

        // Mined in the next block and ingested.
        f.chain.push_raw_transaction(raw.clone()).expect("mine");
        let config = f.processor.config().clone();
        ingest(&mut f.chain, &f.store, &f.scanner, &config)
            .await
            .expect("ingest");
        let store = &f.store;
        let dropped = f
            .landing
            .drop_mined(unix_now(), |txid| {
                store.read(|db| db.has_note_from_tx(txid))
            })
            .expect("drop");
        assert_eq!(dropped, 1);
        assert!(f.landing.digs().is_empty());
        assert_eq!(f.last_landing_event(), Some(vec![]));
        let board = f
            .processor
            .board(&f.store.reader().expect("reader"))
            .expect("board");
        assert_eq!(board.digs, 1);
        assert_eq!(board.gross_dug_zat, 400_000);
        assert_eq!(board.depths[12], 100);

        // A stale mempool that still lists it cannot bring it back.
        let session = f.landing.begin_session();
        let (txid, found) = candidates(&f.scanner, &f.params(), &raw, f.tip()).expect("candidates");
        assert_eq!(found.len(), 1);
        f.landing.observe(session, txid, found, unix_now());
        assert!(f.landing.digs().is_empty());
        assert_eq!(
            f.processor
                .board(&f.store.reader().expect("reader"))
                .expect("board")
                .digs,
            1
        );
    }

    #[tokio::test]
    async fn an_evicted_dig_disappears_after_the_next_session() {
        let mut f = fixture().await;
        let raw = f.chain.unmined_transaction(&f.chain.payment(
            300_000,
            Pool::Ironwood,
            f.vault.dig_memo(0, 4, 4).encode(),
        ));
        f.session(vec![raw.clone()]).await;
        let first = f.landing.digs();
        assert_eq!(first.len(), 1);

        // Re-dumped in the next session: kept, first_seen unchanged, no event.
        f.last_landing_event();
        f.session(vec![raw]).await;
        assert_eq!(f.landing.digs(), first);
        assert_eq!(f.last_landing_event(), None);

        // Absent from the next completed session: evicted.
        f.session(vec![]).await;
        assert!(f.landing.digs().is_empty());
        assert_eq!(f.last_landing_event(), Some(vec![]));
    }

    #[tokio::test]
    async fn a_stalled_session_sweeps_nothing() {
        let mut f = fixture().await;
        let raw = f.chain.unmined_transaction(&f.chain.payment(
            300_000,
            Pool::Ironwood,
            f.vault.dig_memo(0, 4, 4).encode(),
        ));
        f.session(vec![raw]).await;
        let mut source = ScriptedMempool {
            tip: f.tip(),
            sessions: VecDeque::from([vec![None]]),
            ..ScriptedMempool::default()
        };
        let end = run_session(
            &mut source,
            &f.scanner,
            &f.params(),
            &f.landing,
            Duration::from_millis(50),
        )
        .await
        .expect("session");
        assert_eq!(end, SessionEnd::Stalled { transactions: 0 });
        assert_eq!(
            f.landing.digs().len(),
            1,
            "an incomplete session evicts nothing"
        );
    }

    #[tokio::test]
    async fn an_unsupported_server_is_recognised() {
        let f = fixture().await;
        let mut source = ScriptedMempool {
            unsupported: true,
            ..ScriptedMempool::default()
        };
        let err = run_session(
            &mut source,
            &f.scanner,
            &f.params(),
            &f.landing,
            MEMPOOL_STALL,
        )
        .await
        .expect_err("unsupported");
        assert!(is_unsupported(&err));
    }

    #[tokio::test]
    async fn landing_digs_never_reach_settlement() {
        use crate::beacon::FixedBeacons;
        use crate::config::SETTLE_CONFIRMATIONS;

        let mut f = fixture().await;
        let params = f.params();
        // One mined dig, then a mempool dig that is never mined, through to
        // the round's settlement (voided: no beacon fixture is needed).
        let mined = f.chain.unmined_transaction(&f.chain.payment(
            200_000,
            Pool::Ironwood,
            f.vault.dig_memo(0, 5, 5).encode(),
        ));
        f.chain.push_raw_transaction(mined).expect("mine");
        let pending = f.chain.unmined_transaction(&f.chain.payment(
            900_000,
            Pool::Ironwood,
            f.vault.dig_memo(0, 5, 6).encode(),
        ));
        f.session(vec![pending]).await;
        assert_eq!(f.landing.digs().len(), 1);
        while f.chain.next_height() < GENESIS + params.blocks_per_round {
            f.chain.advance(1);
        }
        f.chain.delay(100_000); // close far too late: the round is voided
        f.chain.advance(params.mtp_window + SETTLE_CONFIRMATIONS);
        let config = f.processor.config().clone();
        ingest(&mut f.chain, &f.store, &f.scanner, &config)
            .await
            .expect("ingest");
        f.processor
            .process(&f.store, &mut FixedBeacons::default(), u64::MAX / 2)
            .await
            .expect("process");
        let row = f
            .store
            .read(|db| db.settlement(0))
            .expect("row")
            .expect("settled");
        assert_eq!(row.outcome, zeek_core::settle::RoundOutcome::Voided);
        let classified = f
            .processor
            .classified(&f.store.reader().expect("reader"), 0, true)
            .expect("classified");
        assert!(classified.valid.is_empty());
        let refunded: Vec<u64> = classified.refunds.iter().map(|d| d.note.value).collect();
        assert_eq!(
            refunded,
            vec![200_000],
            "only the mined dig is in the round"
        );
        let gross: Vec<u64> = row.settlement.payouts.iter().map(|p| p.gross).collect();
        assert_eq!(gross.len(), 1, "one refund: the mined dig");
        assert!(
            gross[0] <= 200_000,
            "the landing dig is never paid or refunded"
        );
        assert_eq!(f.landing.digs().len(), 1, "still landing, never settled");
    }
}
