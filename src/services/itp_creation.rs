//! ITP Creation Service for Arbitrum BridgeProxy interactions
//!
//! Handles creating ITPs via BridgeProxy.requestCreateItp() on Arbitrum
//! and optionally waiting for ItpCreated event confirmation.

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder, RootProvider},
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolEvent,
    transports::http::{Client, Http},
};
use std::str::FromStr;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Default gas limit for requestCreateItp (increased for new parameters)
const DEFAULT_GAS_LIMIT: u64 = 500_000;

/// Polling interval for sync mode (ms)
const POLL_INTERVAL_MS: u64 = 2000;

/// Timeout for sync mode (ms)
const SYNC_TIMEOUT_MS: u64 = 60_000;

/// Arbitrum chain ID
const ARBITRUM_CHAIN_ID: u64 = 42161;

// Define BridgeProxy contract interface (v2 with full parameters)
sol! {
    #[sol(rpc)]
    interface IBridgeProxy {
        function requestCreateItp(
            string calldata name,
            string calldata symbol,
            string calldata description,
            string calldata methodology,
            uint256 initialPrice,
            uint128 maxOrderSize,
            uint128[] calldata assets,
            uint128[] calldata weights
        ) external;

        event CreateItpRequested(
            address indexed admin,
            string name,
            string symbol,
            string description,
            string methodology,
            uint256 initialPrice,
            uint128 maxOrderSize,
            uint128[] assets,
            uint128[] weights,
            uint256 indexed nonce
        );

        event ItpCreated(
            address indexed orbitItp,
            address indexed arbitrumBridgedItp,
            uint256 indexed nonce
        );
    }
}

/// Result of ITP creation request (async mode)
#[derive(Debug, Clone)]
pub struct ItpCreationResult {
    pub tx_hash: String,
    pub nonce: u64,
    /// Block number when the transaction was confirmed (used to avoid finding stale events)
    pub confirmed_at_block: u64,
}

/// Result of ITP creation with addresses (sync mode)
#[derive(Debug, Clone)]
pub struct ItpCreationSyncResult {
    pub tx_hash: String,
    pub nonce: u64,
    pub orbit_address: String,
    pub arbitrum_address: String,
}

/// Error types for ITP creation
#[derive(Debug)]
pub enum ItpCreationError {
    ProviderError(String),
    TransactionError(String),
    /// Gas estimation failure - currently unused as estimate_gas_with_fallback
    /// uses a fallback value for resilience, but kept for error mapping completeness
    /// and potential future strict mode.
    #[allow(dead_code)]
    GasEstimationError(String),
    EventParsingError(String),
    Timeout(String),
    InvalidConfig(String),
}

impl std::fmt::Display for ItpCreationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItpCreationError::ProviderError(msg) => write!(f, "Provider error: {}", msg),
            ItpCreationError::TransactionError(msg) => write!(f, "Transaction error: {}", msg),
            ItpCreationError::GasEstimationError(msg) => write!(f, "Gas estimation error: {}", msg),
            ItpCreationError::EventParsingError(msg) => write!(f, "Event parsing error: {}", msg),
            ItpCreationError::Timeout(msg) => write!(f, "Timeout: {}", msg),
            ItpCreationError::InvalidConfig(msg) => write!(f, "Invalid config: {}", msg),
        }
    }
}

impl std::error::Error for ItpCreationError {}

/// ITP Creation Service
pub struct ItpCreationService {
    provider: RootProvider<Http<Client>>,
    wallet: EthereumWallet,
    bridge_proxy_address: Address,
}

impl ItpCreationService {
    /// Create a new ItpCreationService
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - Arbitrum RPC URL
    /// * `private_key` - Private key for signing transactions (hex string with 0x prefix)
    /// * `bridge_proxy_address` - BridgeProxy contract address
    ///
    /// # Errors
    ///
    /// Returns error if configuration is invalid
    pub async fn new(
        rpc_url: &str,
        private_key: &str,
        bridge_proxy_address: &str,
    ) -> Result<Self, ItpCreationError> {
        info!(
            rpc_url = %rpc_url,
            bridge_proxy = %bridge_proxy_address,
            "Initializing ItpCreationService"
        );

        // Parse private key
        let signer: PrivateKeySigner = private_key
            .parse()
            .map_err(|e| ItpCreationError::InvalidConfig(format!("Invalid private key: {}", e)))?;

        let wallet = EthereumWallet::from(signer);

        // Create provider
        let provider = ProviderBuilder::new()
            .on_http(rpc_url.parse().map_err(|e| {
                ItpCreationError::InvalidConfig(format!("Invalid RPC URL: {}", e))
            })?);

        // Verify connection
        let chain_id = provider.get_chain_id().await.map_err(|e| {
            error!(error = %e, "Failed to connect to Arbitrum RPC");
            ItpCreationError::ProviderError(format!("Connection failed: {}", e))
        })?;

        if chain_id != ARBITRUM_CHAIN_ID {
            warn!(
                expected = ARBITRUM_CHAIN_ID,
                actual = chain_id,
                "Chain ID mismatch - expected Arbitrum"
            );
        }

        let bridge_proxy = Address::from_str(bridge_proxy_address).map_err(|e| {
            ItpCreationError::InvalidConfig(format!("Invalid BridgeProxy address: {}", e))
        })?;

        info!(
            chain_id = chain_id,
            bridge_proxy = %bridge_proxy,
            "ItpCreationService initialized successfully"
        );

        Ok(Self {
            provider,
            wallet,
            bridge_proxy_address: bridge_proxy,
        })
    }

