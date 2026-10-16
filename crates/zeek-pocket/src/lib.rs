//! Zeek Pocket: the browser wallet core.
//!
//! Plain Rust that also compiles natively (for tests); the `wasm32` build
//! exports [`PocketCore`] through `wasm-bindgen` and the rayon thread-pool
//! initialiser the Web Worker calls once. The TypeScript package
//! (`packages/zeek-pocket`) owns networking (the engine's relay) and
//! IndexedDB; this crate owns keys, encryption, scanning, notes and
//! transactions. Secrets never cross into JavaScript except the recovery
//! phrase on an explicit, passphrase-checked export.

#![forbid(unsafe_code)]

pub mod bench;
pub mod crypto;
pub mod error;
pub mod keys;
pub mod request;
pub mod store;
pub mod tx;
pub mod wallet;

use serde::Serialize;
use wasm_bindgen::prelude::*;
use zeroize::Zeroizing;

#[cfg(target_arch = "wasm32")]
pub use wasm_bindgen_rayon::init_thread_pool;

use crate::crypto::SeedEnvelope;
use crate::error::PocketError;
use crate::keys::Keys;
use crate::tx::{DigRequest, EngineConfig, SendRequest};
use crate::wallet::Wallet;

/// Installs the panic hook so Rust panics surface in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Builds, proves, signs and extracts one Ironwood transaction (one spend,
/// `payments` payments plus change) and returns the timings as JSON (see
/// [`bench::BenchReport`]).
#[wasm_bindgen]
pub fn bench_prove(payments: u32) -> Result<JsValue, JsValue> {
    let report =
        bench::run(js_sys::Date::now, payments).map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_wasm_bindgen::to_value(&report).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// The error object JavaScript receives: `{ code, message, height? }`
/// (`height` is the fork height of a reorg).
#[derive(Serialize)]
struct JsError {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
}

fn js_err(e: PocketError) -> JsValue {
    let height = match &e {
        PocketError::Reorg { fork_height } => Some(*fork_height),
        _ => None,
    };
    serde_wasm_bindgen::to_value(&JsError {
        code: e.code(),
        message: e.to_string(),
        height,
    })
    .unwrap_or_else(|_| JsValue::from_str(&e.to_string()))
}

/// Builds the any-wallet shout request (rules v4): JSON
/// `{ config, message, amountZat, chainTip }` in, a ZIP 321 URI out.
#[wasm_bindgen(js_name = buildShoutRequest)]
pub fn build_shout_request(request: &str) -> Result<JsValue, JsValue> {
    let request: request::AnyWalletShoutRequest = from_json(request, "shout request")?;
    to_js(&request::build_shout(&request).map_err(js_err)?)
}

fn verify_err(message: impl Into<String>) -> JsValue {
    js_err(PocketError::InvalidRequest(message.into()))
}

fn bundle_from(json: &str) -> Result<zeek_core::proof::ProofBundle, JsValue> {
    from_json(json, "proof bundle")
}

/// Verifier step 1: the compact-block range a round's checks need, after
/// checking the bundle's window against its parameters.
#[wasm_bindgen(js_name = verifyPlan)]
pub fn verify_plan(bundle: &str) -> Result<JsValue, JsValue> {
    let bundle = bundle_from(bundle)?;
    zeek_verify::checks::check_window(&bundle).map_err(|e| verify_err(e.to_string()))?;
    let (start, end) = zeek_verify::checks::block_range(&bundle);
    to_js(&serde_json::json!({
        "start": start,
        "end": end,
        "payoutHeight": zeek_verify::checks::payout_height(&bundle),
    }))
}

/// Verifier step 2: trial-decrypts the window's compact blocks (relay
/// encoding) with the public UIVK and returns the txids to fetch.
#[wasm_bindgen(js_name = verifyScan)]
pub fn verify_scan(bundle: &str, config: &str, blocks: &[u8]) -> Result<JsValue, JsValue> {
    let bundle = bundle_from(bundle)?;
    let config: zeek_verify::checks::PublicConfig = from_json(config, "public config")?;
    let ivk = config.ivk().map_err(|e| verify_err(e.to_string()))?;
    let blocks = zeek_verify::chain::Blocks::new(
        zeek_verify::chain::decode_relay_blocks(blocks).map_err(|e| verify_err(e.to_string()))?,
    );
    let hits = zeek_verify::checks::scan_window(&bundle, &ivk, &blocks);
    to_js(&zeek_verify::checks::transactions_to_fetch(&bundle, &hits))
}

fn mined_txs(
    txs: &js_sys::Array,
    network: zeek_core::params::Network,
) -> Result<std::collections::BTreeMap<String, zeek_verify::chain::MinedTx>, JsValue> {
    use prost::Message;
    let mut out = std::collections::BTreeMap::new();
    for value in txs.iter() {
        let bytes = js_sys::Uint8Array::new(&value).to_vec();
        let raw = zcash_client_backend::proto::service::RawTransaction::decode(bytes.as_slice())
            .map_err(|e| verify_err(format!("raw transaction: {e}")))?;
        let height = u32::try_from(raw.height).map_err(|_| verify_err("height overflows"))?;
        let mined = zeek_verify::chain::MinedTx {
            raw: raw.data,
            height,
        };
        let tx = zeek_verify::chain::parse_tx(network, "fetched", &mined)
            .map_err(|e| verify_err(e.to_string()))?;
        out.insert(tx.txid().to_string(), mined);
    }
    Ok(out)
}

/// Verifier step 3: runs the five checks. `blocks` holds the window, the
/// blocks after it and the payout block (relay encoding); `txs` the relay's
/// `RawTransaction` responses for the txids of step 2.
#[wasm_bindgen(js_name = verifyRound)]
pub fn verify_round(
    bundle: &str,
    config: &str,
    blocks: &[u8],
    txs: js_sys::Array,
) -> Result<JsValue, JsValue> {
    let bundle = bundle_from(bundle)?;
    let config: zeek_verify::checks::PublicConfig = from_json(config, "public config")?;
    let ivk = config.ivk().map_err(|e| verify_err(e.to_string()))?;
    let blocks = zeek_verify::chain::Blocks::new(
        zeek_verify::chain::decode_relay_blocks(blocks).map_err(|e| verify_err(e.to_string()))?,
    );
    let txs = mined_txs(&txs, bundle.params.network)?;
    let report = zeek_verify::checks::verify_round(&zeek_verify::checks::RoundInput {
        config: &config,
        bundle: &bundle,
        blocks: &blocks,
        txs: &txs,
        ivk: &ivk,
    });
    to_js(&report)
}

/// Replay mode (engine off): the parts of the round checks that need no
/// chain data. `public_beacon` is the bundle's beacon as a public drand
/// relay served it (`zeek_verify::offline::PublicBeacon` JSON), or "" for a
/// voided round; chain-dependent parts come back as `needs_chain`.
#[wasm_bindgen(js_name = verifyOffline)]
pub fn verify_offline(bundle: &str, config: &str, public_beacon: &str) -> Result<JsValue, JsValue> {
    let bundle = bundle_from(bundle)?;
    let config: zeek_verify::checks::PublicConfig = from_json(config, "public config")?;
    let public: Option<zeek_verify::offline::PublicBeacon> = if public_beacon.is_empty() {
        None
    } else {
        Some(from_json(public_beacon, "public beacon")?)
    };
    to_js(&zeek_verify::offline::verify_offline(
        &bundle,
        &config,
        public.as_ref(),
    ))
}

/// Heights and hashes (display hex) of relay-encoded compact blocks, for
/// cross-checking one chain source against another.
#[wasm_bindgen(js_name = blockHashes)]
pub fn block_hashes(blocks: &[u8]) -> Result<JsValue, JsValue> {
    let blocks =
        zeek_verify::chain::decode_relay_blocks(blocks).map_err(|e| verify_err(e.to_string()))?;
    let hashes: Vec<serde_json::Value> = blocks
        .iter()
        .map(|b| {
            let mut hash = b.hash.clone();
            hash.reverse();
            serde_json::json!({ "height": b.height, "hash": hex::encode(hash) })
        })
        .collect();
    to_js(&hashes)
}

/// Decodes a base64url win proof into its claim fields (round, payout).
#[wasm_bindgen(js_name = decodeWinProof)]
pub fn decode_win_proof(encoded: &str) -> Result<JsValue, JsValue> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .map_err(|_| verify_err("the proof is not base64url"))?;
    let proof =
        zeek_verify::win::WinProof::decode(&bytes).map_err(|e| verify_err(e.to_string()))?;
    to_js(&serde_json::json!({
        "id": zeek_verify::win::WinProof::id(&bytes),
        "round": proof.round,
        "payoutIndex": proof.payout_index,
        "payoutTxid": proof.payout_txid,
    }))
}

