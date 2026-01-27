//! ITP Chain Discovery Service
//!
//! Discovers ITPs created on-chain by scanning `ItpCreated` events from the
//! Arbitrum BridgeProxy contract, then reads vault metadata from Orbit chain.
//! This bridges the gap between bridge-deployed ITPs and the backend database.

use alloy::{
    eips::BlockNumberOrTag,
    primitives::{Address, FixedBytes, U256},
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::Filter,
    sol,
    transports::http::{Client, Http},
};
use asset_registry::AssetRegistry;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// ItpCreated event signature (keccak256)
/// ItpCreated(address indexed orbitItp, address indexed arbitrumBridgedItp, uint256 indexed nonce)
const ITP_CREATED_SIGNATURE: [u8; 32] = [
    0x65, 0x09, 0xcc, 0x83, 0x94, 0x3c, 0xe9, 0x51,
    0x72, 0xc5, 0x44, 0xa2, 0x96, 0x41, 0xa6, 0xb6,
    0x80, 0x1f, 0xab, 0x52, 0xff, 0xbd, 0x03, 0xbc,
    0x40, 0xed, 0x2e, 0x54, 0xde, 0x8d, 0x7f, 0x34,
];

// Vault contract interface on Orbit (read metadata)
sol! {
    #[sol(rpc)]
    interface IVaultMeta {
        function name() external view returns (string);
        function symbol() external view returns (string);
        function description() external view returns (string);
        function methodology() external view returns (string);
        function initialPrice() external view returns (uint128);
        function totalSupply() external view returns (uint256);
        function indexId() external view returns (uint128);
    }
}

// Steward interface on Orbit (read assets/weights via Castle proxy)
sol! {
    #[sol(rpc)]
    interface IStewardMeta {
        function getIndexAssetsCount(uint128 index_id) external view returns (uint128);
        function getIndexAssets(uint128 index_id) external view returns (bytes);
        function getIndexWeights(uint128 index_id) external view returns (bytes);
    }
}

/// Discovered ITP data from on-chain
#[derive(Debug, Clone)]
pub struct DiscoveredItp {
    pub orbit_address: String,
    pub arbitrum_address: String,
    pub name: String,
    pub symbol: String,
    pub description: String,
    pub methodology: String,
    /// Initial price in 18 decimals
    pub initial_price_18: u128,
    /// Total supply in 18 decimals (as U256 string)
    pub total_supply: String,
    /// On-chain index ID
    pub index_id: u128,
    /// Asset symbols (e.g., ["BTC", "ETH"])
    pub assets: Vec<String>,
    /// Weights as floats (0.0-1.0)
    pub weights: Vec<f64>,
    /// Transaction hash from the ItpCreated event
    pub tx_hash: String,
    /// Admin/deployer address (from field of the ItpCreated transaction)
    pub admin_address: Option<String>,
}

/// Error types for chain discovery
#[derive(Debug)]
pub enum ChainDiscoveryError {
    ProviderError(String),
    ContractCallError(String),
    InvalidConfig(String),
}

impl std::fmt::Display for ChainDiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainDiscoveryError::ProviderError(msg) => write!(f, "Provider error: {}", msg),
            ChainDiscoveryError::ContractCallError(msg) => write!(f, "Contract call error: {}", msg),
            ChainDiscoveryError::InvalidConfig(msg) => write!(f, "Invalid config: {}", msg),
        }
    }
}

impl std::error::Error for ChainDiscoveryError {}

/// ITP Chain Discovery Service
pub struct ItpChainDiscoveryService {
    arb_provider: RootProvider<Http<Client>>,
    orbit_provider: RootProvider<Http<Client>>,
    bridge_proxy_address: Address,
    castle_address: Address,
    asset_registry: Arc<AssetRegistry>,
    /// Block to start scanning from
    start_block: u64,
}

impl ItpChainDiscoveryService {
    /// Create a new discovery service
    pub async fn new(
        arb_rpc_url: &str,
        orbit_rpc_url: &str,
        bridge_proxy_address: &str,
        castle_address: &str,
        asset_registry: Arc<AssetRegistry>,
        start_block: u64,
    ) -> Result<Self, ChainDiscoveryError> {
        let arb_provider = ProviderBuilder::new()
            .on_http(arb_rpc_url.parse().map_err(|e| {
                ChainDiscoveryError::InvalidConfig(format!("Invalid Arbitrum RPC URL: {}", e))
            })?);

        let orbit_provider = ProviderBuilder::new()
            .on_http(orbit_rpc_url.parse().map_err(|e| {
                ChainDiscoveryError::InvalidConfig(format!("Invalid Orbit RPC URL: {}", e))
            })?);

        let bridge_proxy = Address::from_str(bridge_proxy_address).map_err(|e| {
            ChainDiscoveryError::InvalidConfig(format!("Invalid BridgeProxy address: {}", e))
        })?;

        let castle = Address::from_str(castle_address).map_err(|e| {
            ChainDiscoveryError::InvalidConfig(format!("Invalid Castle address: {}", e))
        })?;

        // Verify connections
        arb_provider.get_chain_id().await.map_err(|e| {
            ChainDiscoveryError::ProviderError(format!("Arbitrum connection failed: {}", e))
        })?;
        orbit_provider.get_chain_id().await.map_err(|e| {
            ChainDiscoveryError::ProviderError(format!("Orbit connection failed: {}", e))
        })?;

        Ok(Self {
            arb_provider,
            orbit_provider,
            bridge_proxy_address: bridge_proxy,
            castle_address: castle,
            asset_registry,
            start_block,
        })
    }