    /// Request ITP creation via BridgeProxy (async mode)
    ///
    /// # Arguments
    ///
    /// * `name` - ITP name
    /// * `symbol` - ITP symbol
    /// * `description` - ITP description
    /// * `methodology` - ITP methodology
    /// * `initial_price` - Initial price in USDC (6 decimals)
    /// * `max_order_size` - Maximum order size
    /// * `assets` - Array of asset IDs
    /// * `weights` - Array of weights corresponding to assets
    ///
    /// # Returns
    ///
    /// Transaction hash and nonce from CreateItpRequested event
    pub async fn request_create_itp(
        &self,
        name: &str,
        symbol: &str,
        description: &str,
        methodology: &str,
        initial_price: u64,
        max_order_size: u128,
        assets: Vec<u128>,
        weights: Vec<u128>,
    ) -> Result<ItpCreationResult, ItpCreationError> {
        info!(
            name = %name,
            symbol = %symbol,
            description = %description,
            initial_price = initial_price,
            max_order_size = max_order_size,
            num_assets = assets.len(),
            "Requesting ITP creation"
        );

        // Get wallet address (used for logging)
        let _wallet_address = self.wallet.default_signer().address();

        // Estimate gas with fallback
        let gas_limit = self
            .estimate_gas_with_fallback(name, symbol, description, methodology, initial_price, max_order_size, &assets, &weights)
            .await?;

        debug!(gas_limit = gas_limit, "Gas estimation complete");

        // Build the provider with the wallet for signing
        // Use ARB_RPC_URL from environment - fail explicitly if not configured
        let rpc_url = std::env::var("ARB_RPC_URL")
            .map_err(|_| ItpCreationError::InvalidConfig("ARB_RPC_URL not configured".to_string()))?;

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(self.wallet.clone())
            .on_http(
                rpc_url
                    .parse()
                    .map_err(|e| ItpCreationError::ProviderError(format!("RPC URL error: {}", e)))?,
            );

        // Create contract instance
        let bridge_proxy = IBridgeProxy::new(self.bridge_proxy_address, &provider);

        // Send transaction
        let price_u256 = U256::from(initial_price);

        let tx_builder = bridge_proxy
            .requestCreateItp(
                name.to_string(),
                symbol.to_string(),
                description.to_string(),
                methodology.to_string(),
                price_u256,
                max_order_size,
                assets.clone(),
                weights.clone(),
            )
            .gas(gas_limit);

        let pending_tx = tx_builder.send().await.map_err(|e| {
            error!(error = %e, "Failed to send requestCreateItp transaction");
            ItpCreationError::TransactionError(format!("Send failed: {}", e))
        })?;

        let tx_hash = format!("{:?}", pending_tx.tx_hash());
        info!(tx_hash = %tx_hash, "Transaction sent, waiting for confirmation");

        // Wait for receipt
        let receipt = pending_tx.get_receipt().await.map_err(|e| {
            error!(error = %e, "Failed to get transaction receipt");
            ItpCreationError::TransactionError(format!("Receipt failed: {}", e))
        })?;

        if !receipt.status() {
            return Err(ItpCreationError::TransactionError(
                "Transaction reverted".to_string(),
            ));
        }

        // Parse CreateItpRequested event from logs to get nonce
        let nonce = self.parse_create_itp_requested_event(receipt.inner.logs())?;

        // Get the block number where the transaction was confirmed
        // This prevents finding stale ItpCreated events from previous attempts
        let confirmed_at_block = receipt.block_number.unwrap_or(0);

        info!(
            tx_hash = %tx_hash,
            nonce = nonce,
            confirmed_at_block = confirmed_at_block,
            "ITP creation requested successfully"
        );

        Ok(ItpCreationResult { tx_hash, nonce, confirmed_at_block })
    }

