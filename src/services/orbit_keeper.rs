//! Orbit Keeper RPC client for fetching claimable data from VAULT contract
//!
//! Connects to Orbit chain and calls getClaimableAcquisition/getClaimableDisposal
//! methods on the VAULT contract for keeper addresses.

use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder, RootProvider},
    sol,
    transports::http::{Client, Http},
};
use std::str::FromStr;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// VAULT contract address on Orbit chain
const VAULT_ADDRESS: &str = "0x621f5f30d4902ab75d8bd50820cc0a09cc563559";

/// Orbit chain ID
const ORBIT_CHAIN_ID: u64 = 111222333;

/// Maximum retry attempts for RPC calls
const MAX_RETRIES: u32 = 3;

/// Base delay between retries (will be exponentially increased)
const RETRY_BASE_DELAY_MS: u64 = 1000;

// Define the VAULT contract interface using alloy's sol! macro
sol! {
    #[sol(rpc)]
    interface IVault {
        function getClaimableAcquisition(address keeper) external view returns (uint128, uint128);
        function getClaimableDisposal(address keeper) external view returns (uint128, uint128);
    }
}

/// Result of fetching keeper claimable data
#[derive(Debug, Clone)]
pub struct KeeperClaimableResult {
    pub keeper_address: String,
    pub acquisition_value_1: u128,
    pub acquisition_value_2: u128,
    pub disposal_value_1: u128,
    pub disposal_value_2: u128,
    pub block_number: u64,
    pub block_timestamp: u64,
}

/// Error types for Orbit Keeper service
#[derive(Debug)]
pub enum OrbitKeeperError {
    ProviderError(String),
    ContractCallError(String),
    InvalidAddress(String),
    MaxRetriesExceeded(String),
}

impl std::fmt::Display for OrbitKeeperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrbitKeeperError::ProviderError(msg) => write!(f, "Provider error: {}", msg),
            OrbitKeeperError::ContractCallError(msg) => write!(f, "Contract call error: {}", msg),
            OrbitKeeperError::InvalidAddress(msg) => write!(f, "Invalid address: {}", msg),
            OrbitKeeperError::MaxRetriesExceeded(msg) => {
                write!(f, "Max retries exceeded: {}", msg)
            }
        }
    }
}

impl std::error::Error for OrbitKeeperError {}

/// Orbit Keeper service for fetching claimable data
pub struct OrbitKeeperService {
    provider: RootProvider<Http<Client>>,
    vault_address: Address,
}

impl OrbitKeeperService {
    /// Create a new OrbitKeeperService
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - Orbit RPC URL (e.g., "https://index.rpc.zeeve.net")
    ///
    /// # Errors
    ///
    /// Returns error if URL is invalid or connection fails
    pub async fn new(rpc_url: &str) -> Result<Self, OrbitKeeperError> {
        info!(rpc_url = %rpc_url, "Initializing OrbitKeeperService");

        let provider = ProviderBuilder::new()
            .on_http(rpc_url.parse().map_err(|e| {
                OrbitKeeperError::ProviderError(format!("Invalid RPC URL: {}", e))
            })?);

        // Verify connection
        let chain_id = provider.get_chain_id().await.map_err(|e| {
            error!(error = %e, "Failed to connect to Orbit RPC");
            OrbitKeeperError::ProviderError(format!("Connection failed: {}", e))
        })?;

        if chain_id != ORBIT_CHAIN_ID {
            warn!(
                expected = ORBIT_CHAIN_ID,
                actual = chain_id,
                "Chain ID mismatch - expected Orbit chain"
            );
        }

        let vault_address = Address::from_str(VAULT_ADDRESS).map_err(|e| {
            OrbitKeeperError::InvalidAddress(format!("Invalid VAULT address: {}", e))
        })?;

        info!(
            chain_id = chain_id,
            vault_address = %vault_address,
            "OrbitKeeperService initialized successfully"
        );

        Ok(Self {
            provider,
            vault_address,
        })
    }

    /// Get claimable acquisition values for a keeper address
    ///
    /// # Arguments
    ///
    /// * `keeper` - Keeper address in 0x format
    ///
    /// # Returns
    ///
    /// Tuple of (value1, value2) from getClaimableAcquisition
    pub async fn get_claimable_acquisition(
        &self,
        keeper: &str,
    ) -> Result<(u128, u128), OrbitKeeperError> {
        let keeper_address = Address::from_str(keeper)
            .map_err(|e| OrbitKeeperError::InvalidAddress(format!("Invalid keeper: {}", e)))?;

        self.with_retry("getClaimableAcquisition", || async {
            let vault = IVault::new(self.vault_address, &self.provider);
            let result = vault
                .getClaimableAcquisition(keeper_address)
                .call()
                .await
                .map_err(|e| {
                    OrbitKeeperError::ContractCallError(format!(
                        "getClaimableAcquisition failed: {}",
                        e
                    ))
                })?;

            Ok((result._0, result._1))
        })
        .await
    }