    /// Discover all ITPs by scanning ItpCreated events on Arbitrum
    pub async fn discover_all_itps(&self) -> Result<Vec<DiscoveredItp>, ChainDiscoveryError> {
        info!("Scanning ItpCreated events on Arbitrum BridgeProxy");

        // Get current block
        let current_block = self.arb_provider.get_block_number().await.map_err(|e| {
            ChainDiscoveryError::ProviderError(format!("Failed to get block number: {}", e))
        })?;

        // Build filter for ItpCreated events
        let event_sig = FixedBytes::from(ITP_CREATED_SIGNATURE);
        let filter = Filter::new()
            .address(self.bridge_proxy_address)
            .event_signature(event_sig)
            .from_block(BlockNumberOrTag::Number(self.start_block))
            .to_block(BlockNumberOrTag::Number(current_block));

        let logs = self.arb_provider.get_logs(&filter).await.map_err(|e| {
            ChainDiscoveryError::ProviderError(format!("Failed to get logs: {}", e))
        })?;

        info!(
            event_count = logs.len(),
            from_block = self.start_block,
            to_block = current_block,
            "Found ItpCreated events"
        );

        let mut discovered = Vec::new();

        for log in &logs {
            let topics = log.inner.topics();
            if topics.len() < 4 {
                warn!("ItpCreated log with insufficient topics, skipping");
                continue;
            }

            // Parse indexed params from topics
            // topic[0] = event signature
            // topic[1] = orbitItp (indexed address)
            // topic[2] = arbitrumBridgedItp (indexed address)
            // topic[3] = nonce (indexed uint256)
            let orbit_address = Address::from_slice(&topics[1][12..32]);
            let arbitrum_address = Address::from_slice(&topics[2][12..32]);
            let _nonce = U256::from_be_slice(&topics[3][..]);

            let tx_hash = log.transaction_hash
                .map(|h| format!("{:?}", h))
                .unwrap_or_default();

            let orbit_addr_str = format!("{:?}", orbit_address);
            let arb_addr_str = format!("{:?}", arbitrum_address);

            info!(
                orbit = %orbit_addr_str,
                arbitrum = %arb_addr_str,
                "Found ItpCreated event, reading vault metadata from Orbit"
            );

            // Extract admin address from the transaction sender
            let admin_address = if let Some(tx_hash_bytes) = log.transaction_hash {
                match self.arb_provider.get_transaction_by_hash(tx_hash_bytes).await {
                    Ok(Some(tx)) => Some(format!("{:?}", tx.from)),
                    Ok(None) => {
                        debug!("Transaction not found for ItpCreated event");
                        None
                    }
                    Err(e) => {
                        debug!(error = %e, "Failed to fetch transaction for admin address");
                        None
                    }
                }
            } else {
                None
            };

            // Read metadata from Orbit vault
            match self.read_vault_metadata(orbit_address).await {
                Ok(mut itp) => {
                    itp.arbitrum_address = arb_addr_str;
                    itp.tx_hash = tx_hash;
                    itp.admin_address = admin_address;
                    discovered.push(itp);
                }
                Err(e) => {
                    warn!(
                        orbit_address = %orbit_addr_str,
                        error = %e,
                        "Failed to read vault metadata, skipping"
                    );
                }
            }
        }

        info!(
            discovered = discovered.len(),
            "Chain discovery complete"
        );

        Ok(discovered)
    }

