//! The any-wallet path: a ZIP 321 payment request that any Zcash wallet can
//! pay, with one payment per tile and the dig memo in its text encoding
//! (rules v3, `zeek1:` + base64url), because many wallets only send text
//! memos. The payout address the player pastes is sealed to the vault's key
//! inside each memo.

use rand_chacha_09::rand_core::SeedableRng;
use rand_chacha_09::ChaCha20Rng;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zeek_core::memo::{DigMemo, PayoutReceiver};
use zeek_core::params::Network;
use zeek_core::payment::{dig_request_uri, MemoEncoding};

use crate::error::PocketError;
use crate::tx::{EngineConfig, MAX_TILES};

/// What the player provides.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnyWalletRequest {
    pub config: EngineConfig,
    pub payout_address: String,
    pub tiles: Vec<u8>,
    pub amount_each_zat: u64,
    pub round: u32,
    /// The chain tip now; the request is refused while text memos are not
    /// yet active (they would be forfeited).
    pub chain_tip: u32,
    /// Campfire message for every dig (rules v4), at most 275 bytes.
    #[serde(default)]
    pub message: Option<String>,
}

/// An any-wallet shout (rules v4).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnyWalletShoutRequest {
    pub config: EngineConfig,
    pub message: String,
    pub amount_zat: u64,
    pub chain_tip: u32,
}

/// A shout request to show as a QR code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnyWalletShout {
    pub uri: String,
    pub amount_zat: u64,
    pub memo: String,
    pub notice: String,
}

/// What to tell a shouter.
pub const SHOUT_NOTICE: &str = "A shout is a public message on the Campfire. Its amount goes to \
the Deep Cache and is never refunded. Anyone can read the message; nobody can tell who sent it.";

/// Builds a ZIP 321 request for a shout with a text memo.
pub fn build_shout(request: &AnyWalletShoutRequest) -> Result<AnyWalletShout, PocketError> {
    let params = &request.config.params;
    let next = request.chain_tip.saturating_add(1);
    let campfire = params
        .campfire
        .as_ref()
        .filter(|_| params.campfire_active(next))
        .ok_or_else(|| {
            PocketError::InvalidRequest("Campfire is not active yet in this game".into())
        })?;
    if request.amount_zat < campfire.min_shout_zat {
        return Err(PocketError::InvalidAmount(format!(
            "a shout must be at least {} zatoshis",
            campfire.min_shout_zat
        )));
    }
    let memo = zeek_core::memo::ShoutMemo::new(&request.message)
        .map_err(|e| PocketError::InvalidRequest(format!("message: {e}")))?;
    let uri = zeek_core::payment::shout_request_uri(
        Network::Test,
        &request.config.vault_address,
        request.amount_zat,
        &memo,
        MemoEncoding::Text,
    )
    .map_err(|e| PocketError::InvalidRequest(e.to_string()))?;
    Ok(AnyWalletShout {
        uri,
        amount_zat: request.amount_zat,
        memo: memo.encode_text(),
        notice: SHOUT_NOTICE.to_owned(),
    })
}

/// The request to show as a QR code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnyWalletDig {
    pub uri: String,
    pub round: u32,
    pub tiles: Vec<u8>,
    pub amount_each_zat: u64,
    pub total_zat: u64,
    /// The text memo of each payment, in tile order.
    pub memos: Vec<String>,
    /// What to tell the player.
    pub notice: String,
}

/// Plain-language advice returned with every request.
pub const NOTICE: &str = "Paste a fresh receive address from your wallet as the payout address \
(most wallets can generate a new shielded address each time). It is sealed so only the vault \
can read it, and a new one per dig keeps your digs unlinkable. Pay the request before the \
round closes; if it arrives late or names a closed round it is refunded to that address.";

