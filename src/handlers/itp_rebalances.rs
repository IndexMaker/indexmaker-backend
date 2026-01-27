//! ITP Rebalance History handler
//!
//! GET /api/itp/{index_id}/rebalances endpoint for fetching rebalance history.
//! Story 0-1 AC5: Rebalance history tracking.
//!
//! Queries the rebalances table which is populated by the rebalance_sync background job.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use sea_orm::{EntityTrait, QueryFilter, QueryOrder, ColumnTrait};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::AppState;
use crate::entities::rebalances;

/// A single rebalance event from history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebalanceEventResponse {
    pub index_id: u128,
    pub timestamp: Option<u64>,
    pub tx_hash: Option<String>,
    pub coins: serde_json::Value,
    pub rebalance_type: String,
    pub deployed: Option<bool>,
}

/// Response for the rebalance history endpoint.
#[derive(Debug, Serialize)]
pub struct RebalanceHistoryResponse {
    pub index_id: u128,
    pub rebalances: Vec<RebalanceEventResponse>,
    pub total: usize,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct RebalanceErrorResponse {
    pub error: String,
    pub code: Option<String>,
}

/// GET /api/itp/{index_id}/rebalances
///
/// Returns rebalance history for an ITP from the database.
///
/// # Path Parameters
/// - `index_id`: The on-chain index ID (u128)
///
/// # Response
/// - 200: Rebalance history (may be empty for new ITPs)
/// - 400: Invalid index_id
/// - 500: Database query error
pub async fn get_rebalance_history(
    State(state): State<AppState>,
    Path(index_id): Path<String>,
) -> Result<Json<RebalanceHistoryResponse>, (StatusCode, Json<RebalanceErrorResponse>)> {
    // Parse index_id
    let index_id: u128 = index_id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(RebalanceErrorResponse {
                error: "Invalid index_id: must be a numeric value".to_string(),
                code: Some("INVALID_INDEX_ID".to_string()),
            }),
        )
    })?;

    info!(index_id, "Fetching rebalance history");

    // Query rebalances table (populated by rebalance_sync background job)
    let db_index_id = index_id as i32;
    let rebalance_records = rebalances::Entity::find()
        .filter(rebalances::Column::IndexId.eq(db_index_id))
        .order_by_desc(rebalances::Column::Timestamp)
        .all(&state.db)
        .await
        .map_err(|e| {
            warn!(index_id, error = %e, "Database query failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RebalanceErrorResponse {
                    error: "Failed to query rebalance history".to_string(),
                    code: Some("DB_ERROR".to_string()),
                }),
            )
        })?;

    let rebalances: Vec<RebalanceEventResponse> = rebalance_records
        .into_iter()
        .map(|r| RebalanceEventResponse {
            index_id,
            timestamp: Some(r.timestamp as u64),
            tx_hash: r.tx_hash,
            coins: r.coins,
            rebalance_type: r.rebalance_type,
            deployed: r.deployed,
        })
        .collect();

    let total = rebalances.len();
    info!(index_id, count = total, "Rebalance history retrieved");

    Ok(Json(RebalanceHistoryResponse {
        index_id,
        rebalances,
        total,
    }))
}
