//! FROST signing sessions with single-use nonces.
//!
//! Round 1 draws fresh nonces per pending spend and keeps them only in
//! memory, keyed by session. Round 2 removes the session before anything else
//! happens, so the nonces are consumed exactly once whatever follows; an
//! abort, a restart or a stale session simply loses them. The node records
//! the rounds it released shares for before answering, so a retried request
//! for the same rounds is refused.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zeek_vault::frost::{
    self, KeyPackage, SignatureShare, SigningCommitments, SigningNonces, SigningPackage,
};
use zeek_vault::send::PendingSpend;

use crate::db::NodeDb;
use crate::error::KeyholderError;
use crate::verify::Approved;
use zeek_engine::protocol::SessionId;

/// A session between round 1 and round 2.
struct Session {
    rounds: Vec<u32>,
    sighash: [u8; 32],
    pending: Vec<PendingSpend>,
    nonces: Vec<SigningNonces>,
    opened: Instant,
}

/// Sessions live at most this long between rounds.
pub const SESSION_TTL: Duration = Duration::from_secs(600);

/// In-memory session store.
#[derive(Default)]
pub struct SignerState {
    sessions: std::sync::Mutex<HashMap<SessionId, Session>>,
}

impl SignerState {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Session>> {
        match self.sessions.lock() {
            Ok(guard) => guard,
            // A poisoned lock only means a handler panicked while holding it;
            // the map itself is still consistent (inserts and removes are
            // single operations), so the nonces it holds remain single-use.
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn purge(sessions: &mut HashMap<SessionId, Session>) {
        sessions.retain(|_, s| s.opened.elapsed() < SESSION_TTL);
    }

    /// Round 1: draws nonces for every pending spend of an approved PCZT and
    /// returns the commitments to send to the coordinator.
    pub fn open_session(
        &self,
        session: SessionId,
        approved: Approved,
        key_package: &KeyPackage,
    ) -> Result<Vec<SigningCommitments>, KeyholderError> {
        let mut sessions = self.lock();
        Self::purge(&mut sessions);
        if sessions.contains_key(&session) {
            return Err(crate::error::protocol("session already open".into()));
        }
        let mut nonces = Vec::with_capacity(approved.pending.len());
        let mut commitments = Vec::with_capacity(approved.pending.len());
        for _ in &approved.pending {
            let (n, c) = frost::commit(key_package, &mut rand_core::OsRng);
            nonces.push(n);
            commitments.push(c);
        }
        sessions.insert(
            session,
            Session {
                rounds: approved.rounds,
                sighash: approved.sighash,
                pending: approved.pending,
                nonces,
                opened: Instant::now(),
            },
        );
        Ok(commitments)
    }

    /// Round 2: consumes the session's nonces and produces the shares.
    pub fn round2(
        &self,
        session: SessionId,
        packages: &[Vec<u8>],
        key_package: &KeyPackage,
        node_db: &NodeDb,
    ) -> Result<Vec<SignatureShare>, KeyholderError> {
        // Removing first guarantees the nonces are never used twice.
        let taken = {
            let mut sessions = self.lock();
            Self::purge(&mut sessions);
            sessions.remove(&session)
        }
        .ok_or(KeyholderError::UnknownSession)?;
        if packages.len() != taken.pending.len() {
            return Err(crate::error::protocol(format!(
                "{} signing packages for {} pending spends",
                packages.len(),
                taken.pending.len()
            )));
        }
        let mut shares = Vec::with_capacity(packages.len());
        for ((bytes, nonces), spend) in packages.iter().zip(&taken.nonces).zip(&taken.pending) {
            let package = SigningPackage::deserialize(bytes)
                .map_err(|e| crate::error::protocol(format!("signing package: {e}")))?;
            if package.message().as_slice() != taken.sighash {
                return Err(crate::error::protocol(
                    "signing package message is not the sighash of the verified PCZT".into(),
                ));
            }
            shares.push(frost::sign_share(
                &package,
                nonces,
                key_package,
                &spend.alpha,
            )?);
        }
        // Record before answering: a lost response must not lead to a second signature.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        node_db.mark_signed(&taken.rounds, &hex::encode(session), now)?;
        tracing::info!(
            rounds = ?taken.rounds,
            spends = shares.len(),
            session = %hex::encode(session),
            "released signature shares"
        );
        Ok(shares)
    }

    /// Number of open sessions (tests, health).
    pub fn open_sessions(&self) -> usize {
        let mut sessions = self.lock();
        Self::purge(&mut sessions);
        sessions.len()
    }
}
