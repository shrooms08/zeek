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

/// Builds the any-wallet dig request: JSON
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
        let envelope: SeedEnvelope = from_json(seed_envelope, "seed envelope")?;
        let entropy = crypto::open_seed(&envelope, passphrase).map_err(js_err)?;
        if *entropy != *self.wallet.keys().entropy() {
            return Err(js_err(PocketError::BadPassphrase));
        }
        Ok(self.wallet.keys().phrase().map_err(js_err)?.to_string())
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
