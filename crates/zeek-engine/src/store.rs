//! Database access shared by the chain loop and the API.
//!
//! One **writer** connection, used only for short synchronous closures, and a
//! pool of **read-only** connections. The database runs in WAL mode, so a
//! reader sees the last committed state and never waits for the writer.
//!
//! Both [`Store::read`] and [`Store::write`] take synchronous closures and
//! use `std::sync::Mutex`, whose guard is not `Send`: a guard can never be
//! held across an `.await` in a spawned task, so no network call, beacon
//! fetch or proof can run while a connection is checked out. Callers fetch
//! first, compute second, and then write once.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::db::Db;
use crate::error::EngineError;

/// Read connections opened up front; when all are busy an extra one is
/// opened for the call, up to [`MAX_EXTRA_READERS`] at once.
const READERS: usize = 8;
/// Extra read connections allowed at once (review N9).
const MAX_EXTRA_READERS: usize = 32;

/// Shared database handle.
pub struct Store {
    writer: Mutex<Db>,
    readers: Option<Readers>,
}

struct Readers {
    path: PathBuf,
    slots: Vec<Mutex<Db>>,
    extra: AtomicUsize,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic inside a closure cannot leave SQLite half-written: an open
    // transaction is rolled back before the connection is used again.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Store {
    /// Opens the database file (creating it and applying the schema) with a
    /// read pool over the same file.
    pub fn open(path: &Path) -> Result<Self, EngineError> {
        let writer = Db::open(path)?;
        let slots = (0..READERS)
            .map(|_| Db::open_reader(path).map(Mutex::new))
            .collect::<Result<_, _>>()?;
        Ok(Store {
            writer: Mutex::new(writer),
            readers: Some(Readers {
                path: path.to_owned(),
                slots,
                extra: AtomicUsize::new(0),
            }),
        })
    }

    /// An in-memory database (tests). Reads use the writer connection, since
    /// an in-memory database cannot be opened twice.
    pub fn open_in_memory() -> Result<Self, EngineError> {
        Ok(Store::from_db(Db::open_in_memory()?))
    }

    /// Wraps a single connection; reads and writes share it.
    pub fn from_db(db: Db) -> Self {
        Store {
            writer: Mutex::new(db),
            readers: None,
        }
    }

    /// Checks out a read-only connection. Never hold one across an `.await`
    /// (its guard is not `Send`, so a spawned task cannot).
    pub fn reader(&self) -> Result<Reader<'_>, EngineError> {
        let Some(readers) = &self.readers else {
            return Ok(Reader::Shared(guard(&self.writer)));
        };
        for slot in &readers.slots {
            if let Ok(db) = slot.try_lock() {
                return Ok(Reader::Slot(db));
            }
        }
        if readers.extra.fetch_add(1, Ordering::SeqCst) >= MAX_EXTRA_READERS {
            readers.extra.fetch_sub(1, Ordering::SeqCst);
            return Err(EngineError::Busy("every database reader is in use"));
        }
        let counter = &readers.extra;
        match Db::open_reader(&readers.path) {
            Ok(db) => Ok(Reader::Extra(ExtraReader { db, counter })),
            Err(e) => {
                counter.fetch_sub(1, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    /// Runs `f` on a read-only connection.
    pub fn read<T, E>(&self, f: impl FnOnce(&Db) -> Result<T, E>) -> Result<T, E>
    where
        E: From<EngineError>,
    {
        let reader = self.reader()?;
        f(&reader)
    }

    /// Runs `f` on the writer connection. Keep it short: compute outside,
    /// write inside. Use [`Db::transaction`] inside for several statements
    /// that must land together.
    pub fn write<T, E>(&self, f: impl FnOnce(&mut Db) -> Result<T, E>) -> Result<T, E>
    where
        E: From<EngineError>,
    {
        let mut db = guard(&self.writer);
        db.abandon_open_transaction()?;
        f(&mut db)
    }
}

/// A connection opened beyond the pool; it is closed on drop.
pub struct ExtraReader<'a> {
    db: Db,
    counter: &'a AtomicUsize,
}

impl Drop for ExtraReader<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A checked-out read connection.
pub enum Reader<'a> {
    Slot(MutexGuard<'a, Db>),
    Extra(ExtraReader<'a>),
    /// In-memory stores read through the writer connection.
    Shared(MutexGuard<'a, Db>),
}

impl Deref for Reader<'_> {
    type Target = Db;

    fn deref(&self) -> &Db {
        match self {
            Reader::Slot(db) | Reader::Shared(db) => db,
            Reader::Extra(extra) => &extra.db,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::BlockRow;

    fn block(height: u32) -> BlockRow {
        BlockRow {
            height,
            hash: vec![height as u8; 32],
            prev_hash: vec![height.wrapping_sub(1) as u8; 32],
            time: 1_700_000_000 + u64::from(height),
        }
    }

    #[test]
    fn readers_see_commits_and_never_wait_for_an_open_write() {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::open(&dir.path().join("e.sqlite")).expect("open");
        store.write(|db| db.insert_block(&block(1))).expect("write");
        assert_eq!(
            store
                .read(|db| db.tip().map(|t| t.map(|b| b.height)))
                .expect("read"),
            Some(1)
        );

        // A write transaction stays open (uncommitted) while a reader runs
        // on another thread: the reader sees the last commit, immediately.
        std::thread::scope(|s| {
            let (opened, wait_open) = std::sync::mpsc::channel();
            let (release, wait_release) = std::sync::mpsc::channel::<()>();
            let store = &store;
            let writer = s.spawn(move || {
                store.write(|db| {
                    db.transaction(|tx| {
                        tx.insert_block(&block(2))?;
                        opened
                            .send(())
                            .map_err(|e| EngineError::Server(e.to_string()))?;
                        wait_release
                            .recv()
                            .map_err(|e| EngineError::Server(e.to_string()))?;
                        Ok(())
                    })
                })
            });
            wait_open.recv().expect("writer opened");
            let started = std::time::Instant::now();
            let seen = store
                .read(|db| db.tip().map(|t| t.map(|b| b.height)))
                .expect("read during write");
            assert!(started.elapsed() < std::time::Duration::from_millis(50));
            assert_eq!(seen, Some(1), "uncommitted rows are invisible");
            release.send(()).expect("release");
            writer.join().expect("join").expect("write");
        });
        assert_eq!(
            store
                .read(|db| db.tip().map(|t| t.map(|b| b.height)))
                .expect("read"),
            Some(2)
        );
    }

    #[test]
    fn readers_cannot_write() {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::open(&dir.path().join("e.sqlite")).expect("open");
        let refused = store.read(|db| db.insert_block(&block(1)));
        assert!(refused.is_err());
    }
}
