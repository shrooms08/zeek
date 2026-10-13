//! Synthetic chains for tests: a throw-away vault key, real Ironwood note
//! encryption to it, V6 transactions and compact blocks built without a
//! network, and a [`ChainSource`] over them.
//!
//! The transactions carry no valid proofs or signatures (the engine never
//! verifies them; the chain does), but their actions, ciphertexts and txids
//! are exactly what the engine sees on the real chain.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use orchard::bundle::{Authorized, BundleVersion};
use orchard::keys::{FullViewingKey, Scope, SpendingKey};
use orchard::note_encryption::IronwoodDomain;
use orchard::primitives::redpallas;
use orchard::{Anchor, Proof};
use rand_08::rngs::StdRng as StdRng08;
use rand_08::{RngCore as RngCore08, SeedableRng as SeedableRng08};
use rand_09::rngs::StdRng;
use rand_09::SeedableRng;
use zcash_client_backend::proto::compact_formats::{CompactBlock, CompactOrchardAction, CompactTx};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_note_encryption::{ShieldedOutput, ENC_CIPHERTEXT_SIZE};
use zcash_primitives::transaction::{TransactionData, TxVersion};
use zcash_protocol::consensus::{BlockHeight, BranchId};
use zcash_protocol::value::ZatBalance;
use zeek_core::memo::{DigMemo, PayoutReceiver, ORCHARD_RECEIVER_LEN};
use zeek_core::params::{GameParams, Network};
use zeek_core::seal::{generate_sealing_keypair, SealingPrivateKey, SealingPublicKey};

use crate::chain::ChainSource;
use crate::config::GameConfig;
use crate::db::Pool;
use crate::error::EngineError;

/// A vault whose keys live only in memory.
pub struct TestVault {
    pub fvk: FullViewingKey,
    pub ufvk: UnifiedFullViewingKey,
    pub sealing_private: SealingPrivateKey,
    pub sealing_public: SealingPublicKey,
}

impl TestVault {
    /// Deterministic vault from `seed`.
    pub fn new(seed: u64) -> Self {
        let mut key_rng = StdRng08::seed_from_u64(seed);
        let sk = loop {
            let mut bytes = [0u8; 32];
            key_rng.fill_bytes(&mut bytes);
            if let Some(sk) = Option::<SpendingKey>::from(SpendingKey::from_bytes(bytes)) {
                break sk;
            }
        };
        let mut rng = StdRng::seed_from_u64(seed);
        let fvk = FullViewingKey::from(&sk);
        let ufvk = UnifiedFullViewingKey::from_orchard_fvk(fvk.clone())
            .expect("an Orchard-only UFVK is always valid");
        let (sealing_private, sealing_public) = generate_sealing_keypair(&mut rng);
        TestVault {
            fvk,
            ufvk,
            sealing_private,
            sealing_public,
        }
    }

    /// The external payment address.
    pub fn address(&self) -> orchard::Address {
        self.fvk.address_at(0u32, Scope::External)
    }

    /// Testnet game config with round 0 starting at `genesis_height`.
    pub fn config(&self, genesis_height: u32) -> GameConfig {
        let receiver = PayoutReceiver {
            network: Network::Test,
            orchard: self.address().to_raw_address_bytes(),
        };
        GameConfig {
            params: GameParams::testnet(genesis_height),
            vault_address: receiver
                .to_unified_address()
                .expect("an Orchard receiver always encodes"),
            sealing_public_key: self.sealing_public.clone(),
            uivk: self
                .ufvk
                .to_unified_incoming_viewing_key()
                .encode(&zcash_protocol::consensus::TEST_NETWORK),
            genesis_height,
            initial_cache_zat: 0,
        }
    }

    /// A dig memo for `round`/`tile` whose payout receiver is derived from
    /// `player` (a stand-in for the player's own address).
    pub fn dig_memo(&self, round: u32, tile: u8, player: u8) -> DigMemo {
        let receiver = PayoutReceiver {
            network: Network::Test,
            orchard: [player; ORCHARD_RECEIVER_LEN],
        };
        let mut rng = StdRng::seed_from_u64(
            u64::from(round) << 16 | u64::from(tile) << 8 | u64::from(player),
        );
        DigMemo::seal(round, tile, &receiver, &self.sealing_public, &mut rng)
            .expect("tile is in range")
    }
}