/// Verifies a win proof: the round's bundle, the relay's `RawTransaction`
/// response for the payout transaction, and the relay-encoded compact block
/// at its height (to confirm inclusion).
#[wasm_bindgen(js_name = verifyWinProof)]
pub fn verify_win_proof(
    encoded: &str,
    bundle: &str,
    payout_tx: &[u8],
    block: &[u8],
) -> Result<JsValue, JsValue> {
    use base64::Engine;
    use prost::Message;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .map_err(|_| verify_err("the proof is not base64url"))?;
    let proof =
        zeek_verify::win::WinProof::decode(&bytes).map_err(|e| verify_err(e.to_string()))?;
    let bundle = bundle_from(bundle)?;
    let raw = zcash_client_backend::proto::service::RawTransaction::decode(payout_tx)
        .map_err(|e| verify_err(format!("raw transaction: {e}")))?;
    let mined = zeek_verify::chain::MinedTx {
        raw: raw.data,
        height: u32::try_from(raw.height).map_err(|_| verify_err("height overflows"))?,
    };
    let blocks = zeek_verify::chain::Blocks::new(
        zeek_verify::chain::decode_relay_blocks(block).map_err(|e| verify_err(e.to_string()))?,
    );
    let included = zeek_verify::chain::txid_bytes(&proof.payout_txid)
        .is_some_and(|t| blocks.contains_tx(mined.height, &t));
    to_js(&zeek_verify::win::verify(&proof, &bundle, &mined, included))
}
/// `{ config, payoutAddress, tiles, amountEachZat, round, chainTip }` in,
/// a ZIP 321 URI with one text-memo payment per tile out (see
/// [`request::build`]). Needs no wallet.
#[wasm_bindgen(js_name = buildDigRequest)]
pub fn build_dig_request(request: &str) -> Result<JsValue, JsValue> {
    let request: request::AnyWalletRequest = from_json(request, "dig request")?;
    to_js(&request::build(&request).map_err(js_err)?)
}

fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| js_err(PocketError::Corrupt(format!("serialize: {e}"))))
}

fn from_json<T: serde::de::DeserializeOwned>(json: &str, what: &str) -> Result<T, JsValue> {
    serde_json::from_str(json)
        .map_err(|e| js_err(PocketError::InvalidRequest(format!("{what}: {e}"))))
}

/// An unlocked wallet. Dropping it (lock) drops the keys.
#[wasm_bindgen]
pub struct PocketCore {
    wallet: Wallet,
    state_key: Zeroizing<[u8; 32]>,
}

/// What `create` and `restore` return: the wallet and its sealed seed.
#[wasm_bindgen]
pub struct Opened {
    core: Option<PocketCore>,
    seed_envelope: String,
}

#[wasm_bindgen]
impl Opened {
    /// The seed envelope (JSON) to store; it never contains the seed in clear.
    #[wasm_bindgen(getter, js_name = seedEnvelope)]
    pub fn seed_envelope(&self) -> String {
        self.seed_envelope.clone()
    }

    /// Takes the wallet (once).
    #[wasm_bindgen(js_name = takeCore)]
    pub fn take_core(&mut self) -> Result<PocketCore, JsValue> {
        self.core
            .take()
            .ok_or_else(|| js_err(PocketError::InvalidRequest("wallet already taken".into())))
    }
}

impl PocketCore {
    fn open(keys: Keys, passphrase: &str, tree_state: &[u8]) -> Result<Opened, PocketError> {
        let envelope = crypto::seal_seed(keys.entropy(), passphrase)?;
        let state_key = crypto::state_key(keys.entropy())?;
        let wallet = Wallet::new(keys, tree_state)?;
        Ok(Opened {
            core: Some(PocketCore { wallet, state_key }),
            seed_envelope: serde_json::to_string(&envelope)
                .map_err(|e| PocketError::Corrupt(e.to_string()))?,
        })
    }
}

