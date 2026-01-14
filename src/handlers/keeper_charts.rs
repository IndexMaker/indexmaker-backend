//! Keeper Charts API handlers
//!
//! Provides endpoints for querying keeper claimable data time-series
//! for chart rendering in the frontend.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::{prelude::ToPrimitive, Decimal};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};

use crate::entities::{keeper_claimable_data, prelude::*};
use crate::models::token::ErrorResponse;
use crate::AppState;

/// Query parameters for time range filtering
#[derive(Debug, Deserialize)]
pub struct TimeRangeQuery {
    /// Start date (inclusive) in YYYY-MM-DD format
    pub start_date: Option<String>,
    /// End date (inclusive) in YYYY-MM-DD format
    pub end_date: Option<String>,
}

/// Single data point for chart rendering
#[derive(Debug, Serialize)]
pub struct KeeperChartDataPoint {
    pub timestamp: String,
    pub acquisition: AcquisitionValues,
    pub disposal: DisposalValues,
}

#[derive(Debug, Serialize)]
pub struct AcquisitionValues {
    pub value1: String,
    pub value2: String,
}

#[derive(Debug, Serialize)]
pub struct DisposalValues {
    pub value1: String,
    pub value2: String,
}

/// Response for historical data endpoint
#[derive(Debug, Serialize)]
pub struct KeeperHistoryResponse {
    pub keeper_address: String,
    pub data: Vec<KeeperChartDataPoint>,
    pub total_records: usize,
    pub time_range: Option<TimeRange>,
}

#[derive(Debug, Serialize)]
pub struct TimeRange {
    pub start: String,
    pub end: String,
}

/// Response for latest data endpoint
#[derive(Debug, Serialize)]
pub struct KeeperLatestResponse {
    pub keeper_address: String,
    pub timestamp: Option<String>,
    pub acquisition: Option<AcquisitionValues>,
    pub disposal: Option<DisposalValues>,
}

/// Response for all keepers endpoint
#[derive(Debug, Serialize)]
pub struct AllKeepersResponse {
    pub keepers: Vec<KeeperLatestResponse>,
    pub total_count: usize,
}

/// GET /api/keeper-charts/{keeper_address}/history
///
/// Returns time-series data for a specific keeper address.
/// Supports optional time range filtering via query parameters.
pub async fn get_keeper_history(
    State(state): State<AppState>,
    Path(keeper_address): Path<String>,
    Query(query): Query<TimeRangeQuery>,
) -> Result<Json<KeeperHistoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    // First, find the earliest timestamp with non-zero data (if no start_date provided)
    let effective_start: Option<NaiveDateTime> = if query.start_date.is_none() {
        // Find first record where any value is non-zero
        let first_nonzero = KeeperClaimableData::find()
            .filter(keeper_claimable_data::Column::KeeperAddress.eq(&keeper_address))
            .filter(
                keeper_claimable_data::Column::AcquisitionValue1.ne(Decimal::ZERO)
                    .or(keeper_claimable_data::Column::AcquisitionValue2.ne(Decimal::ZERO))
                    .or(keeper_claimable_data::Column::DisposalValue1.ne(Decimal::ZERO))
                    .or(keeper_claimable_data::Column::DisposalValue2.ne(Decimal::ZERO))
            )
            .order_by(keeper_claimable_data::Column::RecordedAt, Order::Asc)
            .one(&state.db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Database error finding first non-zero record");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Database error: {}", e),
                    }),
                )
            })?;

        first_nonzero.map(|r| r.recorded_at)
    } else {
        None
    };

    let mut db_query = KeeperClaimableData::find()
        .filter(keeper_claimable_data::Column::KeeperAddress.eq(&keeper_address))
        .order_by(keeper_claimable_data::Column::RecordedAt, Order::Asc);

    // Apply time range filters
    // Use effective_start (first non-zero) if no start_date provided
    if let Some(start_str) = &query.start_date {
        if let Ok(start_date) = NaiveDate::parse_from_str(start_str, "%Y-%m-%d") {
            let start_datetime = start_date.and_hms_opt(0, 0, 0).unwrap();
            db_query = db_query.filter(
                keeper_claimable_data::Column::RecordedAt.gte(start_datetime),
            );
        }
    } else if let Some(start_dt) = effective_start {
        // Start from first non-zero record
        db_query = db_query.filter(
            keeper_claimable_data::Column::RecordedAt.gte(start_dt),
        );
    }

    if let Some(end_str) = &query.end_date {
        if let Ok(end_date) = NaiveDate::parse_from_str(end_str, "%Y-%m-%d") {
            let end_datetime = end_date.and_hms_opt(23, 59, 59).unwrap();
            db_query = db_query.filter(
                keeper_claimable_data::Column::RecordedAt.lte(end_datetime),
            );
        }
    }

    let records = db_query.all(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "Database error fetching keeper history");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Database error: {}", e),
            }),
        )
    })?;

    let data: Vec<KeeperChartDataPoint> = records
        .iter()
        .map(|r| KeeperChartDataPoint {
            timestamp: r.recorded_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            acquisition: AcquisitionValues {
                value1: r.acquisition_value_1.to_string(),
                value2: r.acquisition_value_2.to_string(),
            },
            disposal: DisposalValues {
                value1: r.disposal_value_1.to_string(),
                value2: r.disposal_value_2.to_string(),
            },
        })
        .collect();

    let time_range = if !data.is_empty() {
        Some(TimeRange {
            start: data.first().map(|d| d.timestamp.clone()).unwrap_or_default(),
            end: data.last().map(|d| d.timestamp.clone()).unwrap_or_default(),
        })
    } else {
        None
    };

    Ok(Json(KeeperHistoryResponse {
        keeper_address,
        total_records: data.len(),
        data,
        time_range,
    }))
}