/// One output to put in a synthetic transaction.
#[derive(Clone)]
pub struct TxSpec {
    recipient: orchard::Address,
    value: u64,
    pool: Pool,
    memo: [u8; 512],
}

/// A block chain the engine can follow, with reorgs on demand.
pub struct SyntheticChain {
    next_height: u32,
    next_time: u64,
    time_step: u64,
    blocks: BTreeMap<u32, CompactBlock>,
    transactions: HashMap<[u8; 32], Vec<u8>>,
    nonce: u64,
    rng: StdRng08,
    vault_fvk: FullViewingKey,
    vault_external: orchard::Address,
    vault_internal: orchard::Address,
}

impl SyntheticChain {
    /// A chain whose first block will be `start_height` at `start_time`,
    /// with 25-second blocks.
    pub fn new(vault: &TestVault, start_height: u32, start_time: u64) -> Self {
        SyntheticChain {
            next_height: start_height,
            next_time: start_time,
            time_step: 25,
            blocks: BTreeMap::new(),
            transactions: HashMap::new(),
            nonce: 0,
            rng: StdRng08::seed_from_u64(start_height.into()),
            vault_fvk: vault.fvk.clone(),
            vault_external: vault.address(),
            vault_internal: vault.fvk.address_at(0u32, Scope::Internal),
        }
    }

    /// Seconds between blocks from now on.
    pub fn set_time_step(&mut self, secs: u64) {
        self.time_step = secs;
    }

    /// Delays the next block by `secs` beyond the regular step.
    pub fn delay(&mut self, secs: u64) {
        self.next_time += secs;
    }

    /// The next block's height.
    pub fn next_height(&self) -> u32 {
        self.next_height
    }

    /// A payment to the vault's external address.
    pub fn payment(&self, value: u64, pool: Pool, memo: [u8; 512]) -> TxSpec {
        TxSpec {
            recipient: self.vault_external,
            value,
            pool,
            memo,
        }
    }

    /// Change to the vault's internal address (invisible to the external IVK).
    pub fn change(&self, value: u64) -> TxSpec {
        TxSpec {
            recipient: self.vault_internal,
            value,
            pool: Pool::Ironwood,
            memo: [0xF6; 512],
        }
    }

    /// Appends `n` empty blocks.
    pub fn advance(&mut self, n: u32) {
        for _ in 0..n {
            self.push_block(vec![]);
        }
    }

    /// Appends a block containing one transaction per spec; returns its height.
    pub fn push_block(&mut self, specs: Vec<TxSpec>) -> u32 {
        let height = self.next_height;
        let prev_hash = self
            .blocks
            .get(&(height.wrapping_sub(1)))
            .map(|b| b.hash.clone())
            .unwrap_or_else(|| vec![0u8; 32]);
        let mut vtx = Vec::new();
        for (index, spec) in specs.into_iter().enumerate() {
            let (txid, raw, compact) = self.build_transaction(height, &spec);
            self.transactions.insert(txid, raw);
            vtx.push(CompactTx {
                index: index as u64,
                txid: txid.to_vec(),
                ..compact
            });
        }
        self.nonce += 1;
        let mut hash = vec![0u8; 32];
        hash[..8].copy_from_slice(&self.nonce.to_le_bytes());
        hash[8..12].copy_from_slice(&height.to_le_bytes());
        self.rng.fill_bytes(&mut hash[12..]);
        let time = u32::try_from(self.next_time).expect("test times fit u32");
        self.blocks.insert(
            height,
            CompactBlock {
                height: u64::from(height),
                hash,
                prev_hash,
                time,
                header: vec![],
                vtx,
                chain_metadata: None,
            },
        );
        self.next_height = height + 1;
        self.next_time += self.time_step;
        height
    }

    /// Forgets every block at or above `height` so a fork can be built.
    pub fn truncate(&mut self, height: u32) {
        self.blocks.retain(|h, _| *h < height);
        self.next_height = height;
    }

    /// The compact block at `height`.
    pub fn block(&self, height: u32) -> Option<&CompactBlock> {
        self.blocks.get(&height)
    }

    /// Raw transaction bytes by txid.
    pub fn transaction(&self, txid: &[u8]) -> Option<Vec<u8>> {
        <[u8; 32]>::try_from(txid)
            .ok()
            .and_then(|id| self.transactions.get(&id).cloned())
    }

