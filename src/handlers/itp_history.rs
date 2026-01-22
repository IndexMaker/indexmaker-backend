//! ITP Price History handler
//!
//! GET /api/itp/{id}/history endpoint for fetching historical price data.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};
use rust_decimal::prelude::ToPrimitive;
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder};
use tracing::{error, info, warn};

use crate::entities::{itp_price_history, itps, prelude::{ItpPriceHistory, Itps}};
use crate::models::itp_history::{
    HistoryErrorResponse, Period, PriceHistoryEntry, PriceHistoryQuery, PriceHistoryResponse,
};
use crate::AppState;

/// GET /api/itp/{id}/history
///
/// Returns historical price data for an ITP.
///
/// # Query Parameters
/// - `period`: 1d, 7d, 30d, all (default: 7d)
///
/// # Response
/// - 200: Price history data
/// - 400: Invalid period parameter
/// - 404: ITP not found (when ITP listing is available)
/// - 500: Database error
pub async fn get_itp_price_history(
    State(state): State<AppState>,
    Path(itp_id): Path<String>,
    Query(query): Query<PriceHistoryQuery>,
) -> Result<Json<PriceHistoryResponse>, (StatusCode, Json<HistoryErrorResponse>)> {
    info!(itp_id = %itp_id, period = %query.period, "Fetching ITP price history");

    // Validate period parameter
    let period = query.validate().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(HistoryErrorResponse {
                error: e,
                code: Some("INVALID_PERIOD".to_string()),
            }),
        )
    })?;

    // Check if ITP exists in the database (AC #8: Return 404 for invalid ITP id)
    let itp_exists = Itps::find()
        .filter(itps::Column::OrbitAddress.eq(&itp_id))
        .one(&state.db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database error checking ITP existence");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(HistoryErrorResponse {
                    error: format!("Database error: {}", e),
                    code: Some("DATABASE_ERROR".to_string()),
                }),
            )
        })?;

    if itp_exists.is_none() {
        warn!(itp_id = %itp_id, "ITP not found");
        return Err((
            StatusCode::NOT_FOUND,
            Json(HistoryErrorResponse {
                error: "ITP not found".to_string(),
                code: Some("ITP_NOT_FOUND".to_string()),
            }),
        ));
    }

    // Calculate date range based on period
    let (start_time, granularity) = calculate_range_and_granularity(period);

    // Query price history from database
    let prices = ItpPriceHistory::find()
        .filter(itp_price_history::Column::ItpId.eq(&itp_id))
        .filter(itp_price_history::Column::Timestamp.gte(start_time))
        .filter(itp_price_history::Column::Granularity.eq(granularity))
        .order_by(itp_price_history::Column::Timestamp, Order::Asc)
        .all(&state.db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database error fetching price history");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(HistoryErrorResponse {
                    error: format!("Database error: {}", e),
                    code: Some("DATABASE_ERROR".to_string()),
                }),
            )
        })?;

    info!(
        itp_id = %itp_id,
        count = prices.len(),
        "Price history query completed"
    );

    // Transform to response format (empty vec is valid for new ITPs)
    let data: Vec<PriceHistoryEntry> = prices
        .into_iter()
        .map(|p| PriceHistoryEntry {
            timestamp: p.timestamp.with_timezone(&Utc),
            price: p.price.to_f64().unwrap_or(0.0),
            volume: p.volume.and_then(|v| v.to_f64()),
        })
        .collect();

    Ok(Json(PriceHistoryResponse {
        data,
        itp_id,
        period: period.as_str().to_string(),
    }))
}

/// Calculate the start time and granularity based on period
fn calculate_range_and_granularity(period: Period) -> (chrono::DateTime<Utc>, &'static str) {
    let now = Utc::now();

    match period {
        Period::Day1 => (now - Duration::hours(24), "5min"),
        Period::Day7 => (now - Duration::days(7), "5min"),
        Period::Day30 => (now - Duration::days(30), "hourly"),
        Period::All => {
            // For "all", go back 5 years which covers all realistic data
            (now - Duration::days(365 * 5), "daily")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_range_1d() {
        let (start, granularity) = calculate_range_and_granularity(Period::Day1);
        assert_eq!(granularity, "5min");
        let now = Utc::now();
        let diff = now - start;
        // Should be approximately 24 hours
        assert!(diff.num_hours() >= 23 && diff.num_hours() <= 25);
    }

    #[test]
    fn test_calculate_range_7d() {
        let (start, granularity) = calculate_range_and_granularity(Period::Day7);
        assert_eq!(granularity, "5min");
        let now = Utc::now();
        let diff = now - start;
        // Should be approximately 7 days
        assert!(diff.num_days() >= 6 && diff.num_days() <= 8);
    }

    #[test]
    fn test_calculate_range_30d() {
        let (start, granularity) = calculate_range_and_granularity(Period::Day30);
        assert_eq!(granularity, "hourly");
        let now = Utc::now();
        let diff = now - start;
        // Should be approximately 30 days
        assert!(diff.num_days() >= 29 && diff.num_days() <= 31);
    }

    #[test]
    fn test_calculate_range_all() {
        let (_start, granularity) = calculate_range_and_granularity(Period::All);
        assert_eq!(granularity, "daily");
    }
}
