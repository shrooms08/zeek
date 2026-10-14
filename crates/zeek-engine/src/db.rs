//! SQLite persistence.
//!
//! Everything the engine knows is derived from the chain and the beacons, so
//! the database is a cache that any restart can rebuild to an identical state;
//! the tables mirror that derivation order (blocks → notes → rounds →
//! settlements). Sealed receivers are stored as the sealed bytes inside the
//! serialized settlement; nothing opened is ever written.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use zeek_core::beacon::{Beacon, Randomness};
use zeek_core::digs::ReceivedNote;
use zeek_core::settle::{RoundOutcome, Settlement};
use zeek_core::timing::TimingVerdict;

use crate::error::EngineError;

/// Schema version written to `meta`. Version 2 added `notes.cmx`.
pub const SCHEMA_VERSION: u32 = 2;

/// A stored block header summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRow {
    pub height: u32,
    /// Block hash, internal byte order.
    pub hash: Vec<u8>,
    /// Previous block hash, internal byte order.
    pub prev_hash: Vec<u8>,
    /// Block `nTime`.
    pub time: u64,
}

/// Pool a note was received in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pool {
    Ironwood,
    Orchard,
}

impl Pool {
    fn as_str(self) -> &'static str {
        match self {
            Pool::Ironwood => "ironwood",
            Pool::Orchard => "orchard",
        }
    }

    fn parse(s: &str) -> Result<Self, EngineError> {
        match s {
            "ironwood" => Ok(Pool::Ironwood),
            "orchard" => Ok(Pool::Orchard),
            other => Err(EngineError::Corrupt(format!("unknown pool `{other}`"))),
        }
    }
}

/// A note received at the vault's external address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRow {
    /// Transaction id, internal byte order.
    pub txid: [u8; 32],
    pub action_index: u16,
    pub pool: Pool,
    pub height: u32,
    pub block_time: u64,
    pub value: u64,
    /// The full 512-byte memo field.
    pub memo: Vec<u8>,
    /// Extracted note commitment `cmx` of the output, the public handle a
    /// payout transaction's spend is matched against. `None` only for rows
    /// written before schema version 2 and not yet backfilled.
    pub cmx: Option<[u8; 32]>,
}

impl NoteRow {
    /// Display-order hex txid.
    pub fn txid_hex(&self) -> String {
        let mut reversed = self.txid;
        reversed.reverse();
        hex::encode(reversed)
    }

    /// The zeek-core view of this note.
    pub fn to_received_note(&self) -> ReceivedNote {
        ReceivedNote {
            txid: self.txid_hex(),
            output_index: self.action_index,
            height: self.height,
            value: self.value,
            memo: self.memo.clone(),
        }
    }
}

/// Stored, verified beacon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeaconRow {
    pub beacon: Beacon,
    pub randomness: Randomness,
    pub relay: String,
}

/// Lifecycle of a persisted settlement plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Computed and waiting for the keyholders to sign the payouts.
    AwaitingSignatures,
    /// The payout transaction was broadcast (see the payout record).
    Paid,
    /// The settlement has no payout to make (everything stayed in the vault).
    NothingToPay,
}

impl PlanStatus {
    fn as_str(self) -> &'static str {
        match self {
            PlanStatus::AwaitingSignatures => "awaiting_signatures",
            PlanStatus::Paid => "paid",
            PlanStatus::NothingToPay => "nothing_to_pay",
        }
    }

    fn parse(s: &str) -> Result<Self, EngineError> {
        match s {
            "awaiting_signatures" => Ok(PlanStatus::AwaitingSignatures),
            "paid" => Ok(PlanStatus::Paid),
            "nothing_to_pay" => Ok(PlanStatus::NothingToPay),
            other => Err(EngineError::Corrupt(format!(
                "unknown plan status `{other}`"
            ))),
        }
    }
}

pub use zeek_core::proof::{PayoutRecord, PayoutState};

/// How a keyholder answered a signing request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyholderOutcome {
    Approved,
    Refused,
    /// No valid answer (unreachable, timed out, or a malformed reply).
    Unreachable,
}

impl KeyholderOutcome {
    fn as_str(self) -> &'static str {
        match self {
            KeyholderOutcome::Approved => "approved",
            KeyholderOutcome::Refused => "refused",
            KeyholderOutcome::Unreachable => "unreachable",
        }
    }

    fn parse(s: &str) -> Result<Self, EngineError> {
        match s {
            "approved" => Ok(KeyholderOutcome::Approved),
            "refused" => Ok(KeyholderOutcome::Refused),
            "unreachable" => Ok(KeyholderOutcome::Unreachable),
            other => Err(EngineError::Corrupt(format!(
                "unknown keyholder outcome `{other}`"
            ))),
        }
    }
}