#[wasm_bindgen]
impl PocketCore {
    /// A new wallet with a fresh random 24-word phrase. `tree_state` is the
    /// `TreeState` protobuf of the birthday block (the current tip).
    pub fn create(passphrase: &str, tree_state: &[u8]) -> Result<Opened, JsValue> {
        let mut entropy = Zeroizing::new([0u8; 32]);
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, entropy.as_mut());
        let keys = Keys::from_entropy(*entropy).map_err(js_err)?;
        Self::open(keys, passphrase, tree_state).map_err(js_err)
    }

    /// Restores from a 24-word phrase; `tree_state` is the block before the
    /// first block that may hold the wallet's notes.
    pub fn restore(phrase: &str, passphrase: &str, tree_state: &[u8]) -> Result<Opened, JsValue> {
        let keys = Keys::from_phrase(phrase).map_err(js_err)?;
        Self::open(keys, passphrase, tree_state).map_err(js_err)
    }

    /// Unlocks a stored wallet: the seed envelope (JSON) and the state
    /// envelope (bytes) from IndexedDB.
    pub fn unlock(
        seed_envelope: &str,
        passphrase: &str,
        state_envelope: &[u8],
    ) -> Result<PocketCore, JsValue> {
        let envelope: SeedEnvelope = from_json(seed_envelope, "seed envelope")?;
        let entropy = crypto::open_seed(&envelope, passphrase).map_err(js_err)?;
        let state_key = crypto::state_key(&entropy).map_err(js_err)?;
        let bytes = crypto::open_state(&state_key, state_envelope).map_err(js_err)?;
        let state = store::from_bytes(&bytes).map_err(js_err)?;
        let keys = Keys::from_entropy(*entropy).map_err(js_err)?;
        let wallet = Wallet::from_state(keys, state).map_err(js_err)?;
        Ok(PocketCore { wallet, state_key })
    }

    /// Checks the passphrase against the seed envelope and returns the
    /// 24-word phrase.
    #[wasm_bindgen(js_name = exportPhrase)]
    pub fn export_phrase(&self, seed_envelope: &str, passphrase: &str) -> Result<String, JsValue> {
        self.confirm_passphrase(seed_envelope, passphrase)?;
        Ok(self.wallet.keys().phrase().map_err(js_err)?.to_string())
    }

    /// Checks `passphrase` against the stored seed of this open wallet
    /// (before a send leaves the game, security review M4).
    #[wasm_bindgen(js_name = confirmPassphrase)]
    pub fn confirm_passphrase(&self, seed_envelope: &str, passphrase: &str) -> Result<(), JsValue> {
        let envelope: SeedEnvelope = from_json(seed_envelope, "seed envelope")?;
        let entropy = crypto::open_seed(&envelope, passphrase).map_err(js_err)?;
        if *entropy != *self.wallet.keys().entropy() {
            return Err(js_err(PocketError::BadPassphrase));
        }
        Ok(())
    }

    /// The encrypted state to persist.
    #[wasm_bindgen(js_name = sealedState)]
    pub fn sealed_state(&mut self) -> Result<Vec<u8>, JsValue> {
        let bytes = Zeroizing::new(self.wallet.state_bytes().map_err(js_err)?);
        crypto::seal_state(&self.state_key, &bytes).map_err(js_err)
    }

    #[wasm_bindgen(js_name = receiveAddress)]
    pub fn receive_address(&self) -> Result<String, JsValue> {
        self.wallet.receive_address().map_err(js_err)
    }

    #[wasm_bindgen(js_name = scannedHeight)]
    pub fn scanned_height(&self) -> u32 {
        self.wallet.scanned_height()
    }

    #[wasm_bindgen(js_name = birthdayHeight)]
    pub fn birthday_height(&self) -> u32 {
        self.wallet.birthday_height()
    }

    /// Scans relay-encoded compact blocks; see [`Wallet::scan`].
    pub fn scan(&mut self, blocks: &[u8]) -> Result<JsValue, JsValue> {
        to_js(&self.wallet.scan(blocks).map_err(js_err)?)
    }

    pub fn rewind(&mut self, height: u32) -> Result<(), JsValue> {
        self.wallet.rewind(height).map_err(js_err)
    }

    #[wasm_bindgen(js_name = memoRequests)]
    pub fn memo_requests(&self) -> Result<JsValue, JsValue> {
        to_js(&self.wallet.memo_requests())
    }

    /// Reads memos from a `RawTransaction` protobuf as the relay serves it.
    #[wasm_bindgen(js_name = attachRelayTransaction)]
    pub fn attach_relay_transaction(&mut self, relay_bytes: &[u8]) -> Result<u32, JsValue> {
        use prost::Message;
        let raw = zcash_client_backend::proto::service::RawTransaction::decode(relay_bytes)
            .map_err(|e| js_err(PocketError::BadChainData(format!("raw transaction: {e}"))))?;
        let height = u32::try_from(raw.height).unwrap_or(0);
        if height == 0 {
            // Not mined yet: nothing of ours can be in it.
            return Ok(0);
        }
        self.wallet
            .attach_transaction(&raw.data, height)
            .map_err(js_err)
    }

    pub fn balance(&self) -> Result<JsValue, JsValue> {
        to_js(&self.wallet.balance().map_err(js_err)?)
    }

    pub fn history(&self) -> Result<JsValue, JsValue> {
        to_js(&self.wallet.history())
    }

    /// Builds a dig. `config` is the engine's `/v1/config` JSON, `request`
    /// a JSON `{ tiles, amountEachZat, round, blocksRemaining, chainTip }`.
    #[wasm_bindgen(js_name = buildDig)]
    pub fn build_dig(&mut self, config: &str, request: &str) -> Result<JsValue, JsValue> {
        let config: EngineConfig = from_json(config, "engine config")?;
        let request: DigRequest = from_json(request, "dig request")?;
        to_js(&self.wallet.build_dig(&config, &request).map_err(js_err)?)
    }

    /// Builds a shout (rules v4): JSON `{ message, amountZat, chainTip }`.
    #[wasm_bindgen(js_name = buildShout)]
    pub fn build_shout(&mut self, config: &str, request: &str) -> Result<JsValue, JsValue> {
        let config: EngineConfig = from_json(config, "engine config")?;
        let request: tx::ShoutRequest = from_json(request, "shout request")?;
        to_js(&self.wallet.build_shout(&config, &request).map_err(js_err)?)
    }

    /// Builds a win proof for treasure payout `payoutIndex` of the round
    /// whose proof bundle (format 4, with payout openings) is `bundle`.
    /// Returns `{ id, proof }` with the proof base64url-encoded.
    #[wasm_bindgen(js_name = createWinProof)]
    pub fn create_win_proof(&self, bundle: &str, payout_index: u32) -> Result<JsValue, JsValue> {
        use base64::Engine;
        let bundle = bundle_from(bundle)?;
        let (id, encoded) = self
            .wallet
            .create_win_proof(&bundle, payout_index)
            .map_err(js_err)?;
        to_js(&serde_json::json!({
            "id": id,
            "proof": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(encoded),
            "round": bundle.round,
            "payoutIndex": payout_index,
        }))
    }

    /// The split this wallet would make now (`SplitPlan` JSON), or null.
    #[wasm_bindgen(js_name = splitPlan)]
    pub fn split_plan(&self) -> Result<JsValue, JsValue> {
        to_js(&self.wallet.split_plan().map_err(js_err)?)
    }

    /// Builds the split of `splitPlan` (a shielded self-send) at `chain_tip`.
    #[wasm_bindgen(js_name = buildSplit)]
    pub fn build_split(&mut self, chain_tip: u32) -> Result<JsValue, JsValue> {
        to_js(&self.wallet.build_split(chain_tip).map_err(js_err)?)
    }

    /// Builds a send: JSON `{ to, amountZat, memo?, chainTip }`.
    #[wasm_bindgen(js_name = buildSend)]
    pub fn build_send(&mut self, request: &str) -> Result<JsValue, JsValue> {
        let request: SendRequest = from_json(request, "send request")?;
        to_js(&self.wallet.build_send(&request).map_err(js_err)?)
    }

    #[wasm_bindgen(js_name = markBroadcast)]
    pub fn mark_broadcast(&mut self, txid: &str) -> Result<(), JsValue> {
        self.wallet.mark_broadcast(txid).map_err(js_err)
    }

    pub fn abandon(&mut self, txid: &str) -> Result<(), JsValue> {
        self.wallet.abandon(txid).map_err(js_err)
    }
}