/// Validates the payout address, seals it into one text memo per tile and
/// builds the ZIP 321 URI.
pub fn build(request: &AnyWalletRequest) -> Result<AnyWalletDig, PocketError> {
    let params = &request.config.params;
    if params.network != Network::Test {
        return Err(PocketError::InvalidRequest("testnet only".into()));
    }
    if !params.text_memo_active(request.chain_tip.saturating_add(1)) {
        return Err(PocketError::InvalidRequest(
            "this game does not accept text memos yet (rules v3 is not active), so an any-wallet dig would be forfeited".into(),
        ));
    }
    if request.tiles.is_empty() || request.tiles.len() > MAX_TILES {
        return Err(PocketError::InvalidRequest(format!(
            "choose between 1 and {MAX_TILES} tiles"
        )));
    }
    let mut unique = request.tiles.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != request.tiles.len() || unique.iter().any(|t| *t >= zeek_core::TILE_COUNT) {
        return Err(PocketError::InvalidRequest(
            "tiles must be distinct, 0 to 24".into(),
        ));
    }
    if request.amount_each_zat < params.min_dig_zat {
        return Err(PocketError::InvalidAmount(format!(
            "each dig must be at least {} zatoshis",
            params.min_dig_zat
        )));
    }
    // Shielded Orchard-protocol receivers only: transparent and
    // Sapling-only addresses are rejected here.
    let receiver = PayoutReceiver::from_unified_address(Network::Test, &request.payout_address)
        .map_err(|e| PocketError::InvalidAddress(e.to_string()))?;
    let message = request.message.as_deref().filter(|m| !m.is_empty());
    if message.is_some() && !params.campfire_active(request.chain_tip.saturating_add(1)) {
        return Err(PocketError::InvalidRequest(
            "Campfire messages are not active yet in this game".into(),
        ));
    }
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let mut rng = ChaCha20Rng::from_seed(seed);
    let memos: Vec<DigMemo> = request
        .tiles
        .iter()
        .map(|tile| {
            DigMemo::seal(
                request.round,
                *tile,
                &receiver,
                &request.config.sealing_public_key,
                &mut rng,
            )
            .map_err(|e| PocketError::Build(e.to_string()))
            .and_then(|memo| match message {
                Some(m) => memo
                    .with_message(m)
                    .map_err(|e| PocketError::InvalidRequest(format!("message: {e}"))),
                None => Ok(memo),
            })
        })
        .collect::<Result<_, _>>()?;
    let digs: Vec<(u64, &DigMemo)> = memos.iter().map(|m| (request.amount_each_zat, m)).collect();
    let uri = dig_request_uri(
        Network::Test,
        &request.config.vault_address,
        &digs,
        MemoEncoding::Text,
    )
    .map_err(|e| PocketError::InvalidRequest(e.to_string()))?;
    let total_zat = request
        .amount_each_zat
        .checked_mul(request.tiles.len() as u64)
        .ok_or(PocketError::Overflow("total"))?;
    Ok(AnyWalletDig {
        uri,
        round: request.round,
        tiles: request.tiles.clone(),
        amount_each_zat: request.amount_each_zat,
        total_zat,
        memos: memos.iter().map(DigMemo::encode_text).collect(),
        notice: NOTICE.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeek_core::params::GameParams;
    use zeek_core::seal::generate_sealing_keypair;

    fn request(payout: &str) -> (AnyWalletRequest, zeek_core::seal::SealingPrivateKey) {
        let mut rng = rand_chacha_09::ChaCha20Rng::from_seed([5; 32]);
        let (sk, pk) = generate_sealing_keypair(&mut rng);
        let mut params = GameParams::testnet(1_000);
        params.text_memo_activation_height = Some(1_100);
        let vault = PayoutReceiver {
            network: Network::Test,
            orchard: crate::keys::Keys::from_entropy([8; 32])
                .expect("keys")
                .address(0)
                .to_raw_address_bytes(),
        }
        .to_unified_address()
        .expect("ua");
        (
            AnyWalletRequest {
                config: EngineConfig {
                    params,
                    vault_address: vault,
                    sealing_public_key: pk,
                },
                payout_address: payout.to_owned(),
                tiles: vec![2, 9],
                amount_each_zat: 100_000,
                round: 4,
                chain_tip: 1_200,
                message: None,
            },
            sk,
        )
    }

    #[test]
    fn builds_a_text_memo_request_that_opens_to_the_payout_address() {
        let player = crate::keys::Keys::from_entropy([6; 32]).expect("keys");
        let payout = crate::keys::encode_address(&player.address(7)).expect("ua");
        let (req, sk) = request(&payout);
        let dig = build(&req).expect("request");
        assert_eq!(dig.total_zat, 200_000);
        assert!(dig.notice.contains("fresh receive address"));
        let parsed = zip321::TransactionRequest::from_uri(&dig.uri).expect("zip321");
        assert_eq!(parsed.payments().len(), 2);
        for (index, tile) in [2u8, 9].iter().enumerate() {
            let payment = parsed.payments().get(&index).expect("payment");
            assert_eq!(payment.amount().map(u64::from), Some(100_000));
            let memo = payment.memo().expect("memo").as_slice().to_vec();
            assert!(memo.starts_with(b"zeek1:"), "text memo");
            let decoded = DigMemo::decode_text(&memo).expect("decode");
            assert_eq!(decoded.tile, *tile);
            assert_eq!(decoded.round, 4);
            assert_eq!(
                decoded.open(Network::Test, &sk).expect("open").orchard,
                player.address(7).to_raw_address_bytes()
            );
            assert_eq!(dig.memos[index].as_bytes(), memo.as_slice());
        }
    }

    #[test]
    fn rejects_transparent_payouts_and_inactive_text_memos() {
        let (req, _) = request("tmEZhbWHTpdKMw5it8YDspUXSMGQyFwovpU");
        assert_eq!(
            build(&req).expect_err("transparent").code(),
            "invalid_address"
        );
        let player = crate::keys::Keys::from_entropy([6; 32]).expect("keys");
        let payout = crate::keys::encode_address(&player.address(1)).expect("ua");
        let (mut req, _) = request(&payout);
        req.chain_tip = 1_000;
        assert_eq!(build(&req).expect_err("inactive").code(), "invalid_request");
        let (mut req, _) = request(&payout);
        req.tiles = vec![3, 3];
        assert!(build(&req).is_err());
        let (mut req, _) = request(&payout);
        req.amount_each_zat = 99_999;
        assert_eq!(build(&req).expect_err("small").code(), "invalid_amount");
    }
}