/// GET /api/keeper-charts/{keeper_address}/latest
///
/// Returns the most recent data point for a specific keeper address.
pub async fn get_keeper_latest(
    State(state): State<AppState>,
    Path(keeper_address): Path<String>,
) -> Result<Json<KeeperLatestResponse>, (StatusCode, Json<ErrorResponse>)> {
    let record = KeeperClaimableData::find()
        .filter(keeper_claimable_data::Column::KeeperAddress.eq(&keeper_address))
        .order_by(keeper_claimable_data::Column::RecordedAt, Order::Desc)
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching keeper latest");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    let response = match record {
        Some(r) => KeeperLatestResponse {
            keeper_address,
            timestamp: Some(r.recorded_at.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
            acquisition: Some(AcquisitionValues {
                value1: r.acquisition_value_1.to_string(),
                value2: r.acquisition_value_2.to_string(),
            }),
            disposal: Some(DisposalValues {
                value1: r.disposal_value_1.to_string(),
                value2: r.disposal_value_2.to_string(),
            }),
        },
        None => KeeperLatestResponse {
            keeper_address,
            timestamp: None,
            acquisition: None,
            disposal: None,
        },
    };

    Ok(Json(response))
}

/// GET /api/keeper-charts/all
///
/// Returns the latest data point for all monitored keepers.
/// Uses efficient query to get only distinct keeper addresses first,
/// then fetches latest record for each.
pub async fn get_all_keepers(
    State(state): State<AppState>,
) -> Result<Json<AllKeepersResponse>, (StatusCode, Json<ErrorResponse>)> {
    use sea_orm::QuerySelect;

    // Get distinct keeper addresses efficiently
    let distinct_keepers: Vec<String> = KeeperClaimableData::find()
        .select_only()
        .column(keeper_claimable_data::Column::KeeperAddress)
        .distinct()
        .into_tuple()
        .all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching distinct keepers");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    // Fetch latest record for each keeper
    let mut keepers = Vec::with_capacity(distinct_keepers.len());
    for keeper_address in distinct_keepers {
        let record = KeeperClaimableData::find()
            .filter(keeper_claimable_data::Column::KeeperAddress.eq(&keeper_address))
            .order_by(keeper_claimable_data::Column::RecordedAt, Order::Desc)
            .one(&state.db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, keeper = %keeper_address, "Database error fetching keeper latest");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Database error: {}", e),
                    }),
                )
            })?;

        if let Some(r) = record {
            keepers.push(KeeperLatestResponse {
                keeper_address: r.keeper_address,
                timestamp: Some(r.recorded_at.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                acquisition: Some(AcquisitionValues {
                    value1: r.acquisition_value_1.to_string(),
                    value2: r.acquisition_value_2.to_string(),
                }),
                disposal: Some(DisposalValues {
                    value1: r.disposal_value_1.to_string(),
                    value2: r.disposal_value_2.to_string(),
                }),
            });
        }
    }

    let total_count = keepers.len();

    Ok(Json(AllKeepersResponse {
        keepers,
        total_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_range_query_defaults() {
        let query = TimeRangeQuery {
            start_date: None,
            end_date: None,
        };
        assert!(query.start_date.is_none());
        assert!(query.end_date.is_none());
    }

    #[test]
    fn test_acquisition_values_serialize() {
        let values = AcquisitionValues {
            value1: "1000".to_string(),
            value2: "2000".to_string(),
        };
        let json = serde_json::to_string(&values).unwrap();
        assert!(json.contains("value1"));
        assert!(json.contains("1000"));
    }
}