/// One keyholder's answer to one signing request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyholderEvent {
    /// Unix seconds.
    pub time: u64,
    /// Signing session id (hex).
    pub session: String,
    pub keyholder: u16,
    pub rounds: Vec<u32>,
    pub outcome: KeyholderOutcome,
    /// The keyholder's refusal text, or the error; empty when approved.
    pub reason: String,
}

/// The last health probe of a keyholder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyholderHealth {
    pub keyholder: u16,
    pub last_check: u64,
    pub last_seen: Option<u64>,
    pub reachable: bool,
    pub error: String,
}

fn payout_state_str(state: PayoutState) -> &'static str {
    match state {
        PayoutState::Signing => "signing",
        PayoutState::Broadcast => "broadcast",
    }
}

fn parse_payout_state(s: &str) -> Result<PayoutState, EngineError> {
    match s {
        "signing" => Ok(PayoutState::Signing),
        "broadcast" => Ok(PayoutState::Broadcast),
        other => Err(EngineError::Corrupt(format!(
            "unknown payout state `{other}`"
        ))),
    }
}

/// A settled round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRow {
    pub round: u32,
    /// Height of the chain tip when the settlement was computed.
    pub settled_at_height: u32,
    pub start_block_time: u64,
    pub post_close_block_times: Vec<u64>,
    pub drand_round: u64,
    pub beacon_time: u64,
    pub timing: TimingVerdict,
    pub outcome: RoundOutcome,
    pub settlement: Settlement,
    pub status: PlanStatus,
    /// Rules the settlement was computed under (1 or 2); rows written before
    /// rules v2 existed are version 1.
    pub rules_version: u32,
}

