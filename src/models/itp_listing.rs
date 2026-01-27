//! ITP (Index Token Product) listing request/response models
//!
//! Models for the GET /api/itp/list endpoint that returns available ITPs.

use serde::{Deserialize, Serialize};

/// Query parameters for ITP listing
#[derive(Debug, Clone, Deserialize)]
pub struct ItpListQuery {
    /// Maximum number of results (default: 20, max: 100)
    pub limit: Option<i32>,
    /// Offset for pagination (default: 0)
    pub offset: Option<i32>,
    /// Filter by active state only (state == 1)
    pub active: Option<bool>,
    /// Filter by user holdings (address) - MVP: not implemented
    pub user_holdings: Option<String>,
    /// Search by name or symbol (case-insensitive)
    pub search: Option<String>,
    /// Filter by admin/issuer address (Story 2-3 AC#6: Issuer portfolio view)
    pub admin_address: Option<String>,
}

impl ItpListQuery {
    /// Validate query parameters
    pub fn validate(&self) -> Result<(), String> {
        if let Some(limit) = self.limit {
            if limit < 1 {
                return Err("limit must be at least 1".to_string());
            }
            if limit > 100 {
                return Err("limit cannot exceed 100".to_string());
            }
        }
        if let Some(offset) = self.offset {
            if offset < 0 {
                return Err("offset cannot be negative".to_string());
            }
        }
        Ok(())
    }
}

/// Single ITP entry in the listing response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItpListEntry {
    /// Database ID
    pub id: i32,
    /// ITP name (e.g., "Top 10 DeFi Index")
    pub name: String,
    /// ITP symbol (e.g., "DEFI10")
    pub symbol: String,
    /// ITP contract address on Orbit chain
    pub orbit_address: String,
    /// BridgedItp contract address on Arbitrum
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arbitrum_address: Option<String>,
    /// Index ID on chain
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_id: Option<i64>,
    /// Current price in USD (None if not yet tracked)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_price: Option<f64>,
    /// Price change in last 24 hours as percentage (None if no history)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_24h_change: Option<f64>,
    /// Initial price (18 decimals)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_price: Option<String>,
    /// Total supply as string (large number)
    pub total_supply: String,
    /// Investment methodology
    #[serde(skip_serializing_if = "Option::is_none")]
    pub methodology: Option<String>,
    /// ITP description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Asset symbols (e.g., ["BTC", "ETH", "SOL"])
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<Vec<String>>,
    /// Asset weights (e.g., [0.5, 0.3, 0.2])
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weights: Option<Vec<f64>>,
    /// Assets Under Management in USD (total_supply * current_price)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aum: Option<f64>,
    /// Admin/issuer wallet address (Story 2-3 AC#6)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_address: Option<String>,
    /// Unix timestamp when ITP was created
    pub created_at: i64,
}

/// Response for GET /api/itp/list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItpListResponse {
    /// List of ITPs
    pub itps: Vec<ItpListEntry>,
    /// Total count of ITPs matching filters (for pagination)
    pub total: i64,
    /// Limit used in query
    pub limit: i32,
    /// Offset used in query
    pub offset: i32,
    /// Total AUM across all ITPs in USD
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_aum: Option<f64>,
}

/// Error response for ITP listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItpListErrorResponse {
    /// Error message
    pub error: String,
}