    fn build_transaction(&mut self, height: u32, spec: &TxSpec) -> ([u8; 32], Vec<u8>, CompactTx) {
        // Post-NU6.3 every pool's bundle is version 3; the Orchard pool forbids
        // cross-address outputs, so its only representable output is a change
        // output to an address the vault's key owns (paired by the builder with
        // a wallet-controlled zero-value spend).
        let bundle_version = match spec.pool {
            Pool::Ironwood => BundleVersion::ironwood_v3(),
            Pool::Orchard => BundleVersion::orchard_v3(),
        };
        let mut builder = orchard::builder::Builder::new(
            orchard::builder::BundleType::DEFAULT,
            bundle_version,
            bundle_version.default_flags(),
            Anchor::empty_tree(),
        )
        .expect("default flags are representable");
        let value = orchard::value::NoteValue::from_raw(spec.value);
        match spec.pool {
            Pool::Ironwood => builder
                .add_output(None, spec.recipient, value, spec.memo)
                .expect("output"),
            Pool::Orchard => builder
                .add_change_output(
                    self.vault_fvk.clone(),
                    None,
                    spec.recipient,
                    value,
                    spec.memo,
                )
                .expect("change output to an owned address"),
        }
        let (bundle, _meta) = builder
            .build::<ZatBalance>(&mut self.rng)
            .expect("build")
            .expect("one output makes a bundle");
        let proof_len = Proof::expected_proof_size(bundle.actions().len());
        // Proofs and signatures are placeholders of the right shape: the
        // engine parses transactions, it does not validate them.
        let bundle: orchard::Bundle<Authorized, ZatBalance> = bundle.map_authorization(
            &mut self.rng,
            |_, _, _| redpallas::Signature::from([0u8; 64]),
            |_, _| {
                Authorized::from_parts(
                    Proof::new(vec![0u8; proof_len]),
                    redpallas::Signature::from([0u8; 64]),
                )
            },
        );
        let (orchard_bundle, ironwood_bundle) = match spec.pool {
            Pool::Ironwood => (None, Some(bundle)),
            Pool::Orchard => (Some(bundle), None),
        };
        let data = TransactionData::<zcash_primitives::transaction::Authorized>::from_parts_v6(
            BranchId::Nu6_3,
            0,
            BlockHeight::from_u32(height + 40),
            None,
            None,
            orchard_bundle,
            ironwood_bundle,
        );
        let tx = data.freeze().expect("freeze");
        assert_eq!(tx.version(), TxVersion::V6);
        let mut raw = Vec::new();
        tx.write(&mut raw).expect("serialize");
        let txid: [u8; 32] = *tx.txid().as_ref();

        let bundle = match spec.pool {
            Pool::Ironwood => tx.ironwood_bundle(),
            Pool::Orchard => tx.orchard_bundle(),
        }
        .expect("bundle present");
        let actions: Vec<CompactOrchardAction> = bundle
            .actions()
            .iter()
            .map(|action| CompactOrchardAction {
                nullifier: action.nullifier().to_bytes().to_vec(),
                cmx: action.cmx().to_bytes().to_vec(),
                ephemeral_key:
                    ShieldedOutput::<IronwoodDomain, ENC_CIPHERTEXT_SIZE>::ephemeral_key(action)
                        .0
                        .to_vec(),
                ciphertext: ShieldedOutput::<IronwoodDomain, ENC_CIPHERTEXT_SIZE>::enc_ciphertext(
                    action,
                )[..52]
                    .to_vec(),
            })
            .collect();
        let compact = match spec.pool {
            Pool::Ironwood => CompactTx {
                ironwood_actions: actions,
                ..CompactTx::default()
            },
            Pool::Orchard => CompactTx {
                actions,
                ..CompactTx::default()
            },
        };
        (txid, raw, compact)
    }
}

#[async_trait]
impl ChainSource for SyntheticChain {
    async fn latest_height(&mut self) -> Result<u32, EngineError> {
        self.blocks
            .keys()
            .next_back()
            .copied()
            .ok_or_else(|| EngineError::Server("synthetic chain is empty".into()))
    }

    async fn block_range(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<Vec<CompactBlock>, EngineError> {
        Ok(self
            .blocks
            .range(start..=end)
            .map(|(_, b)| b.clone())
            .collect())
    }

    async fn transaction(&mut self, txid: &[u8; 32]) -> Result<Vec<u8>, EngineError> {
        self.transactions
            .get(txid)
            .cloned()
            .ok_or_else(|| EngineError::Server("unknown synthetic transaction".into()))
    }
}