/// The engine database.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Opens (or creates) the database file and applies the schema.
    pub fn open(path: &Path) -> Result<Self, EngineError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| EngineError::io(parent, e))?;
            }
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// Opens an in-memory database (tests).
    pub fn open_in_memory() -> Result<Self, EngineError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, EngineError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS meta (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS blocks (
                 height INTEGER PRIMARY KEY,
                 hash BLOB NOT NULL,
                 prev_hash BLOB NOT NULL,
                 time INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS notes (
                 id INTEGER PRIMARY KEY,
                 txid BLOB NOT NULL,
                 action_index INTEGER NOT NULL,
                 pool TEXT NOT NULL,
                 height INTEGER NOT NULL,
                 block_time INTEGER NOT NULL,
                 value INTEGER NOT NULL,
                 memo BLOB NOT NULL,
                 cmx BLOB,
                 UNIQUE (txid, action_index, pool)
             );
             CREATE INDEX IF NOT EXISTS notes_height ON notes (height);
             CREATE TABLE IF NOT EXISTS beacons (
                 drand_round INTEGER PRIMARY KEY,
                 signature TEXT NOT NULL,
                 randomness TEXT NOT NULL,
                 relay TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS payouts (
                 round INTEGER PRIMARY KEY,
                 state TEXT NOT NULL,
                 session TEXT NOT NULL,
                 txid TEXT,
                 height INTEGER NOT NULL,
                 fee INTEGER NOT NULL,
                 signers TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settlements (
                 round INTEGER PRIMARY KEY,
                 settled_at_height INTEGER NOT NULL,
                 start_block_time INTEGER NOT NULL,
                 post_close_block_times TEXT NOT NULL,
                 drand_round INTEGER NOT NULL,
                 beacon_time INTEGER NOT NULL,
                 timing TEXT NOT NULL,
                 outcome TEXT NOT NULL,
                 settlement TEXT NOT NULL,
                 status TEXT NOT NULL,
                 rules_version INTEGER NOT NULL DEFAULT 1
             );
             CREATE TABLE IF NOT EXISTS payout_openings (
                 round INTEGER PRIMARY KEY,
                 openings TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS keyholder_events (
                 id INTEGER PRIMARY KEY,
                 time INTEGER NOT NULL,
                 session TEXT NOT NULL,
                 keyholder INTEGER NOT NULL,
                 rounds TEXT NOT NULL,
                 outcome TEXT NOT NULL,
                 reason TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS keyholder_health (
                 keyholder INTEGER PRIMARY KEY,
                 last_check INTEGER NOT NULL,
                 last_seen INTEGER,
                 reachable INTEGER NOT NULL,
                 error TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS win_proofs (
                 id TEXT PRIMARY KEY,
                 round INTEGER NOT NULL,
                 created_at INTEGER NOT NULL,
                 proof BLOB NOT NULL
             );",
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        let stored: String = conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )?;
        if stored == "1" {
            // Version 1 → 2: note commitments are recorded; existing rows are
            // backfilled from their transactions by the chain follower.
            conn.execute_batch(
                "ALTER TABLE notes ADD COLUMN cmx BLOB;
                 ALTER TABLE settlements ADD COLUMN rules_version INTEGER NOT NULL DEFAULT 1;
                 UPDATE meta SET value = '2' WHERE key = 'schema_version';",
            )?;
            tracing::info!("engine database migrated from schema 1 to 2 (note commitments)");
        } else if stored != SCHEMA_VERSION.to_string() {
            return Err(EngineError::Corrupt(format!(
                "schema version {stored} is not supported (expected {SCHEMA_VERSION})"
            )));
        }
        Ok(Db { conn })
    }

    // ---- payout openings (proof bundle format 4) ----

    /// Stores the openings of a round's mined payout transaction.
    pub fn set_payout_openings(
        &self,
        round: u32,
        openings: &zeek_core::proof::PayoutOpenings,
    ) -> Result<(), EngineError> {
        let json =
            serde_json::to_string(openings).map_err(|e| EngineError::Corrupt(e.to_string()))?;
        self.conn.execute(
            "INSERT OR REPLACE INTO payout_openings (round, openings) VALUES (?1, ?2)",
            params![round, json],
        )?;
        Ok(())
    }

    /// The openings of a round's payout transaction, once recorded.
    pub fn payout_openings(
        &self,
        round: u32,
    ) -> Result<Option<zeek_core::proof::PayoutOpenings>, EngineError> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT openings FROM payout_openings WHERE round = ?1",
                params![round],
                |r| r.get(0),
            )
            .optional()?;
        json.map(|j| serde_json::from_str(&j).map_err(|e| EngineError::Corrupt(e.to_string())))
            .transpose()
    }

    /// Rounds whose payout was broadcast but whose openings are not recorded.
    pub fn rounds_missing_openings(&self) -> Result<Vec<(u32, String)>, EngineError> {
        let mut stmt = self.conn.prepare(
            "SELECT p.round, p.txid FROM payouts p
             LEFT JOIN payout_openings o ON o.round = p.round
             WHERE p.state = 'broadcast' AND p.txid IS NOT NULL AND o.round IS NULL
             ORDER BY p.round",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    // ---- keyholder records (no secrets: outcomes and reasons only) ----

    /// Records one keyholder's answer to a signing request.
    pub fn insert_keyholder_event(&self, event: &KeyholderEvent) -> Result<(), EngineError> {
        self.conn.execute(
            "INSERT INTO keyholder_events (time, session, keyholder, rounds, outcome, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                i64::try_from(event.time).unwrap_or(i64::MAX),
                event.session,
                event.keyholder,
                serde_json::to_string(&event.rounds)
                    .map_err(|e| EngineError::Corrupt(e.to_string()))?,
                event.outcome.as_str(),
                event.reason,
            ],
        )?;
        Ok(())
    }

    /// Keyholder events, newest first.
    pub fn keyholder_events(&self, limit: u32) -> Result<Vec<KeyholderEvent>, EngineError> {
        let mut stmt = self.conn.prepare(
            "SELECT time, session, keyholder, rounds, outcome, reason FROM keyholder_events
             ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u16>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(time, session, keyholder, rounds, outcome, reason)| {
                Ok(KeyholderEvent {
                    time: u64::try_from(time).unwrap_or(0),
                    session,
                    keyholder,
                    rounds: serde_json::from_str(&rounds)
                        .map_err(|e| EngineError::Corrupt(e.to_string()))?,
                    outcome: KeyholderOutcome::parse(&outcome)?,
                    reason,
                })
            })
            .collect()
    }

    /// Per keyholder: (approved, refused) counts over all recorded requests.
    pub fn keyholder_counts(&self) -> Result<Vec<(u16, u64, u64)>, EngineError> {
        let mut stmt = self.conn.prepare(
            "SELECT keyholder,
                    SUM(CASE WHEN outcome = 'approved' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN outcome = 'refused' THEN 1 ELSE 0 END)
             FROM keyholder_events GROUP BY keyholder ORDER BY keyholder",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, u16>(0)?,
                    u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                    u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Records a health probe of a keyholder.
    pub fn set_keyholder_health(
        &self,
        keyholder: u16,
        now: u64,
        reachable: bool,
        error: &str,
    ) -> Result<(), EngineError> {
        let now = i64::try_from(now).unwrap_or(i64::MAX);
        self.conn.execute(
            "INSERT INTO keyholder_health (keyholder, last_check, last_seen, reachable, error)
             VALUES (?1, ?2, CASE WHEN ?3 THEN ?2 ELSE NULL END, ?3, ?4)
             ON CONFLICT(keyholder) DO UPDATE SET
                 last_check = excluded.last_check,
                 last_seen = CASE WHEN excluded.reachable THEN excluded.last_check ELSE keyholder_health.last_seen END,
                 reachable = excluded.reachable,
                 error = excluded.error",
            params![keyholder, now, reachable, error],
        )?;
        Ok(())
    }

    /// Health of every probed keyholder: (id, last_check, last_seen, reachable, error).
    pub fn keyholder_health(&self) -> Result<Vec<KeyholderHealth>, EngineError> {
        let mut stmt = self.conn.prepare(
            "SELECT keyholder, last_check, last_seen, reachable, error FROM keyholder_health ORDER BY keyholder",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(KeyholderHealth {
                    keyholder: r.get(0)?,
                    last_check: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                    last_seen: r
                        .get::<_, Option<i64>>(2)?
                        .map(|t| u64::try_from(t).unwrap_or(0)),
                    reachable: r.get(3)?,
                    error: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    // ---- win proofs ----

    /// Stores a validated win proof under `id` (idempotent).
    pub fn insert_win_proof(
        &self,
        id: &str,
        round: u32,
        created_at: u64,
        proof: &[u8],
    ) -> Result<(), EngineError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO win_proofs (id, round, created_at, proof) VALUES (?1, ?2, ?3, ?4)",
            params![id, round, i64::try_from(created_at).unwrap_or(i64::MAX), proof],
        )?;
        Ok(())
    }

    /// A stored win proof.
    pub fn win_proof(&self, id: &str) -> Result<Option<(u32, u64, Vec<u8>)>, EngineError> {
        Ok(self
            .conn
            .query_row(
                "SELECT round, created_at, proof FROM win_proofs WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?)
    }

    // ---- meta ----

    /// Stores a key/value pair.
    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), EngineError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Reads a key/value pair.
    pub fn meta(&self, key: &str) -> Result<Option<String>, EngineError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// The chain tip last reported by the block source (which may be below
    /// the game's genesis, when no block is stored yet).
    pub fn chain_tip(&self) -> Result<Option<u32>, EngineError> {
        self.meta("chain_tip")?
            .map(|v| {
                v.parse::<u32>()
                    .map_err(|_| EngineError::Corrupt("chain_tip is not a number".into()))
            })
            .transpose()
    }

    /// Records the chain tip reported by the block source.
    pub fn set_chain_tip(&self, height: u32) -> Result<(), EngineError> {
        self.set_meta("chain_tip", &height.to_string())
    }

    /// `nTime` of the chain tip block, recorded while the tip is below genesis.
    pub fn chain_tip_time(&self) -> Result<Option<u64>, EngineError> {
        self.meta("chain_tip_time")?
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|_| EngineError::Corrupt("chain_tip_time is not a number".into()))
            })
            .transpose()
    }

    /// Records the chain tip's `nTime`.
    pub fn set_chain_tip_time(&self, time: u64) -> Result<(), EngineError> {
        self.set_meta("chain_tip_time", &time.to_string())
    }

    // ---- blocks ----

    /// Stores a block (replacing any row at the same height).
    pub fn insert_block(&self, block: &BlockRow) -> Result<(), EngineError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO blocks (height, hash, prev_hash, time) VALUES (?1, ?2, ?3, ?4)",
            params![block.height, block.hash, block.prev_hash, block.time],
        )?;
        Ok(())
    }

    /// The stored block at `height`.
    pub fn block(&self, height: u32) -> Result<Option<BlockRow>, EngineError> {
        Ok(self
            .conn
            .query_row(
                "SELECT height, hash, prev_hash, time FROM blocks WHERE height = ?1",
                params![height],
                |r| {
                    Ok(BlockRow {
                        height: r.get(0)?,
                        hash: r.get(1)?,
                        prev_hash: r.get(2)?,
                        time: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// The highest stored block.
    pub fn tip(&self) -> Result<Option<BlockRow>, EngineError> {
        Ok(self
            .conn
            .query_row(
                "SELECT height, hash, prev_hash, time FROM blocks ORDER BY height DESC LIMIT 1",
                [],
                |r| {
                    Ok(BlockRow {
                        height: r.get(0)?,
                        hash: r.get(1)?,
                        prev_hash: r.get(2)?,
                        time: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Timestamps of the blocks `start..end` (exclusive), in height order,
    /// stopping at the first missing block.
    pub fn block_times(&self, start: u32, end: u32) -> Result<Vec<u64>, EngineError> {
        let mut times = Vec::new();
        for height in start..end {
            match self.block(height)? {
                Some(b) => times.push(b.time),
                None => break,
            }
        }
        Ok(times)
    }

    // ---- notes ----

    /// Stores a note; a note already present (same txid, action, pool) is kept.
    pub fn insert_note(&self, note: &NoteRow) -> Result<bool, EngineError> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO notes (txid, action_index, pool, height, block_time, value, memo, cmx)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                note.txid.as_slice(),
                note.action_index,
                note.pool.as_str(),
                note.height,
                note.block_time,
                note.value,
                note.memo,
                note.cmx.map(|c| c.to_vec())
            ],
        )?;
        Ok(inserted == 1)
    }

    const NOTE_COLUMNS: &'static str =
        "txid, action_index, pool, height, block_time, value, memo, cmx";

    fn query_notes(
        &self,
        where_clause: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<NoteRow>, EngineError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM notes WHERE {where_clause} ORDER BY height, id",
            Self::NOTE_COLUMNS
        ))?;
        let rows = stmt.query_map(args, |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, u16>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, Vec<u8>>(6)?,
                r.get::<_, Option<Vec<u8>>>(7)?,
            ))
        })?;
        let mut notes = Vec::new();
        for row in rows {
            let (txid, action_index, pool, height, block_time, value, memo, cmx) = row?;
            let txid = <[u8; 32]>::try_from(txid.as_slice())
                .map_err(|_| EngineError::Corrupt("txid is not 32 bytes".into()))?;
            let cmx = cmx
                .map(|c| {
                    <[u8; 32]>::try_from(c.as_slice())
                        .map_err(|_| EngineError::Corrupt("cmx is not 32 bytes".into()))
                })
                .transpose()?;
            notes.push(NoteRow {
                txid,
                action_index,
                pool: Pool::parse(&pool)?,
                height,
                block_time,
                value,
                memo,
                cmx,
            });
        }
        Ok(notes)
    }

    /// Notes mined in heights `start..end` (exclusive), in chain order.
    pub fn notes_in_range(&self, start: u32, end: u32) -> Result<Vec<NoteRow>, EngineError> {
        self.query_notes("height >= ?1 AND height < ?2", &[&start, &end])
    }

    /// Notes stored before schema version 2, whose commitment is not recorded yet.
    pub fn notes_without_cmx(&self) -> Result<Vec<NoteRow>, EngineError> {
        self.query_notes("cmx IS NULL", &[])
    }

    /// The note with commitment `cmx`, if the vault received it.
    pub fn note_by_cmx(&self, cmx: &[u8; 32]) -> Result<Option<NoteRow>, EngineError> {
        Ok(self.query_notes("cmx = ?1", &[&cmx.as_slice()])?.pop())
    }

    /// Forgets every recorded commitment (simulates a schema-1 database).
    #[cfg(test)]
    pub(crate) fn clear_note_commitments(&self) -> Result<(), EngineError> {
        self.conn.execute("UPDATE notes SET cmx = NULL", [])?;
        Ok(())
    }

    /// Records the commitment of a note stored before schema version 2.
    pub fn set_note_cmx(
        &self,
        txid: &[u8; 32],
        action_index: u16,
        pool: Pool,
        cmx: &[u8; 32],
    ) -> Result<(), EngineError> {
        self.conn.execute(
            "UPDATE notes SET cmx = ?1 WHERE txid = ?2 AND action_index = ?3 AND pool = ?4",
            params![cmx.as_slice(), txid.as_slice(), action_index, pool.as_str()],
        )?;
        Ok(())
    }

    /// Number of stored notes.
    pub fn note_count(&self) -> Result<u64, EngineError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))?)
    }

    // ---- beacons ----

    /// Stores a verified beacon.
    pub fn insert_beacon(&self, row: &BeaconRow) -> Result<(), EngineError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO beacons (drand_round, signature, randomness, relay)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                row.beacon.round,
                row.beacon.signature,
                hex::encode(row.randomness.0),
                row.relay
            ],
        )?;
        Ok(())
    }

    /// A stored beacon.
    pub fn beacon(&self, drand_round: u64) -> Result<Option<BeaconRow>, EngineError> {
        let row = self
            .conn
            .query_row(
                "SELECT signature, randomness, relay FROM beacons WHERE drand_round = ?1",
                params![drand_round],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(signature, randomness, relay)| {
            let bytes = hex::decode(randomness)
                .map_err(|e| EngineError::Corrupt(format!("randomness hex: {e}")))?;
            let randomness = <[u8; 32]>::try_from(bytes.as_slice())
                .map(Randomness)
                .map_err(|_| EngineError::Corrupt("randomness is not 32 bytes".into()))?;
            Ok(BeaconRow {
                beacon: Beacon {
                    round: drand_round,
                    signature,
                },
                randomness,
                relay,
            })
        })
        .transpose()
    }

    // ---- settlements ----

    /// Stores a settlement plan.
    pub fn insert_settlement(&self, row: &SettlementRow) -> Result<(), EngineError> {
        let json = |v: &dyn erased::Ser| v.to_json();
        self.conn.execute(
            "INSERT INTO settlements (round, settled_at_height, start_block_time,
                 post_close_block_times, drand_round, beacon_time, timing, outcome, settlement, status,
                 rules_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                row.round,
                row.settled_at_height,
                row.start_block_time,
                json(&row.post_close_block_times)?,
                row.drand_round,
                row.beacon_time,
                json(&row.timing)?,
                json(&row.outcome)?,
                json(&row.settlement)?,
                row.status.as_str(),
                row.rules_version
            ],
        )?;
        Ok(())
    }

    /// The settlement of `round`, if any.
    pub fn settlement(&self, round: u32) -> Result<Option<SettlementRow>, EngineError> {
        let row = self
            .conn
            .query_row(
                "SELECT settled_at_height, start_block_time, post_close_block_times, drand_round,
                        beacon_time, timing, outcome, settlement, status, rules_version
                 FROM settlements WHERE round = ?1",
                params![round],
                |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, u64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, u64>(3)?,
                        r.get::<_, u64>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, u32>(9)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(
                settled_at_height,
                start_block_time,
                times,
                drand_round,
                beacon_time,
                timing,
                outcome,
                settlement,
                status,
                rules_version,
            )| {
                Ok(SettlementRow {
                    round,
                    settled_at_height,
                    start_block_time,
                    post_close_block_times: erased::de(&times)?,
                    drand_round,
                    beacon_time,
                    timing: erased::de(&timing)?,
                    outcome: erased::de(&outcome)?,
                    settlement: erased::de(&settlement)?,
                    status: PlanStatus::parse(&status)?,
                    rules_version,
                })
            },
        )
        .transpose()
    }

    /// The highest settled round.
    pub fn last_settled_round(&self) -> Result<Option<u32>, EngineError> {
        Ok(self
            .conn
            .query_row("SELECT MAX(round) FROM settlements", [], |r| {
                r.get::<_, Option<u32>>(0)
            })?)
    }

    /// Settled rounds in order.
    pub fn settled_rounds(&self) -> Result<Vec<u32>, EngineError> {
        let mut stmt = self
            .conn
            .prepare("SELECT round FROM settlements ORDER BY round")?;
        let rows = stmt.query_map([], |r| r.get::<_, u32>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Updates a settlement's plan status.
    pub fn set_plan_status(&self, round: u32, status: PlanStatus) -> Result<(), EngineError> {
        self.conn.execute(
            "UPDATE settlements SET status = ?1 WHERE round = ?2",
            params![status.as_str(), round],
        )?;
        Ok(())
    }

    // ---- payouts ----

    /// Records that a signing session was opened for `round`.
    pub fn insert_payout(&self, record: &PayoutRecord) -> Result<(), EngineError> {
        self.conn.execute(
            "INSERT INTO payouts (round, state, session, txid, height, fee, signers)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.round,
                payout_state_str(record.state),
                record.session,
                record.txid,
                record.height,
                record.fee,
                serde_json::to_string(&record.signers)
                    .map_err(|e| EngineError::Corrupt(e.to_string()))?
            ],
        )?;
        Ok(())
    }

    /// Records which keyholders signed, before the transaction is broadcast.
    pub fn set_payout_signers(&self, round: u32, signers: &[u16]) -> Result<(), EngineError> {
        self.conn.execute(
            "UPDATE payouts SET signers = ?1 WHERE round = ?2",
            params![
                serde_json::to_string(signers).map_err(|e| EngineError::Corrupt(e.to_string()))?,
                round
            ],
        )?;
        Ok(())
    }

    /// Payout records still in the signing state.
    pub fn payouts_in_signing(&self) -> Result<Vec<PayoutRecord>, EngineError> {
        let mut stmt = self
            .conn
            .prepare("SELECT round FROM payouts WHERE state = 'signing' ORDER BY round")?;
        let rounds: Vec<u32> = stmt
            .query_map([], |r| r.get::<_, u32>(0))?
            .collect::<Result<_, _>>()?;
        let mut out = Vec::with_capacity(rounds.len());
        for round in rounds {
            if let Some(record) = self.payout(round)? {
                out.push(record);
            }
        }
        Ok(out)
    }

    /// Marks the round's payout as broadcast.
    pub fn mark_payout_broadcast(
        &self,
        round: u32,
        txid: &str,
        height: u32,
        signers: &[u16],
    ) -> Result<(), EngineError> {
        self.conn.execute(
            "UPDATE payouts SET state = 'broadcast', txid = ?1, height = ?2, signers = ?3 WHERE round = ?4",
            params![
                txid,
                height,
                serde_json::to_string(signers).map_err(|e| EngineError::Corrupt(e.to_string()))?,
                round
            ],
        )?;
        Ok(())
    }

    /// Removes a payout record whose session aborted before broadcast.
    pub fn delete_payout_if_signing(&self, round: u32) -> Result<(), EngineError> {
        self.conn.execute(
            "DELETE FROM payouts WHERE round = ?1 AND state = 'signing'",
            params![round],
        )?;
        Ok(())
    }

    /// The payout record of `round`.
    pub fn payout(&self, round: u32) -> Result<Option<PayoutRecord>, EngineError> {
        let row = self
            .conn
            .query_row(
                "SELECT state, session, txid, height, fee, signers FROM payouts WHERE round = ?1",
                params![round],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, u32>(3)?,
                        r.get::<_, u64>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(state, session, txid, height, fee, signers)| {
            Ok(PayoutRecord {
                round,
                state: parse_payout_state(&state)?,
                session,
                txid,
                height,
                fee,
                signers: serde_json::from_str::<Vec<u16>>(&signers)
                    .map_err(|e| EngineError::Corrupt(e.to_string()))?,
            })
        })
        .transpose()
    }

    // ---- reorgs ----

    /// Forgets every block and note at or above `height`, plus any settlement
    /// computed from a tip at or above it (which the confirmation policy
    /// should make impossible; it is logged by the caller).
    pub fn rollback_to(&mut self, height: u32) -> Result<Vec<u32>, EngineError> {
        let tx = self.conn.transaction()?;
        let affected: Vec<u32> = {
            let mut stmt = tx.prepare(
                "SELECT round FROM settlements WHERE settled_at_height >= ?1 ORDER BY round",
            )?;
            let rows = stmt.query_map(params![height], |r| r.get::<_, u32>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        tx.execute(
            "DELETE FROM settlements WHERE settled_at_height >= ?1",
            params![height],
        )?;
        tx.execute("DELETE FROM notes WHERE height >= ?1", params![height])?;
        tx.execute("DELETE FROM blocks WHERE height >= ?1", params![height])?;
        tx.commit()?;
        Ok(affected)
    }

    /// A canonical dump of all state, for idempotency checks.
    pub fn dump(&self) -> Result<String, EngineError> {
        let mut out = String::new();
        let mut stmt = self.conn.prepare(
            "SELECT height, hex(hash), hex(prev_hash), time FROM blocks ORDER BY height",
        )?;
        for row in stmt.query_map([], |r| {
            Ok(format!(
                "block {} {} {} {}\n",
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u64>(3)?
            ))
        })? {
            out.push_str(&row?);
        }
        let mut stmt = self.conn.prepare(
            "SELECT hex(txid), action_index, pool, height, block_time, value, hex(memo), hex(coalesce(cmx, x'')) FROM notes
             ORDER BY height, txid, action_index, pool",
        )?;
        for row in stmt.query_map([], |r| {
            Ok(format!(
                "note {} {} {} {} {} {} {} {}\n",
                r.get::<_, String>(0)?,
                r.get::<_, u16>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?
            ))
        })? {
            out.push_str(&row?);
        }
        let mut stmt = self.conn.prepare(
            "SELECT round, state, session, coalesce(txid, ''), height, fee, signers FROM payouts ORDER BY round",
        )?;
        for row in stmt.query_map([], |r| {
            Ok(format!(
                "payout {} {} {} {} {} {} {}\n",
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, String>(6)?
            ))
        })? {
            out.push_str(&row?);
        }
        let mut stmt = self.conn.prepare(
            "SELECT round, settled_at_height, start_block_time, post_close_block_times, drand_round,
                    beacon_time, timing, outcome, settlement, status, rules_version FROM settlements ORDER BY round",
        )?;
        for row in stmt.query_map([], |r| {
            Ok(format!(
                "settlement {} {} {} {} {} {} {} {} {} {} {}\n",
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, u32>(10)?
            ))
        })? {
            out.push_str(&row?);
        }
        Ok(out)
    }
}

/// Small JSON helpers keeping `serde_json` errors inside [`EngineError`].
mod erased {
    use crate::error::EngineError;

    pub trait Ser {
        fn to_json(&self) -> Result<String, EngineError>;
    }

    impl<T: serde::Serialize> Ser for T {
        fn to_json(&self) -> Result<String, EngineError> {
            serde_json::to_string(self).map_err(|e| EngineError::Corrupt(format!("serialize: {e}")))
        }
    }

    pub fn de<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, EngineError> {
        serde_json::from_str(text).map_err(|e| EngineError::Corrupt(format!("deserialize: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(h: u32) -> BlockRow {
        BlockRow {
            height: h,
            hash: vec![h as u8; 32],
            prev_hash: vec![(h - 1) as u8; 32],
            time: 1_000 + u64::from(h),
        }
    }

    #[test]
    fn blocks_notes_and_rollback() {
        let mut db = Db::open_in_memory().expect("db");
        for h in 1..=5 {
            db.insert_block(&block(h)).expect("insert");
        }
        assert_eq!(db.tip().expect("tip").map(|b| b.height), Some(5));
        assert_eq!(
            db.block_times(2, 10).expect("times"),
            vec![1_002, 1_003, 1_004, 1_005]
        );
        let note = NoteRow {
            txid: [7; 32],
            action_index: 1,
            pool: Pool::Ironwood,
            height: 4,
            block_time: 1_004,
            value: 100_000,
            memo: vec![0xFF; 512],
            cmx: None,
        };
        assert!(db.insert_note(&note).expect("insert"));
        assert!(
            !db.insert_note(&note).expect("dup"),
            "duplicate notes are ignored"
        );
        assert_eq!(db.notes_in_range(1, 5).expect("notes"), vec![note.clone()]);
        assert_eq!(note.to_received_note().txid, "07".repeat(32));
        assert_eq!(db.notes_without_cmx().expect("missing").len(), 1);
        db.set_note_cmx(&note.txid, 1, Pool::Ironwood, &[9; 32])
            .expect("backfill");
        assert!(db.notes_without_cmx().expect("missing").is_empty());
        assert_eq!(
            db.note_by_cmx(&[9; 32]).expect("lookup").map(|n| n.value),
            Some(100_000)
        );
        assert!(db.note_by_cmx(&[8; 32]).expect("lookup").is_none());

        let affected = db.rollback_to(4).expect("rollback");
        assert!(affected.is_empty());
        assert_eq!(db.tip().expect("tip").map(|b| b.height), Some(3));
        assert_eq!(db.note_count().expect("count"), 0);
    }

    #[test]
    fn schema_version_1_is_migrated_in_place() {
        let conn = Connection::open_in_memory().expect("conn");
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES ('schema_version', '1');
             CREATE TABLE notes (
                 id INTEGER PRIMARY KEY, txid BLOB NOT NULL, action_index INTEGER NOT NULL,
                 pool TEXT NOT NULL, height INTEGER NOT NULL, block_time INTEGER NOT NULL,
                 value INTEGER NOT NULL, memo BLOB NOT NULL, UNIQUE (txid, action_index, pool));
             CREATE TABLE settlements (
                 round INTEGER PRIMARY KEY, settled_at_height INTEGER NOT NULL,
                 start_block_time INTEGER NOT NULL, post_close_block_times TEXT NOT NULL,
                 drand_round INTEGER NOT NULL, beacon_time INTEGER NOT NULL, timing TEXT NOT NULL,
                 outcome TEXT NOT NULL, settlement TEXT NOT NULL, status TEXT NOT NULL);
             INSERT INTO notes (txid, action_index, pool, height, block_time, value, memo)
                 VALUES (x'00', 0, 'ironwood', 5, 1, 7, x'ff');",
        )
        .expect("v1 schema");
        let db = Db::init(conn).expect("migrate");
        assert_eq!(db.meta("schema_version").expect("meta"), Some("2".into()));
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT count(*) FROM pragma_table_info('settlements') WHERE name = 'rules_version'",
                    [],
                    |r| r.get::<_, u32>(0)
                )
                .expect("column"),
            1,
            "old settlement rows read back as rules version 1"
        );
        // The v1 txid above is 1 byte, so the row is reported as corrupt when
        // read; the migration itself only adds the column.
        assert_eq!(
            db.conn
                .query_row("SELECT count(*) FROM notes WHERE cmx IS NULL", [], |r| r
                    .get::<_, u32>(0))
                .expect("count"),
            1
        );
    }
}
