//! Minimal `lightwalletd` gRPC client helpers.

use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use zcash_client_backend::proto::service::compact_tx_streamer_client::CompactTxStreamerClient;
use zcash_client_backend::proto::service::{
    BlockId, ChainSpec, Empty, LightdInfo, RawTransaction, TreeState,
};
use zcash_protocol::consensus::BlockHeight;

use crate::error::VaultError;

/// The gRPC client type used throughout the crate.
pub type Client = CompactTxStreamerClient<Channel>;

/// Chain name `lightwalletd` reports for testnet.
pub const TESTNET_CHAIN_NAME: &str = "test";

/// Connects to `url` (http or https) and checks that it serves testnet.
pub async fn connect(url: &str) -> Result<Client, VaultError> {
    let mut endpoint = Endpoint::from_shared(url.to_owned())?;
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