    /// Request ITP creation and wait for completion (sync mode)
    ///
    /// # Arguments
    ///
    /// * `name` - ITP name
    /// * `symbol` - ITP symbol
    /// * `description` - ITP description
    /// * `methodology` - ITP methodology
    /// * `initial_price` - Initial price in USDC (6 decimals)
    /// * `max_order_size` - Maximum order size
    /// * `assets` - Array of asset IDs
    /// * `weights` - Array of weights corresponding to assets
    ///
    /// # Returns
    ///
    /// Full creation result with both Orbit and Arbitrum addresses
    pub async fn request_create_itp_sync(
        &self,
        name: &str,
        symbol: &str,
        description: &str,
        methodology: &str,
        initial_price: u64,
        max_order_size: u128,
        assets: Vec<u128>,
        weights: Vec<u128>,
    ) -> Result<ItpCreationSyncResult, ItpCreationError> {
        // First, send the creation request
        let result = self.request_create_itp(name, symbol, description, methodology, initial_price, max_order_size, assets, weights).await?;

        info!(
            tx_hash = %result.tx_hash,
            nonce = result.nonce,
            confirmed_at_block = result.confirmed_at_block,
            "Waiting for ITP creation completion"
        );

        // Poll for ItpCreated event - only look for events AFTER the request was confirmed
        // This prevents returning stale events from previous attempts with the same nonce
        let (orbit_address, arbitrum_address) =
            self.wait_for_itp_creation(result.nonce, result.confirmed_at_block).await?;

        info!(
            orbit_address = %orbit_address,
            arbitrum_address = %arbitrum_address,
            "ITP creation completed"
        );

        Ok(ItpCreationSyncResult {
            tx_hash: result.tx_hash,
            nonce: result.nonce,
            orbit_address,
            arbitrum_address,
        })
    }

    /// Estimate gas for requestCreateItp with fallback
    async fn estimate_gas_with_fallback(
        &self,
        name: &str,
        symbol: &str,
        description: &str,
        methodology: &str,
        initial_price: u64,
        max_order_size: u128,
        assets: &[u128],
        weights: &[u128],
    ) -> Result<u64, ItpCreationError> {
        let bridge_proxy = IBridgeProxy::new(self.bridge_proxy_address, &self.provider);
        let price_u256 = U256::from(initial_price);

        match bridge_proxy
            .requestCreateItp(
                name.to_string(),
                symbol.to_string(),
                description.to_string(),
                methodology.to_string(),
                price_u256,
                max_order_size,
                assets.to_vec(),
                weights.to_vec(),
            )
            .estimate_gas()
            .await
        {
            Ok(gas) => {
                // Add 20% buffer
                let gas_with_buffer = gas * 120 / 100;
                debug!(
                    estimated = gas,
                    with_buffer = gas_with_buffer,
                    "Gas estimation successful"
                );
                Ok(gas_with_buffer)
            }
            Err(e) => {
                warn!(
                    error = %e,
                    fallback = DEFAULT_GAS_LIMIT,
                    "Gas estimation failed, using fallback"
                );
                Ok(DEFAULT_GAS_LIMIT)
            }
        }
    }

    /// Parse CreateItpRequested event from transaction logs
    fn parse_create_itp_requested_event(
        &self,
        logs: &[alloy::rpc::types::Log],
    ) -> Result<u64, ItpCreationError> {
        // CreateItpRequested(address indexed admin, string name, string symbol, uint256 initialPrice, uint256 indexed nonce)
        // Event signature: keccak256("CreateItpRequested(address,string,string,uint256,uint256)")
        // = 0x... (computed at runtime via alloy)
        let event_signature = IBridgeProxy::CreateItpRequested::SIGNATURE_HASH;

        for log in logs {
            // Verify this is the CreateItpRequested event by checking signature
            if log.topics().len() >= 3 {
                if let Some(topic0) = log.topics().first() {
                    if *topic0 != event_signature {
                        continue; // Not our event, skip
                    }
                }

                // The nonce is the second indexed parameter (topics[2])
                if let Some(nonce_topic) = log.topics().get(2) {
                    let nonce = U256::from_be_bytes(nonce_topic.0);
                    return Ok(nonce.to::<u64>());
                }
            }
        }

        Err(ItpCreationError::EventParsingError(
            "CreateItpRequested event not found in logs".to_string(),
        ))
    }

    /// Wait for ItpCreated event matching the given nonce
    ///
    /// # Arguments
    ///
    /// * `nonce` - The nonce from the CreateItpRequested event
    /// * `from_block` - The block number where the request was confirmed (to avoid stale events)
    async fn wait_for_itp_creation(
        &self,
        nonce: u64,
        request_confirmed_at_block: u64,
    ) -> Result<(String, String), ItpCreationError> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(SYNC_TIMEOUT_MS);
        let poll_interval = Duration::from_millis(POLL_INTERVAL_MS);

        info!(
            nonce = nonce,
            from_block = request_confirmed_at_block,
            timeout_ms = SYNC_TIMEOUT_MS,
            "Polling for ItpCreated event (only looking at blocks after request confirmation)"
        );

