//! ITP Price Snapshot Service
//!
//! Background service for snapshotting ITP prices from the Orbit chain.
//! Stores price history in the itp_price_history table.

use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder, RootProvider},
    sol,
    transports::http::{Client, Http},
};
use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
};
use std::str::FromStr;
use tracing::{debug, error, info, warn};

use crate::entities::{itp_price_history, itps, prelude::*};

/// Default Orbit chain ID
const ORBIT_CHAIN_ID: u64 = 111222333;

/// Maximum retry attempts for RPC calls
const MAX_RETRIES: u32 = 3;

/// Base delay between retries (ms)
const RETRY_BASE_DELAY_MS: u64 = 500;

// Define Castle contract interface for price fetching
sol! {
    #[sol(rpc)]
    interface ICastle {
        /// Get the current index price for an ITP
        function getIndexPrice(uint256 indexId) external view returns (uint256);
    }
}

/// Error types for price snapshot service
#[derive(Debug)]
pub enum ItpPriceSnapshotError {
    ProviderError(String),
    ContractCallError(String),
    DatabaseError(String),
    InvalidConfig(String),
}

impl std::fmt::Display for ItpPriceSnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItpPriceSnapshotError::ProviderError(msg) => write!(f, "Provider error: {}", msg),
            ItpPriceSnapshotError::ContractCallError(msg) => {
                write!(f, "Contract call error: {}", msg)
            }
            ItpPriceSnapshotError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            ItpPriceSnapshotError::InvalidConfig(msg) => write!(f, "Invalid config: {}", msg),
        }
    }
}

impl std::error::Error for ItpPriceSnapshotError {}

/// ITP Price Snapshot Service
pub struct ItpPriceSnapshotService {
    db: DatabaseConnection,
    provider: RootProvider<Http<Client>>,
    castle_address: Address,
}

impl ItpPriceSnapshotService {
    /// Create a new ItpPriceSnapshotService
    ///
    /// # Arguments
    ///
    /// * `db` - Database connection
    /// * `orbit_rpc_url` - Orbit chain RPC URL
    /// * `castle_address` - Castle contract address
    pub async fn new(
        db: DatabaseConnection,
        orbit_rpc_url: &str,
        castle_address_str: &str,
    ) -> Result<Self, ItpPriceSnapshotError> {
        info!(
            orbit_rpc = %orbit_rpc_url,
            castle_address = %castle_address_str,
            "Initializing ItpPriceSnapshotService"
        );

        let provider = ProviderBuilder::new()
            .on_http(orbit_rpc_url.parse().map_err(|e| {
                ItpPriceSnapshotError::InvalidConfig(format!("Invalid RPC URL: {}", e))
            })?);

        // Verify connection
        let chain_id = provider.get_chain_id().await.map_err(|e| {
            error!(error = %e, "Failed to connect to Orbit RPC");
            ItpPriceSnapshotError::ProviderError(format!("Connection failed: {}", e))
        })?;

        if chain_id != ORBIT_CHAIN_ID {
            warn!(
                expected = ORBIT_CHAIN_ID,
                actual = chain_id,
                "Chain ID mismatch - expected Orbit chain"
            );
        }

        let castle_address = Address::from_str(castle_address_str).map_err(|e| {
            ItpPriceSnapshotError::InvalidConfig(format!("Invalid Castle address: {}", e))
        })?;

        info!(
            chain_id = chain_id,
            castle_address = %castle_address,
            "ItpPriceSnapshotService initialized"
        );

        Ok(Self {
            db,
            provider,
            castle_address,
        })
    }

    /// Snapshot prices for all active ITPs
    ///
    /// Queries the itps table for active ITPs (state = 1),
    /// fetches current prices from Castle contract,
    /// and stores snapshots in itp_price_history table.
    pub async fn snapshot_all_itp_prices(&self) -> Result<usize, ItpPriceSnapshotError> {
        let now = Utc::now();
        info!(timestamp = %now, "Starting price snapshot for all ITPs");

        // Get all active ITPs (state = 1 means active/approved)
        let active_itps = Itps::find()
            .filter(itps::Column::State.eq(1i16))
            .all(&self.db)
            .await
            .map_err(|e| {
                ItpPriceSnapshotError::DatabaseError(format!("Failed to query ITPs: {}", e))
            })?;

        if active_itps.is_empty() {
            info!("No active ITPs found to snapshot");
            return Ok(0);
        }

        info!(count = active_itps.len(), "Found active ITPs to snapshot");

        let mut success_count = 0;

        let itps_len = active_itps.len();
        for itp in active_itps {
            // Pass index_id for Castle contract lookup (preferred) or orbit_address as fallback
            match self.snapshot_single_itp(&itp.orbit_address, itp.index_id, now).await {
                Ok(()) => {
                    success_count += 1;
                    debug!(itp = %itp.orbit_address, "Price snapshot saved");
                }
                Err(e) => {
                    warn!(
                        itp = %itp.orbit_address,
                        index_id = ?itp.index_id,
                        error = %e,
                        "Failed to snapshot price, skipping"
                    );
                }
            }
        }

        info!(
            success = success_count,
            total = itps_len,
            "Price snapshot batch completed"
        );

        Ok(success_count)
    }