    /// Read vault metadata from Orbit chain
    async fn read_vault_metadata(
        &self,
        vault_address: Address,
    ) -> Result<DiscoveredItp, ChainDiscoveryError> {
        let vault = IVaultMeta::new(vault_address, &self.orbit_provider);

        // Read basic metadata
        let name = vault.name().call().await
            .map(|r| r._0)
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to read vault name");
                "Unknown".to_string()
            });

        let symbol = vault.symbol().call().await
            .map(|r| r._0)
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to read vault symbol");
                "???".to_string()
            });

        let description = vault.description().call().await
            .map(|r| r._0)
            .unwrap_or_default();

        let methodology = vault.methodology().call().await
            .map(|r| r._0)
            .unwrap_or_default();

        let initial_price_18 = vault.initialPrice().call().await
            .map(|r| r._0)
            .unwrap_or(0);

        let total_supply = vault.totalSupply().call().await
            .map(|r| r._0.to_string())
            .unwrap_or_else(|_| "0".to_string());

        let index_id = vault.indexId().call().await
            .map(|r| r._0)
            .unwrap_or(0);

        debug!(
            name = %name,
            symbol = %symbol,
            index_id = index_id,
            initial_price = initial_price_18,
            "Read vault basic metadata"
        );

        // Read assets and weights from Steward (Castle proxy)
        let (assets, weights) = self.read_assets_and_weights(index_id).await;

        Ok(DiscoveredItp {
            orbit_address: format!("{:?}", vault_address),
            arbitrum_address: String::new(), // Set by caller
            name,
            symbol,
            description,
            methodology,
            initial_price_18,
            total_supply,
            index_id,
            assets,
            weights,
            tx_hash: String::new(), // Set by caller
            admin_address: None, // Set by caller from tx sender
        })
    }

    /// Read assets and weights from Steward contract via Castle proxy
    async fn read_assets_and_weights(&self, index_id: u128) -> (Vec<String>, Vec<f64>) {
        let steward = IStewardMeta::new(self.castle_address, &self.orbit_provider);

        // Read raw asset bytes
        let asset_bytes = match steward.getIndexAssets(index_id).call().await {
            Ok(r) => r._0.to_vec(),
            Err(e) => {
                warn!(index_id = index_id, error = %e, "Failed to read index assets");
                return (vec![], vec![]);
            }
        };

        // Read raw weight bytes
        let weight_bytes = match steward.getIndexWeights(index_id).call().await {
            Ok(r) => r._0.to_vec(),
            Err(e) => {
                warn!(index_id = index_id, error = %e, "Failed to read index weights");
                return (vec![], vec![]);
            }
        };

        // Decode asset IDs (LE u128, 16 bytes each)
        let asset_ids = decode_u128_le_array(&asset_bytes);
        // Decode weight values (LE u128, 16 bytes each)
        let raw_weights = decode_u128_le_array(&weight_bytes);

        // Map asset IDs to symbols via registry
        let assets: Vec<String> = asset_ids.iter().map(|id| {
            match self.asset_registry.by_id(*id) {
                Some(asset) => {
                    // Derive symbol from bitget field (e.g., "BTCUSDC" -> "BTC")
                    let bitget = &asset.bitget;
                    if bitget.ends_with("USDC") {
                        bitget.trim_end_matches("USDC").to_string()
                    } else if bitget.ends_with("USDT") {
                        bitget.trim_end_matches("USDT").to_string()
                    } else {
                        bitget.clone()
                    }
                }
                None => {
                    warn!(asset_id = id, "Unknown asset ID in registry, using ID as placeholder");
                    format!("ASSET_{}", id)
                }
            }
        }).collect();

        // Convert weights: on-chain weights are in 18-decimal (1e18 = 100%)
        // Convert to basis-point floats (0.0-1.0)
        let weights: Vec<f64> = raw_weights.iter().map(|w| {
            *w as f64 / 1e18
        }).collect();

        debug!(
            index_id = index_id,
            assets = ?assets,
            weights = ?weights,
            "Decoded assets and weights from chain"
        );

        (assets, weights)
    }
}

/// Decode a byte array as a sequence of little-endian u128 values (Labels format)
fn decode_u128_le_array(data: &[u8]) -> Vec<u128> {
    data.chunks_exact(16)
        .map(|chunk| {
            let bytes: [u8; 16] = chunk.try_into().unwrap();
            u128::from_le_bytes(bytes)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_u128_le_array_empty() {
        let result = decode_u128_le_array(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_decode_u128_le_array_single() {
        let val: u128 = 42;
        let bytes = val.to_le_bytes();
        let result = decode_u128_le_array(&bytes);
        assert_eq!(result, vec![42u128]);
    }

    #[test]
    fn test_decode_u128_le_array_multiple() {
        let mut data = Vec::new();
        data.extend_from_slice(&100u128.to_le_bytes());
        data.extend_from_slice(&200u128.to_le_bytes());
        data.extend_from_slice(&300u128.to_le_bytes());
        let result = decode_u128_le_array(&data);
        assert_eq!(result, vec![100u128, 200, 300]);
    }

    #[test]
    fn test_itp_created_signature() {
        assert_eq!(ITP_CREATED_SIGNATURE[0], 0x65);
        assert_eq!(ITP_CREATED_SIGNATURE[31], 0x34);
    }
}
