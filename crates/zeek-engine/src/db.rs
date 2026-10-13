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

/// Schema version written to `meta`.
pub const SCHEMA_VERSION: u32 = 1;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Computed and waiting for the keyholders to sign the payouts (Phase 4).
    AwaitingSignatures,
}

impl PlanStatus {
    fn as_str(self) -> &'static str {
        match self {
            PlanStatus::AwaitingSignatures => "awaiting_signatures",
        }
    }

    fn parse(s: &str) -> Result<Self, EngineError> {
        match s {
            "awaiting_signatures" => Ok(PlanStatus::AwaitingSignatures),
            other => Err(EngineError::Corrupt(format!(
                "unknown plan status `{other}`"
            ))),
        }
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
                 UNIQUE (txid, action_index, pool)
             );
             CREATE INDEX IF NOT EXISTS notes_height ON notes (height);
             CREATE TABLE IF NOT EXISTS beacons (
                 drand_round INTEGER PRIMARY KEY,
                 signature TEXT NOT NULL,
                 randomness TEXT NOT NULL,
                 relay TEXT NOT NULL
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
                 status TEXT NOT NULL
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
        if stored != SCHEMA_VERSION.to_string() {
            return Err(EngineError::Corrupt(format!(
                "schema version {stored} is not supported (expected {SCHEMA_VERSION})"
            )));
        }
        Ok(Db { conn })
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
            "INSERT OR IGNORE INTO notes (txid, action_index, pool, height, block_time, value, memo)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                note.txid.as_slice(),
                note.action_index,
                note.pool.as_str(),
                note.height,
                note.block_time,
                note.value,
                note.memo
            ],
        )?;
        Ok(inserted == 1)
    }

    /// Notes mined in heights `start..end` (exclusive), in chain order.
    pub fn notes_in_range(&self, start: u32, end: u32) -> Result<Vec<NoteRow>, EngineError> {
        let mut stmt = self.conn.prepare(
            "SELECT txid, action_index, pool, height, block_time, value, memo FROM notes
             WHERE height >= ?1 AND height < ?2 ORDER BY height, id",
        )?;
        let rows = stmt.query_map(params![start, end], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, u16>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, Vec<u8>>(6)?,
            ))
        })?;
        let mut notes = Vec::new();
        for row in rows {
            let (txid, action_index, pool, height, block_time, value, memo) = row?;
            let txid = <[u8; 32]>::try_from(txid.as_slice())
                .map_err(|_| EngineError::Corrupt("txid is not 32 bytes".into()))?;
            notes.push(NoteRow {
                txid,
                action_index,
                pool: Pool::parse(&pool)?,
                height,
                block_time,
                value,
                memo,
            });
        }
        Ok(notes)
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
                 post_close_block_times, drand_round, beacon_time, timing, outcome, settlement, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                row.status.as_str()
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
                        beacon_time, timing, outcome, settlement, status
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
            "SELECT hex(txid), action_index, pool, height, block_time, value, hex(memo) FROM notes
             ORDER BY height, txid, action_index, pool",
        )?;
        for row in stmt.query_map([], |r| {
            Ok(format!(
                "note {} {} {} {} {} {} {}\n",
                r.get::<_, String>(0)?,
                r.get::<_, u16>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, String>(6)?
            ))
        })? {
            out.push_str(&row?);
        }
        let mut stmt = self.conn.prepare(
            "SELECT round, settled_at_height, start_block_time, post_close_block_times, drand_round,
                    beacon_time, timing, outcome, settlement, status FROM settlements ORDER BY round",
        )?;
        for row in stmt.query_map([], |r| {
            Ok(format!(
                "settlement {} {} {} {} {} {} {} {} {} {}\n",
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?
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
        };
        assert!(db.insert_note(&note).expect("insert"));
        assert!(
            !db.insert_note(&note).expect("dup"),
            "duplicate notes are ignored"
        );
        assert_eq!(db.notes_in_range(1, 5).expect("notes"), vec![note.clone()]);
        assert_eq!(note.to_received_note().txid, "07".repeat(32));

        let affected = db.rollback_to(4).expect("rollback");
        assert!(affected.is_empty());
        assert_eq!(db.tip().expect("tip").map(|b| b.height), Some(3));
        assert_eq!(db.note_count().expect("count"), 0);
    }
}
