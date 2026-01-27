//! ITP (Index Token Product) creation request/response models
//!
//! Models for the POST /api/itp/create endpoint that creates live ITPs
//! via BridgeProxy on Arbitrum.

use serde::{Deserialize, Serialize};

/// Request to create a new ITP via BridgeProxy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateItpRequest {
    /// ITP name (e.g., "Top 10 DeFi Index")
    pub name: String,
    /// ITP symbol (e.g., "DEFI10")
    pub symbol: String,
    /// ITP description
    #[serde(default)]
    pub description: Option<String>,
    /// ITP methodology description
    #[serde(default)]
    pub methodology: Option<String>,
    /// Initial price in USDC (6 decimals, e.g., 1000000 = $1.00)
    pub initial_price: u64,
    /// Maximum order size (default: 1000000000 = 1000 USDC)
    #[serde(default = "default_max_order_size")]
    pub max_order_size: u128,
    /// Asset IDs (required for ITP creation)
    #[serde(default)]
    pub asset_ids: Option<Vec<u128>>,
    /// Asset weights in basis points (must sum to 10000, i.e., 100%)
    #[serde(default)]
    pub weights: Option<Vec<u128>>,
    /// Optional asset composition for metadata/display (deprecated, use asset_ids)
    #[serde(default)]
    pub asset_composition: Option<Vec<String>>,
    /// Wait for bridge confirmation (default: false)
    #[serde(default)]
    pub sync: bool,
    /// Admin/issuer wallet address (Story 2-3 AC#6)
    /// Used to associate the ITP with its creator for portfolio views
    #[serde(default)]
    pub admin_address: Option<String>,
}

/// Default max order size: 1000 USDC (6 decimals)
fn default_max_order_size() -> u128 {
    1_000_000_000
}

/// Async response when sync=false
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateItpResponse {
    /// Transaction hash on Arbitrum
    pub tx_hash: String,
    /// Nonce from CreateItpRequested event
    pub nonce: u64,
    /// Block number where the request was confirmed (for status polling)
    pub confirmed_at_block: u64,
    /// Estimated completion time in seconds
    pub estimated_completion_time: u32,
    /// Current status
    pub status: String,
}

/// Sync response when sync=true (waits for ITP creation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateItpSyncResponse {
    /// Transaction hash on Arbitrum
    pub tx_hash: String,
    /// Nonce from events
    pub nonce: u64,
    /// ITP address on Orbit chain
    pub orbit_address: String,
    /// BridgedItp address on Arbitrum
    pub arbitrum_address: String,
    /// Current status
    pub status: String,
}

/// Error response for ITP creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItpErrorResponse {
    /// Error message
    pub error: String,
    /// Error code for programmatic handling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Query params for status check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItpStatusQuery {
    /// Block number to start searching from (when request was confirmed)
    pub from_block: u64,
}

/// Response for ITP status check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItpStatusResponse {
    /// Nonce being checked
    pub nonce: u64,
    /// Current status: "pending" or "completed"
    pub status: String,
    /// ITP address on Orbit chain (only present when completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orbit_address: Option<String>,
    /// BridgedItp address on Arbitrum (only present when completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arbitrum_address: Option<String>,
}
