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
    assert!(
        u32::from(tip) > 4_134_000,
        "testnet is past NU6.3, got {tip}"
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
    assert_eq!(report.spendable.into_u64(), 0);
    assert_eq!(report.total.into_u64(), 0);
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

    let mut client = lightwalletd::connect(config.lightwalletd_url().expect("url"))
        .await
        .expect("connect");
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
    println!("broadcast {}", outcome.txid);
    assert!(outcome.signed_actions >= 1);
}
