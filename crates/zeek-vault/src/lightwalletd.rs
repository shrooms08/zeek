//! Minimal `lightwalletd` gRPC client helpers.

use std::time::Duration;

use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use zcash_client_backend::proto::service::compact_tx_streamer_client::CompactTxStreamerClient;
use zcash_client_backend::proto::service::{
    BlockId, ChainSpec, Empty, LightdInfo, RawTransaction, TreeState,
};
use zcash_protocol::consensus::{BlockHeight, BranchId};

use crate::error::VaultError;

/// The gRPC client type used throughout the crate.
pub type Client = CompactTxStreamerClient<Channel>;

/// Chain name `lightwalletd` reports for testnet.
pub const TESTNET_CHAIN_NAME: &str = "test";

/// Testnet NU7 activation height (ZIP 259). Kept as a literal so the check
/// below works even in a build whose `zcash_protocol` does not know NU7.
pub const NU7_TESTNET_ACTIVATION: u32 = 4_465_026;

/// Whether this build was compiled with the `zcash_unstable="nu7"` cfg that
/// the pinned librustzcash crates need to speak NU7 (see `.cargo/config.toml`).
pub const fn nu7_supported() -> bool {
    cfg!(zcash_unstable = "nu7")
}

/// Refuses to run against a chain this build cannot follow.
///
/// `supported` is [`nu7_supported`] (a parameter so the failure path is
/// testable), `tip` the server's chain height and `branch_hex` the consensus
/// branch id the server reports. A tip at or past NU7 activation without NU7
/// support, or a branch id this build does not recognise, means every
/// transaction built here would be rejected by the network.
pub fn check_consensus_support(
    supported: bool,
    tip: u32,
    branch_hex: &str,
) -> Result<(), VaultError> {
    if !supported && tip >= NU7_TESTNET_ACTIVATION {
        return Err(VaultError::Nu7Unsupported {
            tip,
            activation: NU7_TESTNET_ACTIVATION,
        });
    }
    let branch = u32::from_str_radix(branch_hex.trim_start_matches("0x"), 16)
        .map_err(|_| VaultError::UnknownConsensusBranch(branch_hex.to_owned()))?;
    BranchId::try_from(branch)
        .map(|_| ())
        .map_err(|_| VaultError::UnknownConsensusBranch(branch_hex.to_owned()))
}

/// Connects to `url` (http or https) and checks that it serves testnet.
pub async fn connect(url: &str) -> Result<Client, VaultError> {
    // Without these a dead connection hangs a call forever (seen live: the
    // engine's pass stalled for an hour holding its database lock). The
    // request timeout (5 min: a wallet's first sync can wait that long for a
    // big block range to start) covers unary calls and the start of a
    // stream; the
    // HTTP/2 keep-alives end a stream whose connection died mid-way. They
    // ping no more often than every 5 minutes and only during calls:
    // lightwalletd (grpc-go) answers more frequent or idle pings with
    // GOAWAY "too_many_pings".
    let mut endpoint = Endpoint::from_shared(url.to_owned())?
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .http2_keep_alive_interval(Duration::from_secs(300))
        .keep_alive_timeout(Duration::from_secs(20))
        .keep_alive_while_idle(false);
    if url.starts_with("https://") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new().with_webpki_roots())?;
    } else if !url.starts_with("http://") {
        return Err(VaultError::Config(format!(
            "lightwalletd URL must start with http:// or https://, got `{url}`"
        )));
    }
    let channel = endpoint.connect().await?;
    let mut client = CompactTxStreamerClient::new(channel);
    let info = lightd_info(&mut client).await?;
    if info.chain_name != TESTNET_CHAIN_NAME {
        return Err(VaultError::WrongNetwork(info.chain_name));
    }
    let tip = u32::try_from(info.block_height).map_err(|_| {
        VaultError::BadResponse(format!("chain height {} overflows u32", info.block_height))
    })?;
    check_consensus_support(nu7_supported(), tip, &info.consensus_branch_id)?;
    Ok(client)
}

/// Fetches server information.
pub async fn lightd_info(client: &mut Client) -> Result<LightdInfo, VaultError> {
    Ok(client.get_lightd_info(Empty {}).await?.into_inner())
}

/// Fetches the current chain tip height.
pub async fn latest_height(client: &mut Client) -> Result<BlockHeight, VaultError> {
    let block = client.get_latest_block(ChainSpec {}).await?.into_inner();
    u32::try_from(block.height)
        .map(BlockHeight::from_u32)
        .map_err(|_| {
            VaultError::BadResponse(format!("chain tip height {} overflows u32", block.height))
        })
}

/// Fetches the note commitment tree state as of `height`.
pub async fn tree_state(client: &mut Client, height: BlockHeight) -> Result<TreeState, VaultError> {
    let request = BlockId {
        height: u64::from(u32::from(height)),
        hash: vec![],
    };
    Ok(client.get_tree_state(request).await?.into_inner())
}

/// Broadcasts a serialized transaction and fails if the server rejects it.
pub async fn broadcast(client: &mut Client, tx_bytes: Vec<u8>) -> Result<(), VaultError> {
    let response = client
        .send_transaction(RawTransaction {
            data: tx_bytes,
            height: 0,
        })
        .await?
        .into_inner();
    if response.error_code != 0 {
        return Err(VaultError::BroadcastRejected {
            code: response.error_code,
            message: response.error_message,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NETWORK;
    use zcash_protocol::consensus::{NetworkUpgrade, Parameters};

    #[test]
    fn this_build_supports_nu7_and_the_literal_matches_the_library() {
        assert!(
            nu7_supported(),
            "build without --cfg zcash_unstable=\"nu7\""
        );
        assert_eq!(
            NETWORK.activation_height(NetworkUpgrade::Nu7),
            Some(BlockHeight::from_u32(NU7_TESTNET_ACTIVATION))
        );
        check_consensus_support(true, 4_500_000, "77190ad9").expect("supported");
        check_consensus_support(false, NU7_TESTNET_ACTIVATION - 1, "37a5165b").expect("pre-NU7 ok");
    }

    #[test]
    fn refuses_nu7_chains_without_support_and_unknown_branches() {
        assert!(matches!(
            check_consensus_support(false, NU7_TESTNET_ACTIVATION, "77190ad9"),
            Err(VaultError::Nu7Unsupported {
                tip: NU7_TESTNET_ACTIVATION,
                activation: NU7_TESTNET_ACTIVATION
            })
        ));
        assert!(matches!(
            check_consensus_support(true, 4_500_000, "deadbeef"),
            Err(VaultError::UnknownConsensusBranch(_))
        ));
        assert!(matches!(
            check_consensus_support(true, 4_500_000, "not-hex"),
            Err(VaultError::UnknownConsensusBranch(_))
        ));
    }
}
