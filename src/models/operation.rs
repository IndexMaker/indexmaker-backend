//! Operation types and status enums for buy/sell/rebalance tracking
//!
//! Story 3.2 - AC #7: Status progresses through defined states

use serde::{Deserialize, Serialize};

/// Operation types tracked by the system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationType {
    Buy,
    Sell,
    Rebalance,
}

impl std::fmt::Display for OperationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OperationType::Buy => write!(f, "buy"),
            OperationType::Sell => write!(f, "sell"),
            OperationType::Rebalance => write!(f, "rebalance"),
        }
    }
}

impl std::str::FromStr for OperationType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "buy" => Ok(OperationType::Buy),
            "sell" => Ok(OperationType::Sell),
            "rebalance" => Ok(OperationType::Rebalance),
            _ => Err(format!("Unknown operation type: {}", s)),
        }
    }
}

/// Operation status values per architecture spec
/// Status progresses: initiated → approved → bridging → executing → settling → complete
///                                                                    ↘ failed
///                                                                    ↘ refunded
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationStatus {
    /// Initial state when user submits transaction
    Initiated,
    /// Transaction approved on Arbitrum
    Approved,
    /// Assets being bridged between chains
    Bridging,
    /// Order being executed on Orbit
    Executing,
    /// Waiting for settlement
    Settling,
    /// Successfully completed
    Complete,
    /// Operation failed
    Failed,
    /// Funds refunded after failure
    Refunded,
}

impl std::fmt::Display for OperationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OperationStatus::Initiated => write!(f, "initiated"),
            OperationStatus::Approved => write!(f, "approved"),
            OperationStatus::Bridging => write!(f, "bridging"),
            OperationStatus::Executing => write!(f, "executing"),
            OperationStatus::Settling => write!(f, "settling"),
            OperationStatus::Complete => write!(f, "complete"),
            OperationStatus::Failed => write!(f, "failed"),
            OperationStatus::Refunded => write!(f, "refunded"),
        }
    }
}

impl std::str::FromStr for OperationStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "initiated" => Ok(OperationStatus::Initiated),
            "approved" => Ok(OperationStatus::Approved),
            "bridging" => Ok(OperationStatus::Bridging),
            "executing" => Ok(OperationStatus::Executing),
            "settling" => Ok(OperationStatus::Settling),
            "complete" => Ok(OperationStatus::Complete),
            "failed" => Ok(OperationStatus::Failed),
            "refunded" => Ok(OperationStatus::Refunded),
            _ => Err(format!("Unknown operation status: {}", s)),
        }
    }
}

/// Operation event sent via WebSocket
/// Matches architecture spec for OperationEvent message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationEvent {
    /// Operation type
    #[serde(rename = "type")]
    pub operation_type: OperationType,
    /// Operation nonce
    pub nonce: u64,
    /// User wallet address
    pub user: String,
    /// Current status
    pub status: OperationStatus,
    /// Current phase description (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Generic transaction hash (for backwards compatibility)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    /// Arbitrum transaction hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arb_tx_hash: Option<String>,
    /// Orbit transaction hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orbit_tx_hash: Option<String>,
    /// Error details if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,
    /// Timestamp in milliseconds
    pub timestamp: u64,
}

/// Operation error structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

/// Request to create or update an operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOperationRequest {
    pub user_address: String,
    pub operation_type: OperationType,
    pub nonce: u64,
    pub status: OperationStatus,
    pub arb_tx_hash: Option<String>,
    pub amount: Option<String>,
    pub itp_address: Option<String>,
}

/// Response for operation queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResponse {
    pub id: i32,
    pub user_address: String,
    pub operation_type: String,
    pub nonce: i64,
    pub status: String,
    pub arb_tx_hash: Option<String>,
    pub orbit_tx_hash: Option<String>,
    pub completion_tx_hash: Option<String>,
    pub amount: Option<String>,
    pub itp_amount: Option<String>,
    pub itp_address: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub retryable: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::entities::operations::Model> for OperationResponse {
    fn from(model: crate::entities::operations::Model) -> Self {
        Self {
            id: model.id,
            user_address: model.user_address,
            operation_type: model.operation_type,
            nonce: model.nonce,
            status: model.status,
            arb_tx_hash: model.arb_tx_hash,
            orbit_tx_hash: model.orbit_tx_hash,
            completion_tx_hash: model.completion_tx_hash,
            amount: model.amount,
            itp_amount: model.itp_amount,
            itp_address: model.itp_address,
            error_code: model.error_code,
            error_message: model.error_message,
            retryable: model.retryable,
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }
}
