//! Network tests against a live Zcash **testnet** `lightwalletd`.
//!
//! These are ignored by default. Run them with:
//!
//! ```text
//! cp .env.example .env            # or export ZEEK_LIGHTWALLETD_URL=https://testnet.zec.rocks:443
//! cargo test -p zeek-vault --test network -- --ignored --nocapture
//! ```
//!
//! `vault_sync_from_tip_reports_zero_balance` creates a throw-away vault in a
//! temporary directory and syncs it from the current chain tip, so it finishes
//! in seconds and never touches `./.secrets`.
//!
//! `funded_vault_send` spends from the vault in `./.secrets` and needs:
//!
//! ```text
//! ZEEK_TEST_SEND_TO=<testnet unified address with an Orchard receiver>
//! ZEEK_TEST_SEND_AMOUNT=0.0001
//! cargo test -p zeek-vault --test network funded_vault_send -- --ignored --nocapture
//! ```

use std::env;

use rand_core::OsRng;
use zeek_vault::config::Config;
use zeek_vault::send::{self, SendRequest};
use zeek_vault::store::SecretsDir;
use zeek_vault::{frost, keys, lightwalletd, wallet};

fn lightwalletd_url() -> String {
    let _ = dotenvy::dotenv();
    env::var("ZEEK_LIGHTWALLETD_URL").expect("ZEEK_LIGHTWALLETD_URL must be set (see .env.example)")
}

#[tokio::test]
#[ignore = "needs network access to a testnet lightwalletd"]
async fn lightwalletd_serves_testnet() {
    let mut client = lightwalletd::connect(&lightwalletd_url())
        .await
        .expect("connect");
    let info = lightwalletd::lightd_info(&mut client).await.expect("info");
    assert_eq!(info.chain_name, "test");
    let tip = lightwalletd::latest_height(&mut client).await.expect("tip");
    assert!(u32::from(tip) > 4_465_026, "testnet is past NU7, got {tip}");
    assert_eq!(
        info.consensus_branch_id, "77190ad9",
        "lightwalletd must report the NU7 consensus branch"
    );
    let state = lightwalletd::tree_state(&mut client, tip)
        .await
        .expect("tree state");
    assert_eq!(state.height, u64::from(u32::from(tip)));
}

#[tokio::test]
#[ignore = "needs network access to a testnet lightwalletd"]
async fn vault_sync_from_tip_reports_zero_balance() {
    let url = lightwalletd_url();
    let tmp = tempfile::tempdir().expect("tempdir");

    let out = frost::dealer_keygen(2, 3, OsRng).expect("keygen");
    let group = frost::group_key_bytes(&out.public_key_package).expect("group key");
    let fvk = keys::derive_full_viewing_key(&group, OsRng).expect("fvk");
    let ufvk = keys::unified_full_viewing_key(&fvk).expect("ufvk");

    let mut client = lightwalletd::connect(&url).await.expect("connect");
    let mut db = wallet::open(tmp.path()).expect("open wallet");
    let (account, birthday) = wallet::ensure_account(&mut db, &mut client, &ufvk, None)
        .await
        .expect("import account");
    assert_eq!(
        wallet::find_account(&db, &ufvk).expect("find"),
        Some(account)
    );

    wallet::sync(&mut db, &mut client).await.expect("sync");
    let report = wallet::balance(&db, account).expect("balance");
    assert!(report.chain_tip >= u32::from(birthday));
    assert_eq!(report.ironwood.spendable.into_u64(), 0);
    assert_eq!(report.ironwood.total.into_u64(), 0);
    assert_eq!(report.orchard.total.into_u64(), 0);
    assert!(report.notes.is_empty());
}

#[tokio::test]
#[ignore = "spends real testnet funds from ./.secrets; see module docs"]
async fn funded_vault_send() {
    let _ = dotenvy::dotenv();
    let config = Config::from_env().expect("config");
    let to = env::var("ZEEK_TEST_SEND_TO").expect("ZEEK_TEST_SEND_TO");
    let amount = env::var("ZEEK_TEST_SEND_AMOUNT").unwrap_or_else(|_| "0.0001".to_owned());

    let secrets = SecretsDir::new(config.secrets_dir());
    let vault = secrets.load_vault().expect("vault.json");
    let pkp = vault.public_key_package().expect("public key package");
    let k1 = secrets.load_keyholder(1, &vault).expect("keyholder 1");
    let k3 = secrets.load_keyholder(3, &vault).expect("keyholder 3");

    let mut client = config.connect().await.expect("connect");
    let mut db = wallet::open(config.data_dir()).expect("open wallet");
    let account = wallet::find_account(&db, &vault.ufvk().expect("ufvk"))
        .expect("find")
        .expect("vault account must exist; run `zeek-vault sync` first");
    wallet::sync(&mut db, &mut client).await.expect("sync");

    let outcome = send::send(
        &mut db,
        &mut client,
        account,
        &pkp,
        &[&k1, &k3],
        &SendRequest {
            to,
            amount_zec: amount,
            memo: Some("zeek vault network test".to_owned()),
        },
    )
    .await
    .expect("send");
    println!(
        "broadcast {} ({:?} on {:?})",
        outcome.txid, outcome.version, outcome.branch
    );
    assert!(!outcome.signed_spends.is_empty());
    assert!(outcome
        .signed_spends
        .iter()
        .all(|s| s.pool == orchard::ValuePool::Ironwood));
}

#[tokio::test]
#[ignore = "needs network access to the public mainnet lightwalletd servers"]
async fn mainnet_pool_servers_answer_and_agree_on_a_block() {
    use zeek_core::params::Network;
    use zeek_vault::pool::{default_servers, Confirmation, Pool};
    let pool = Pool::new(Network::Main, default_servers(Network::Main), 0).expect("pool");
    pool.probe().await;
    for s in pool.status() {
        println!(
            "{} healthy={} tip={:?} latency_ms={:?} error={:?}",
            s.url, s.healthy, s.tip, s.latency_ms, s.last_error
        );
    }
    let healthy = pool.status().iter().filter(|s| s.healthy).count();
    assert!(healthy >= 3, "at least three mainnet servers answer");
    let tip = pool
        .unary(|mut c| async move { lightwalletd::latest_height(&mut c).await })
        .await
        .expect("tip");
    let height = u32::from(tip) - 10;
    let (block, source) = pool
        .call(|mut c| async move {
            Ok(c.get_block(zcash_client_backend::proto::service::BlockId {
                height: u64::from(height),
                hash: vec![],
            })
            .await?
            .into_inner())
        })
        .await
        .expect("block");
    let confirmation = pool
        .check_block(source, height, &block.hash)
        .await
        .expect("check");
    println!("block {height}: {confirmation:?}");
    assert!(matches!(confirmation, Confirmation::Agreed { servers } if servers >= 2));
    // A forged hash is outvoted by the honest majority.
    let forged = vec![0u8; 32];
    let outvoted = pool
        .check_block(source, height, &forged)
        .await
        .expect("check");
    assert!(matches!(outvoted, Confirmation::Outvoted { ref hash, .. } if *hash == block.hash));
}
