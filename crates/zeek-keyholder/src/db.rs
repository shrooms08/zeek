//! The node's own records: which rounds it has released signature shares
//! for. A keyholder never signs a round twice, whatever the coordinator says.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::KeyholderError;

/// The node database.
pub struct NodeDb {
    conn: Connection,
}

impl NodeDb {
    /// Opens or creates the database.
    pub fn open(path: &Path) -> Result<Self, KeyholderError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| KeyholderError::io(parent, e))?;
            }
        }
        Self::init(Connection::open(path)?)
    }

    /// In-memory database (tests).
    pub fn open_in_memory() -> Result<Self, KeyholderError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, KeyholderError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS signed_rounds (
                 round INTEGER PRIMARY KEY,
                 session TEXT NOT NULL,
                 signed_at INTEGER NOT NULL
             );",
        )?;
        Ok(NodeDb { conn })
    }

    /// The session a round was signed in, if any.
    pub fn signed_session(&self, round: u32) -> Result<Option<String>, KeyholderError> {
        Ok(self
            .conn
            .query_row(
                "SELECT session FROM signed_rounds WHERE round = ?1",
                params![round],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Records that shares for `rounds` were released in `session`: all of
    /// them or, if any round was already signed, none (security review L2).
    pub fn mark_signed(
        &self,
        rounds: &[u32],
        session: &str,
        now: u64,
    ) -> Result<(), KeyholderError> {
        let tx = self.conn.unchecked_transaction()?;
        for round in rounds {
            tx.execute(
                "INSERT INTO signed_rounds (round, session, signed_at) VALUES (?1, ?2, ?3)",
                params![round, session, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_are_recorded_once() {
        let db = NodeDb::open_in_memory().expect("db");
        assert_eq!(db.signed_session(3).expect("q"), None);
        db.mark_signed(&[3, 4], "abc", 1).expect("mark");
        assert_eq!(db.signed_session(3).expect("q"), Some("abc".into()));
        assert!(
            db.mark_signed(&[4], "def", 2).is_err(),
            "primary key forbids a second signing"
        );
        // A request that includes a signed round marks none of its rounds.
        assert!(db.mark_signed(&[5, 3], "ghi", 3).is_err());
        assert_eq!(db.signed_session(5).expect("q"), None);
    }
}