    /// Snapshot price for a single ITP
    ///
    /// # Arguments
    ///
    /// * `itp_id` - ITP vault address (orbit_address) - used for DB storage
    /// * `index_id` - Numeric index ID from database (preferred for Castle lookup)
    /// * `timestamp` - Timestamp for the snapshot
    async fn snapshot_single_itp(
        &self,
        itp_id: &str,
        index_id: Option<i64>,
        timestamp: chrono::DateTime<Utc>,
    ) -> Result<(), ItpPriceSnapshotError> {
        // Fetch current price from Castle contract
        let price = match self.get_itp_current_price(itp_id, index_id).await {
            Some(p) => p,
            None => {
                warn!(itp = %itp_id, index_id = ?index_id, "Could not fetch price, skipping");
                return Ok(()); // Not an error, just skip
            }
        };

        // Insert snapshot into database
        let snapshot = itp_price_history::ActiveModel {
            itp_id: Set(itp_id.to_string()),
            price: Set(price),
            volume: Set(None), // Volume not available from Castle
            timestamp: Set(timestamp.fixed_offset()),
            granularity: Set("5min".to_string()),
            ..Default::default()
        };

        snapshot.insert(&self.db).await.map_err(|e| {
            ItpPriceSnapshotError::DatabaseError(format!("Failed to insert snapshot: {}", e))
        })?;

        Ok(())
    }

    /// Get current price for an ITP from Castle contract
    ///
    /// # Arguments
    ///
    /// * `itp_address` - ITP vault address (used for logging/fallback)
    /// * `index_id` - Numeric index ID for Castle contract lookup (preferred)
    ///
    /// # Returns
    ///
    /// Option<Decimal> - Price if successful, None if failed
    pub async fn get_itp_current_price(&self, itp_address: &str, index_id: Option<i64>) -> Option<Decimal> {
        // Determine the index ID to use for Castle lookup
        // Prefer numeric index_id from database, fall back to address-derived ID
        let castle_index_id = if let Some(id) = index_id {
            U256::from(id as u64)
        } else {
            // Fallback: derive from address (legacy behavior)
            let itp_addr = Address::from_str(itp_address).ok()?;
            U256::from_be_bytes(itp_addr.into_word().0)
        };

        // Try to fetch the price with retries
        let mut attempts = 0;
        while attempts < MAX_RETRIES {
            match self.fetch_price_from_castle(castle_index_id).await {
                Ok(price) => return Some(price),
                Err(e) => {
                    attempts += 1;
                    if attempts < MAX_RETRIES {
                        warn!(
                            itp = %itp_address,
                            index_id = ?index_id,
                            attempt = attempts,
                            error = %e,
                            "Price fetch failed, retrying..."
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(
                            RETRY_BASE_DELAY_MS * (1 << attempts),
                        ))
                        .await;
                    }
                }
            }
        }

        error!(itp = %itp_address, index_id = ?index_id, "Max retries exceeded for price fetch");
        None
    }

    /// Fetch price from Castle contract
    ///
    /// # Arguments
    ///
    /// * `index_id` - The index ID to query (U256)
    async fn fetch_price_from_castle(
        &self,
        index_id: U256,
    ) -> Result<Decimal, ItpPriceSnapshotError> {
        let castle = ICastle::new(self.castle_address, &self.provider);

        let price_u256 = castle
            .getIndexPrice(index_id)
            .call()
            .await
            .map_err(|e| {
                ItpPriceSnapshotError::ContractCallError(format!("getIndexPrice failed: {}", e))
            })?
            ._0;

        // Convert U256 to Decimal (assuming 18 decimals)
        // Price is returned as raw U256, we need to convert to Decimal
        let price_str = price_u256.to_string();
        let price = Decimal::from_str(&price_str)
            .map_err(|e| ItpPriceSnapshotError::ContractCallError(format!("Invalid price: {}", e)))?;

        // Normalize to 18 decimal places (divide by 10^18)
        let divisor = Decimal::from_str("1000000000000000000").unwrap_or(Decimal::ONE);
        let normalized_price = price / divisor;

        debug!(
            raw_price = %price_str,
            normalized = %normalized_price,
            "Fetched price from Castle"
        );

        Ok(normalized_price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ItpPriceSnapshotError::ProviderError("test".to_string());
        assert!(err.to_string().contains("Provider error"));

        let err = ItpPriceSnapshotError::DatabaseError("test".to_string());
        assert!(err.to_string().contains("Database error"));
    }

    #[test]
    fn test_constants() {
        assert_eq!(ORBIT_CHAIN_ID, 111222333);
        assert_eq!(MAX_RETRIES, 3);
    }
}
