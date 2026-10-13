//! Signed message envelopes exchanged between keyholders and the coordinator.
//!
//! Every message carries a 32-byte session id, the sender's participant
//! identifier (keyholders `1..=n`, the coordinator `0`), a kind, a postcard
//! payload and an ed25519 signature by the sender's pinned identity over
//! `DOMAIN || session || sender (BE) || kind || payload`. Receivers reject a
//! message whose signature, sender, kind or session is not the expected one.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::identity::{self, Identity};

/// Signature domain.
pub const DOMAIN: &[u8] = b"zeek/keyholder/v1";

/// A session id.
pub type SessionId = [u8; 32];

/// Message kinds.
pub mod kind {
    pub const DKG_ROUND1: &str = "dkg.round1";
    pub const DKG_ROUND2: &str = "dkg.round2";
    pub const DKG_CONFIRM: &str = "dkg.confirm";
    pub const SIGN_REQUEST: &str = "sign.request";
    pub const SIGN_COMMITMENTS: &str = "sign.commitments";
    pub const SIGN_PACKAGES: &str = "sign.packages";
    pub const SIGN_SHARES: &str = "sign.shares";
    pub const REFUSAL: &str = "refusal";
}

/// The wire form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Session id, hex.
    pub session: String,
    /// Sender participant identifier.
    pub sender: u16,
    /// Message kind.
    pub kind: String,
    /// Postcard-encoded payload, hex.
    pub payload: String,
    /// ed25519 signature, hex.
    pub signature: String,
}

fn signed_bytes(session: &SessionId, sender: u16, kind: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(DOMAIN.len() + 32 + 2 + kind.len() + payload.len());
    bytes.extend_from_slice(DOMAIN);
    bytes.extend_from_slice(session);
    bytes.extend_from_slice(&sender.to_be_bytes());
    bytes.extend_from_slice(kind.as_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

/// Builds and signs an envelope.
pub fn seal<T: Serialize>(
    identity: &Identity,
    session: &SessionId,
    sender: u16,
    kind: &str,
    payload: &T,
) -> Result<Envelope, EngineError> {
    let payload = postcard::to_allocvec(payload)
        .map_err(|e| EngineError::Protocol(format!("encode payload: {e}")))?;
    let signature = identity.sign(&signed_bytes(session, sender, kind, &payload));
    Ok(Envelope {
        session: hex::encode(session),
        sender,
        kind: kind.to_owned(),
        payload: hex::encode(payload),
        signature: hex::encode(signature),
    })
}

/// What a receiver expects of an envelope.
pub struct Expect<'a> {
    pub kind: &'a str,
    pub sender: u16,
    /// Pinned public key of `sender`, hex.
    pub sender_key: &'a str,
    /// Required session id, if the receiver already has one.
    pub session: Option<&'a SessionId>,
}

/// Verifies an envelope and decodes its payload.
pub fn open<T: DeserializeOwned>(
    envelope: &Envelope,
    expect: &Expect<'_>,
) -> Result<(SessionId, T), EngineError> {
    if envelope.kind != expect.kind {
        return Err(EngineError::Protocol(format!(
            "expected a `{}` message, got `{}`",
            expect.kind, envelope.kind
        )));
    }
    if envelope.sender != expect.sender {
        return Err(EngineError::Protocol(format!(
            "message from participant {} where {} was expected",
            envelope.sender, expect.sender
        )));
    }
    let session = hex::decode(&envelope.session)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
        .ok_or_else(|| EngineError::Protocol("session id must be 32 hex bytes".into()))?;
    if let Some(required) = expect.session {
        if *required != session {
            return Err(EngineError::Protocol(
                "message belongs to another session".into(),
            ));
        }
    }
    let payload = hex::decode(&envelope.payload)
        .map_err(|e| EngineError::Protocol(format!("payload hex: {e}")))?;
    let signature = hex::decode(&envelope.signature)
        .map_err(|e| EngineError::Protocol(format!("signature hex: {e}")))?;
    identity::verify(
        expect.sender_key,
        &signed_bytes(&session, envelope.sender, &envelope.kind, &payload),
        &signature,
    )?;
    let value = postcard::from_bytes(&payload)
        .map_err(|e| EngineError::Protocol(format!("decode payload: {e}")))?;
    Ok((session, value))
}

/// DKG round 1 package broadcast to every peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DkgRound1 {
    pub package: Vec<u8>,
}

/// DKG round 2 package addressed to one peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DkgRound2 {
    pub package: Vec<u8>,
}

/// What a node derived from the ceremony, for cross-checking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DkgConfirm {
    pub group_key: [u8; 32],
    pub full_viewing_key: Vec<u8>,
    pub address: String,
}

/// Coordinator → keyholder: please sign this payout PCZT.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignRequest {
    /// Serialized PCZT.
    pub pczt: Vec<u8>,
    /// Rounds the coordinator claims this transaction settles.
    pub rounds: Vec<u32>,
}

/// Keyholder → coordinator: round 1 commitments, one per pending spend in
/// `zeek_vault::send::pending_spends` order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignCommitments {
    pub commitments: Vec<Vec<u8>>,
}

/// Coordinator → keyholder: the signing package of every pending spend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignPackages {
    pub packages: Vec<Vec<u8>>,
}

/// Keyholder → coordinator: round 2 shares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignShares {
    pub shares: Vec<Vec<u8>>,
}

/// A signed refusal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_verify_and_reject_tampering() {
        let id = Identity::generate();
        let other = Identity::generate();
        let session = [7u8; 32];
        let env = seal(
            &id,
            &session,
            2,
            kind::DKG_ROUND1,
            &DkgRound1 {
                package: vec![1, 2, 3],
            },
        )
        .expect("seal");
        let expect = Expect {
            kind: kind::DKG_ROUND1,
            sender: 2,
            sender_key: &id.public_key_hex(),
            session: Some(&session),
        };
        let (s, msg): (SessionId, DkgRound1) = open(&env, &expect).expect("open");
        assert_eq!(s, session);
        assert_eq!(msg.package, vec![1, 2, 3]);

        let wrong_key = Expect {
            sender_key: &other.public_key_hex(),
            ..expect
        };
        assert!(open::<DkgRound1>(&env, &wrong_key).is_err());
        let wrong_sender = Expect {
            sender: 3,
            ..expect
        };
        assert!(open::<DkgRound1>(&env, &wrong_sender).is_err());
        let wrong_kind = Expect {
            kind: kind::DKG_ROUND2,
            ..expect
        };
        assert!(open::<DkgRound1>(&env, &wrong_kind).is_err());
        let other_session = [8u8; 32];
        let wrong_session = Expect {
            session: Some(&other_session),
            ..expect
        };
        assert!(open::<DkgRound1>(&env, &wrong_session).is_err());
        let mut tampered = env.clone();
        tampered.payload = hex::encode([9u8; 4]);
        assert!(open::<DkgRound1>(&tampered, &expect).is_err());
        let mut tampered = env;
        tampered.session = hex::encode([8u8; 32]);
        assert!(
            open::<DkgRound1>(
                &tampered,
                &Expect {
                    session: None,
                    ..expect
                }
            )
            .is_err(),
            "session is signed"
        );
    }
}