        while start.elapsed() < timeout {
            // Get recent logs for ItpCreated event
            let current_block = self.provider.get_block_number().await.map_err(|e| {
                ItpCreationError::ProviderError(format!("Failed to get block number: {}", e))
            })?;

            // Only look at blocks AFTER the request was confirmed - this prevents
            // finding stale ItpCreated events from previous attempts with the same nonce
            let from_block = request_confirmed_at_block;

            // Build filter for ItpCreated event
            let filter = alloy::rpc::types::Filter::new()
                .address(self.bridge_proxy_address)
                .from_block(from_block)
                .to_block(current_block);

            let logs = self.provider.get_logs(&filter).await.map_err(|e| {
                ItpCreationError::ProviderError(format!("Failed to get logs: {}", e))
            })?;

            // Check for matching ItpCreated event
            for log in logs {
                if log.topics().len() >= 4 {
                    // Check if nonce matches (topics[3])
                    if let Some(nonce_topic) = log.topics().get(3) {
                        let event_nonce = U256::from_be_bytes(nonce_topic.0).to::<u64>();
                        if event_nonce == nonce {
                            // Extract addresses from topics[1] and topics[2]
                            let orbit_address = format!("0x{}", hex::encode(&log.topics()[1].0[12..]));
                            let arbitrum_address = format!("0x{}", hex::encode(&log.topics()[2].0[12..]));

                            return Ok((orbit_address, arbitrum_address));
                        }
                    }
                }
            }

            debug!(
                elapsed_ms = start.elapsed().as_millis(),
                "ItpCreated event not found yet, polling..."
            );

            tokio::time::sleep(poll_interval).await;
        }

        Err(ItpCreationError::Timeout(format!(
            "Timeout waiting for ItpCreated event with nonce {}",
            nonce
        )))
    }

    /// Check the status of an ITP creation by nonce (non-blocking)
    ///
    /// Returns (status, orbit_address, arbitrum_address) where:
    /// - status is "pending" if ItpCreated event not found yet
    /// - status is "completed" if ItpCreated event found
    pub async fn check_itp_status(
        &self,
        nonce: u64,
        from_block: u64,
    ) -> Result<(String, Option<String>, Option<String>), ItpCreationError> {
        // Get current block
        let current_block = self.provider.get_block_number().await.map_err(|e| {
            ItpCreationError::ProviderError(format!("Failed to get block number: {}", e))
        })?;

        // Build filter for ItpCreated event
        let filter = alloy::rpc::types::Filter::new()
            .address(self.bridge_proxy_address)
            .from_block(from_block)
            .to_block(current_block);

        let logs = self.provider.get_logs(&filter).await.map_err(|e| {
            ItpCreationError::ProviderError(format!("Failed to get logs: {}", e))
        })?;

        // Check for matching ItpCreated event
        let event_signature = IBridgeProxy::ItpCreated::SIGNATURE_HASH;

        for log in logs {
            // Check if this is an ItpCreated event
            if log.topics().len() >= 4 {
                if let Some(topic0) = log.topics().first() {
                    if *topic0 != event_signature {
                        continue;
                    }
                }

                // Check if nonce matches (topics[3])
                if let Some(nonce_topic) = log.topics().get(3) {
                    let event_nonce = U256::from_be_bytes(nonce_topic.0).to::<u64>();
                    if event_nonce == nonce {
                        // Extract addresses from topics[1] and topics[2]
                        let orbit_address = format!("0x{}", hex::encode(&log.topics()[1].0[12..]));
                        let arbitrum_address = format!("0x{}", hex::encode(&log.topics()[2].0[12..]));

                        return Ok((
                            "completed".to_string(),
                            Some(orbit_address),
                            Some(arbitrum_address),
                        ));
                    }
                }
            }
        }

        // Event not found yet
        Ok(("pending".to_string(), None, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ItpCreationError::ProviderError("test".to_string());
        assert!(err.to_string().contains("Provider error"));

        let err = ItpCreationError::Timeout("test".to_string());
        assert!(err.to_string().contains("Timeout"));
    }

    #[test]
    fn test_itp_creation_result_clone() {
        let result = ItpCreationResult {
            tx_hash: "0x123".to_string(),
            nonce: 42,
        };
        let cloned = result.clone();
        assert_eq!(cloned.tx_hash, "0x123");
        assert_eq!(cloned.nonce, 42);
    }

    #[test]
    fn test_itp_creation_sync_result_clone() {
        let result = ItpCreationSyncResult {
            tx_hash: "0x123".to_string(),
            nonce: 42,
            orbit_address: "0xabc".to_string(),
            arbitrum_address: "0xdef".to_string(),
        };
        let cloned = result.clone();
        assert_eq!(cloned.orbit_address, "0xabc");
        assert_eq!(cloned.arbitrum_address, "0xdef");
    }
}