    /// Get claimable disposal values for a keeper address
    ///
    /// # Arguments
    ///
    /// * `keeper` - Keeper address in 0x format
    ///
    /// # Returns
    ///
    /// Tuple of (value1, value2) from getClaimableDisposal
    pub async fn get_claimable_disposal(
        &self,
        keeper: &str,
    ) -> Result<(u128, u128), OrbitKeeperError> {
        let keeper_address = Address::from_str(keeper)
            .map_err(|e| OrbitKeeperError::InvalidAddress(format!("Invalid keeper: {}", e)))?;

        self.with_retry("getClaimableDisposal", || async {
            let vault = IVault::new(self.vault_address, &self.provider);
            let result = vault
                .getClaimableDisposal(keeper_address)
                .call()
                .await
                .map_err(|e| {
                    OrbitKeeperError::ContractCallError(format!(
                        "getClaimableDisposal failed: {}",
                        e
                    ))
                })?;

            Ok((result._0, result._1))
        })
        .await
    }

    /// Get all claimable data for a keeper (both acquisition and disposal)
    ///
    /// # Arguments
    ///
    /// * `keeper` - Keeper address in 0x format
    ///
    /// # Returns
    ///
    /// KeeperClaimableResult with all four values and block timestamp
    pub async fn get_claimable_data(
        &self,
        keeper: &str,
    ) -> Result<KeeperClaimableResult, OrbitKeeperError> {
        debug!(keeper = %keeper, "Fetching claimable data");

        // Get current block for timestamp
        let block = self.provider.get_block_number().await.map_err(|e| {
            OrbitKeeperError::ProviderError(format!("Failed to get block number: {}", e))
        })?;

        // Get block details to extract timestamp using raw JSON-RPC
        let block_timestamp: u64 = {
            let params = serde_json::json!([format!("0x{:x}", block), false]);
            let response: serde_json::Value = self.provider
                .client()
                .request("eth_getBlockByNumber", params)
                .await
                .map_err(|e| {
                    OrbitKeeperError::ProviderError(format!("Failed to get block details: {}", e))
                })?;

            // Parse timestamp from hex string
            response["timestamp"]
                .as_str()
                .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0)
        };

        let (acq_1, acq_2) = self.get_claimable_acquisition(keeper).await?;
        let (disp_1, disp_2) = self.get_claimable_disposal(keeper).await?;

        let result = KeeperClaimableResult {
            keeper_address: keeper.to_string(),
            acquisition_value_1: acq_1,
            acquisition_value_2: acq_2,
            disposal_value_1: disp_1,
            disposal_value_2: disp_2,
            block_number: block,
            block_timestamp,
        };

        debug!(
            keeper = %keeper,
            acq_1 = acq_1,
            acq_2 = acq_2,
            disp_1 = disp_1,
            disp_2 = disp_2,
            block_number = block,
            block_timestamp = block_timestamp,
            "Fetched claimable data successfully"
        );

        Ok(result)
    }

    /// Execute an async operation with exponential backoff retry
    async fn with_retry<T, F, Fut>(&self, operation: &str, f: F) -> Result<T, OrbitKeeperError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, OrbitKeeperError>>,
    {
        let mut attempts = 0;
        let mut last_error = None;

        while attempts < MAX_RETRIES {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    attempts += 1;
                    last_error = Some(e);

                    if attempts < MAX_RETRIES {
                        let delay = Duration::from_millis(RETRY_BASE_DELAY_MS * (1 << attempts));
                        warn!(
                            operation = %operation,
                            attempt = attempts,
                            max_attempts = MAX_RETRIES,
                            delay_ms = delay.as_millis(),
                            "RPC call failed, retrying..."
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        error!(
            operation = %operation,
            attempts = attempts,
            "Max retries exceeded"
        );

        Err(OrbitKeeperError::MaxRetriesExceeded(format!(
            "{}: {}",
            operation,
            last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string())
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_address_is_valid() {
        let result = Address::from_str(VAULT_ADDRESS);
        assert!(result.is_ok());
    }

    #[test]
    fn test_keeper_claimable_result_debug() {
        let result = KeeperClaimableResult {
            keeper_address: "0x1234".to_string(),
            acquisition_value_1: 100,
            acquisition_value_2: 200,
            disposal_value_1: 50,
            disposal_value_2: 75,
            block_number: 12345,
            block_timestamp: 1700000000,
        };
        assert_eq!(result.keeper_address, "0x1234");
        assert_eq!(result.acquisition_value_1, 100);
        assert_eq!(result.block_number, 12345);
    }

    #[test]
    fn test_error_display() {
        let err = OrbitKeeperError::ProviderError("test".to_string());
        assert!(err.to_string().contains("Provider error"));
    }
}
