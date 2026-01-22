//! ITP Listing Handler
//!
//! GET /api/itp/list endpoint for fetching available ITPs.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use tracing::{error, info, warn};

use crate::models::itp_listing::{ItpListErrorResponse, ItpListQuery, ItpListResponse};
use crate::AppState;

/// Get list of available ITPs
///
/// GET /api/itp/list
///
/// Returns a paginated list of ITPs with optional filtering.
///
/// # Query Parameters
///
/// - `limit` - Maximum number of results (default: 20, max: 100)
/// - `offset` - Offset for pagination (default: 0)
/// - `active` - Filter to only active ITPs (state == 1)
/// - `search` - Search by name or symbol (case-insensitive)
/// - `user_holdings` - Filter by user address (MVP: not implemented)
///
/// # Response
///
/// ```json
/// {
///   "itps": [
///     {
///       "id": 1,
///       "name": "Top 10 DeFi Index",
///       "symbol": "DEFI10",
///       "orbitAddress": "0x1234...",
///       "arbitrumAddress": "0xABCD...",
///       "currentPrice": 1.05,
///       "price24hChange": 2.5,
///       "totalSupply": "1000000000000000000",
///       "createdAt": 1705347200
///     }
///   ],
///   "total": 1,
///   "limit": 20,
///   "offset": 0
/// }
/// ```
pub async fn get_itp_list(
    State(state): State<AppState>,
    Query(query): Query<ItpListQuery>,
) -> Result<Json<ItpListResponse>, (StatusCode, Json<ItpListErrorResponse>)> {
    info!(
        limit = query.limit,
        offset = query.offset,
        active = query.active,
        search = query.search,
        "ITP list request received"
    );

    // Validate query parameters
    if let Err(e) = query.validate() {
        warn!(error = %e, "Invalid query parameters");
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpListErrorResponse { error: e }),
        ));
    }

    // Get real-time prices from exchange feeds
    let realtime_prices = state.realtime_prices.get_all_prices().await;

    // Get ITPs from service with real-time prices
    let (itps, total) = state
        .itp_listing
        .get_itps(&state.db, &query, &realtime_prices)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to query ITPs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ItpListErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    let limit = query.limit.unwrap_or(20);
    let offset = query.offset.unwrap_or(0);

    // Calculate total AUM by summing all ITP AUMs
    let total_aum: Option<f64> = {
        let sum: f64 = itps.iter().filter_map(|itp| itp.aum).sum();
        if sum > 0.0 { Some(sum) } else { None }
    };

    info!(
        count = itps.len(),
        total = total,
        limit = limit,
        offset = offset,
        total_aum = ?total_aum,
        "ITP list returned"
    );

    Ok(Json(ItpListResponse {
        itps,
        total,
        limit,
        offset,
        total_aum,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_validation_valid() {
        let query = ItpListQuery {
            limit: Some(50),
            offset: Some(10),
            active: Some(true),
            user_holdings: None,
            search: Some("DEFI".to_string()),
        };
        assert!(query.validate().is_ok());
    }

    #[test]
    fn test_query_validation_limit_too_high() {
        let query = ItpListQuery {
            limit: Some(200),
            offset: None,
            active: None,
            user_holdings: None,
            search: None,
        };
        assert!(query.validate().is_err());
    }

    #[test]
    fn test_query_validation_limit_too_low() {
        let query = ItpListQuery {
            limit: Some(0),
            offset: None,
            active: None,
            user_holdings: None,
            search: None,
        };
        assert!(query.validate().is_err());
    }

    #[test]
    fn test_query_validation_negative_offset() {
        let query = ItpListQuery {
            limit: None,
            offset: Some(-1),
            active: None,
            user_holdings: None,
            search: None,
        };
        assert!(query.validate().is_err());
    }
}
