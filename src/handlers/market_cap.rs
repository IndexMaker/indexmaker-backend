use axum::{extract::{Query, State}, http::StatusCode, Json};
use chrono::{Duration, NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder};

use crate::{
    entities::{category_membership, coingecko_categories, coins, coins_historical_prices, prelude::*},
    models::{
        market_cap::{MarketCapDataPoint, MarketCapHistoryQuery, MarketCapHistoryResponse,
                     TopCategoryQuery, TopCategoryResponse, TopCategoryCoin},
        token::ErrorResponse,
    },
    AppState,
};

/// Handler for GET /api/market-cap/history
/// Fetches historical market cap data for a cryptocurrency over a date range
pub async fn get_market_cap_history(
    State(state): State<AppState>,
    Query(query): Query<MarketCapHistoryQuery>,
) -> Result<Json<MarketCapHistoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate query parameters
    if let Err(e) = query.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: e }),
        ));
    }

    tracing::info!(
        "Fetching market cap history for coin_id: {} (start: {:?}, end: {:?})",
        query.coin_id,
        query.start_date,
        query.end_date
    );

    // Verify coin exists in database
    let coin = Coins::find()
        .filter(coins::Column::CoinId.eq(&query.coin_id))
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    let coin = coin.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Coin '{}' not found in system", query.coin_id),
            }),
        )
    })?;

    // Parse date range with defaults
    let today = Utc::now().date_naive();
    let default_start = today - Duration::days(365);

    let start_date = if let Some(ref start_str) = query.start_date {
        NaiveDate::parse_from_str(start_str, "%Y-%m-%d").map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid start_date format: '{}'", start_str),
                }),
            )
        })?
    } else {
        default_start
    };

    let end_date = if let Some(ref end_str) = query.end_date {
        NaiveDate::parse_from_str(end_str, "%Y-%m-%d").map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid end_date format: '{}'", end_str),
                }),
            )
        })?
    } else {
        today
    };

    // Validate date range logic
    if start_date > end_date {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "start_date must be before or equal to end_date".to_string(),
            }),
        ));
    }

    if end_date > today {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "end_date cannot be in the future".to_string(),
            }),
        ));
    }

    // Warn about large date ranges
    let days_requested = (end_date - start_date).num_days();
    if days_requested > 1825 {
        // 5 years
        tracing::warn!(
            "Large date range requested for {}: {} days ({} to {}). Consider pagination for production use.",
            query.coin_id,
            days_requested,
            start_date,
            end_date
        );
    }

    // Query database for historical prices
    let db_prices = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.eq(&query.coin_id))
        .filter(coins_historical_prices::Column::Date.gte(start_date))
        .filter(coins_historical_prices::Column::Date.lte(end_date))
        .order_by(coins_historical_prices::Column::Date, Order::Asc)
        .all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    tracing::info!(
        "Found {} price records for {} from {} to {}",
        db_prices.len(),
        query.coin_id,
        start_date,
        end_date
    );

    // Check if we have data - if not, attempt to fetch from CoinGecko
    if db_prices.is_empty() {
        tracing::warn!(
            "No historical data found for {} in database, attempting CoinGecko fetch",
            query.coin_id
        );

        // Attempt to fetch from CoinGecko API
        match fetch_from_coingecko_and_cache(
            &state,
            &query.coin_id,
            start_date,
            end_date,
        )
        .await
        {
            Ok(data_points) => {
                if data_points.is_empty() {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ErrorResponse {
                            error: format!(
                                "No historical data available for '{}' from {} to {}",
                                query.coin_id, start_date, end_date
                            ),
                        }),
                    ));
                }

                return Ok(Json(MarketCapHistoryResponse {
                    coin_id: query.coin_id,
                    symbol: coin.symbol,
                    data: data_points,
                }));
            }
            Err(e) => {
                let error_msg = e.to_string();
                tracing::error!("Failed to fetch from CoinGecko: {}", error_msg);

                // Check for rate limit errors
                let status = if error_msg.contains("429") || error_msg.to_lowercase().contains("rate limit") {
                    tracing::warn!("CoinGecko rate limit reached for {}", query.coin_id);
                    StatusCode::TOO_MANY_REQUESTS
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };

                let user_friendly_error = if status == StatusCode::TOO_MANY_REQUESTS {
                    format!("CoinGecko API rate limit reached. Please try again later.")
                } else {
                    format!("Failed to fetch data from CoinGecko: {}", error_msg)
                };

                return Err((
                    status,
                    Json(ErrorResponse {
                        error: user_friendly_error,
                    }),
                ));
            }
        }
    }

    // Convert database records to response format
    let mut data_points = Vec::new();
    for record in db_prices {
        // Convert Decimal to f64 with proper error handling
        let price = record.price.to_f64().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to convert price for {}", record.coin_id),
                }),
            )
        })?;

        // Market cap can legitimately be None for some coins - skip those records
        let market_cap = match record.market_cap.and_then(|mc| mc.to_f64()) {
            Some(mc) => mc,
            None => {
                tracing::debug!(
                    "Skipping record for {} on {} - no market cap data available",
                    record.coin_id,
                    record.date
                );
                continue;
            }
        };

        let volume_24h = record.volume
            .and_then(|v| v.to_f64())
            .unwrap_or(0.0);

        let date_time = record
            .date
            .and_hms_opt(0, 0, 0)
            .unwrap_or_else(|| record.date.and_hms_opt(0, 0, 1).unwrap())
            .and_utc();

        data_points.push(MarketCapDataPoint {
            date: date_time,
            market_cap,
            price,
            volume_24h,
        });
    }

    Ok(Json(MarketCapHistoryResponse {
        coin_id: query.coin_id,
        symbol: coin.symbol,
        data: data_points,
    }))
}

/// Fetch historical market cap data from CoinGecko API and cache in database
async fn fetch_from_coingecko_and_cache(
    state: &AppState,
    coin_id: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<MarketCapDataPoint>, Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!(
        "Fetching market cap history from CoinGecko for {} ({} to {})",
        coin_id,
        start_date,
        end_date
    );

    // Get coin symbol from database for caching
    let coin = Coins::find()
        .filter(coins::Column::CoinId.eq(coin_id))
        .one(&state.db)
        .await?;

    let symbol = coin.map(|c| c.symbol).unwrap_or_else(|| "UNKNOWN".to_string());

    // Convert dates to Unix timestamps
    let start_timestamp = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_timestamp = end_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    // Fetch from CoinGecko
    let data = state
        .coingecko
        .fetch_market_chart(coin_id, start_timestamp, end_timestamp)
        .await?;

    // Parse CoinGecko response and cache to database
    let mut data_points = Vec::new();

    // CoinGecko returns arrays: [[timestamp_ms, value], ...]
    // We need to align prices, market_caps, and total_volumes by date
    let market_caps = data.get("market_caps").and_then(|v| v.as_array());
    let prices = data.get("prices").and_then(|v| v.as_array());
    let total_volumes = data.get("total_volumes").and_then(|v| v.as_array());

    if let (Some(market_caps), Some(prices), Some(volumes)) =
        (market_caps, prices, total_volumes)
    {
        // Process data points (assuming all arrays have same length)
        for i in 0..market_caps.len() {
            if let (Some(mc_arr), Some(p_arr), Some(v_arr)) = (
                market_caps[i].as_array(),
                prices[i].as_array(),
                volumes[i].as_array(),
            ) {
                let timestamp_ms = mc_arr[0].as_i64().unwrap_or(0);
                let market_cap_val = mc_arr[1].as_f64().unwrap_or(0.0);
                let price_val = p_arr[1].as_f64().unwrap_or(0.0);
                let volume_24h = v_arr[1].as_f64().unwrap_or(0.0);

                // Convert timestamp to NaiveDate
                let date = chrono::DateTime::from_timestamp(timestamp_ms / 1000, 0)
                    .unwrap()
                    .naive_utc()
                    .date();

                // Create data point for response
                let date_time = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
                data_points.push(MarketCapDataPoint {
                    date: date_time,
                    market_cap: market_cap_val,
                    price: price_val,
                    volume_24h,
                });

                // Cache to database using upsert pattern (insert or update if exists)
                use rust_decimal::Decimal;
                use sea_orm::{ActiveModelTrait, Set};

                // Check if record already exists
                let existing = CoinsHistoricalPrices::find()
                    .filter(coins_historical_prices::Column::CoinId.eq(coin_id))
                    .filter(coins_historical_prices::Column::Date.eq(date))
                    .one(&state.db)
                    .await?;

                if let Some(existing_record) = existing {
                    // Update existing record
                    let mut active_model: coins_historical_prices::ActiveModel = existing_record.into();
                    active_model.price = Set(Decimal::from_f64_retain(price_val).unwrap_or_default());
                    active_model.market_cap = Set(Some(Decimal::from_f64_retain(market_cap_val).unwrap_or_default()));
                    active_model.volume = Set(Some(Decimal::from_f64_retain(volume_24h).unwrap_or_default()));

                    match active_model.update(&state.db).await {
                        Ok(_) => tracing::debug!("Updated cached data for {} on {}", coin_id, date),
                        Err(e) => tracing::warn!("Failed to update cache for {} on {}: {}", coin_id, date, e),
                    }
                } else {
                    // Insert new record
                    let new_record = coins_historical_prices::ActiveModel {
                        id: sea_orm::ActiveValue::NotSet,
                        coin_id: Set(coin_id.to_string()),
                        symbol: Set(symbol.clone()),
                        date: Set(date),
                        price: Set(Decimal::from_f64_retain(price_val).unwrap_or_default()),
                        market_cap: Set(Some(Decimal::from_f64_retain(market_cap_val).unwrap_or_default())),
                        volume: Set(Some(Decimal::from_f64_retain(volume_24h).unwrap_or_default())),
                        created_at: Set(Some(Utc::now().naive_utc())),
                    };

                    match new_record.insert(&state.db).await {
                        Ok(_) => tracing::debug!("Cached new data for {} on {}", coin_id, date),
                        Err(e) => tracing::warn!("Failed to cache data for {} on {}: {}", coin_id, date, e),
                    }
                }
            }
        }
    }

    tracing::info!(
        "Fetched and cached {} data points from CoinGecko for {}",
        data_points.len(),
        coin_id
    );

    Ok(data_points)
}

/// Handler for GET /api/market-cap/top-category
/// Retrieves top N cryptocurrencies by market capitalization within a specific CoinGecko category
pub async fn get_top_category(
    State(state): State<AppState>,
    Query(query): Query<TopCategoryQuery>,
) -> Result<Json<TopCategoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate query parameters
    if let Err(e) = query.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: e }),
        ));
    }

    let top = query.get_top();
    tracing::info!(
        "Fetching top {} coins for category: {} (date: {:?})",
        top,
        query.category_id,
        query.date
    );

    // Parse date with default to today (validate input before DB lookup)
    let today = Utc::now().date_naive();
    let target_date = if let Some(ref date_str) = query.date {
        NaiveDate::parse_from_str(date_str, "%Y-%m-%d").map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid date format: '{}'", date_str),
                }),
            )
        })?
    } else {
        today
    };

    // Validate date is not in the future (validate input before DB lookup)
    if target_date > today {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "date cannot be in the future".to_string(),
            }),
        ));
    }

    // Verify category exists in database
    let category = CoingeckoCategories::find()
        .filter(coingecko_categories::Column::CategoryId.eq(&query.category_id))
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    let category = category.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Category '{}' not found in system", query.category_id),
            }),
        )
    })?;

    tracing::info!(
        "Querying for top {} coins in category '{}' on date {}",
        top,
        query.category_id,
        target_date
    );

    // Find all coins in this category using category_membership table
    let category_coins = CategoryMembership::find()
        .filter(category_membership::Column::CategoryId.eq(&query.category_id))
        .all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    if category_coins.is_empty() {
        tracing::warn!("No coins found for category '{}'", query.category_id);
        return Ok(Json(TopCategoryResponse {
            category_id: query.category_id.clone(),
            category_name: category.name.clone(),
            date: target_date.to_string(),
            top,
            coins: vec![],
        }));
    }

    let coin_ids: Vec<String> = category_coins.iter().map(|cm| cm.coin_id.clone()).collect();
    tracing::info!(
        "Found {} coins in category '{}'",
        coin_ids.len(),
        query.category_id
    );

    // Batch fetch all coins and prices to avoid N+1 queries
    use std::collections::HashMap;

    // Batch query 1: Get all coins info
    let all_coins = Coins::find()
        .filter(coins::Column::CoinId.is_in(coin_ids.clone()))
        .all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    // Build coin lookup map
    let coin_map: HashMap<String, crate::entities::coins::Model> = all_coins
        .into_iter()
        .map(|c| (c.coin_id.clone(), c))
        .collect();

    // Batch query 2: Get all historical prices for target date
    let all_prices = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.is_in(coin_ids))
        .filter(coins_historical_prices::Column::Date.eq(target_date))
        .all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    // Build results by joining in memory
    let mut coin_data = Vec::new();
    for record in all_prices {
        // Look up coin info from map
        let coin = match coin_map.get(&record.coin_id) {
            Some(c) => c,
            None => {
                tracing::debug!("Coin {} not found in coins table, skipping", record.coin_id);
                continue;
            }
        };

        // Convert Decimal to f64 with validation
        let price = match record.price.to_f64() {
            Some(p) => p,
            None => {
                tracing::warn!("Failed to convert price for {} on {}, skipping", record.coin_id, target_date);
                continue;
            }
        };

        let market_cap = match record.market_cap.and_then(|mc| mc.to_f64()) {
            Some(mc) if mc > 0.0 => mc,
            _ => {
                tracing::debug!("Skipping {} - no valid market cap data on {}", record.coin_id, target_date);
                continue;
            }
        };

        let volume_24h = record.volume
            .and_then(|v| v.to_f64())
            .unwrap_or(0.0); // Volume can default to 0 if missing

        coin_data.push(TopCategoryCoin {
            rank: 0, // Will be set after sorting
            coin_id: record.coin_id.clone(),
            symbol: coin.symbol.clone(),
            name: coin.name.clone(),
            market_cap,
            price,
            volume_24h,
            logo: None, // Historical data doesn't have logos
        });
    }

    // Sort by market cap descending
    coin_data.sort_by(|a, b| {
        b.market_cap
            .partial_cmp(&a.market_cap)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit to top N and assign ranks
    let top_coins: Vec<TopCategoryCoin> = coin_data
        .into_iter()
        .take(top as usize)
        .enumerate()
        .map(|(i, mut coin)| {
            coin.rank = (i + 1) as u32;
            coin
        })
        .collect();

    tracing::info!(
        "Returning {} top coins for category '{}' on {}",
        top_coins.len(),
        query.category_id,
        target_date
    );

    Ok(Json(TopCategoryResponse {
        category_id: query.category_id,
        category_name: category.name,
        date: target_date.to_string(),
        top,
        coins: top_coins,
    }))
}

/// Query parameters for live category data endpoint
#[derive(Debug, serde::Deserialize)]
pub struct LiveCategoryQuery {
    pub category_id: String,
    #[serde(default = "default_top")]
    pub top: u32,
}

fn default_top() -> u32 {
    50
}

/// Handler for GET /api/market-cap/live-category
/// Fetches LIVE market data from CoinGecko for a category (no historical data required)
pub async fn get_live_category(
    State(state): State<AppState>,
    Query(query): Query<LiveCategoryQuery>,
) -> Result<Json<TopCategoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    let top = query.top.min(250); // CoinGecko max per page

    tracing::info!(
        "Fetching LIVE top {} coins for category: {}",
        top,
        query.category_id
    );

    // Fetch live data from CoinGecko
    let market_data = state
        .coingecko
        .fetch_category_market_data(&query.category_id, top)
        .await
        .map_err(|e| {
            tracing::error!("CoinGecko API error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to fetch category data: {}", e),
                }),
            )
        })?;

    if market_data.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("No coins found in category '{}'", query.category_id),
            }),
        ));
    }

    // Convert to TopCategoryCoin format
    let coins: Vec<TopCategoryCoin> = market_data
        .into_iter()
        .enumerate()
        .map(|(i, coin)| TopCategoryCoin {
            rank: (i + 1) as u32,
            coin_id: coin.id,
            symbol: coin.symbol,
            name: coin.name,
            market_cap: coin.market_cap.unwrap_or(0.0),
            price: coin.current_price.unwrap_or(0.0),
            volume_24h: coin.total_volume.unwrap_or(0.0),
            logo: Some(coin.image),
        })
        .collect();

    tracing::info!(
        "Returning {} live coins for category '{}'",
        coins.len(),
        query.category_id
    );

    Ok(Json(TopCategoryResponse {
        category_id: query.category_id,
        category_name: "".to_string(), // CoinGecko doesn't return category name in this endpoint
        date: Utc::now().date_naive().to_string(),
        top,
        coins,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        Router,
        routing::get,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use sea_orm::{Database, DatabaseConnection};
    use crate::services::coingecko::CoinGeckoService;
    use crate::services::exchange_api::ExchangeApiService;

    async fn setup_test_app() -> Router {
        // Load environment for DATABASE_URL
        dotenvy::dotenv().ok();
        
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set for integration tests");
        
        let db = Database::connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        let coingecko_api_key = std::env::var("COINGECKO_API_KEY")
            .unwrap_or_else(|_| "test_key".to_string());
        let coingecko_base_url = std::env::var("COINGECKO_BASE_URL")
            .unwrap_or_else(|_| "https://pro-api.coingecko.com/api/v3".to_string());
        
        let coingecko = CoinGeckoService::new(coingecko_api_key, coingecko_base_url);
        let exchange_api = ExchangeApiService::new(600);
        let itp_listing = crate::services::itp_listing::ItpListingService::new();
        let realtime_prices = crate::services::realtime_prices::RealTimePriceService::new(60);

        let state = AppState {
            db,
            coingecko,
            exchange_api,
            itp_listing,
            realtime_prices,
        };

        Router::new()
            .route("/api/market-cap/history", get(get_market_cap_history))
            .route("/api/market-cap/top-category", get(get_top_category))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_market_cap_history_invalid_date_format() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/history?coin_id=bitcoin&start_date=2024/01/01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Invalid start_date format"));
    }

    #[tokio::test]
    async fn test_market_cap_history_invalid_coin_id() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/history?coin_id=")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("coin_id cannot be empty"));
    }

    #[tokio::test]
    async fn test_market_cap_history_coin_not_found() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/history?coin_id=nonexistent-coin-xyz-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("not found in system"));
    }

    #[tokio::test]
    async fn test_market_cap_history_future_end_date() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/history?coin_id=bitcoin&end_date=2030-01-01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("cannot be in the future"));
    }

    #[tokio::test]
    async fn test_market_cap_history_start_after_end() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/history?coin_id=bitcoin&start_date=2024-12-31&end_date=2024-01-01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("start_date must be before"));
    }

    #[tokio::test]
    async fn test_market_cap_history_valid_request_with_bitcoin() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/history?coin_id=bitcoin&start_date=2024-01-01&end_date=2024-01-05")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should be 200 OK or 404 if no data (both acceptable for integration test)
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::NOT_FOUND,
            "Expected 200 OK or 404 NOT FOUND, got: {}",
            response.status()
        );

        if response.status() == StatusCode::OK {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert!(body_str.contains("coin_id"));
            assert!(body_str.contains("symbol"));
            assert!(body_str.contains("data"));
        }
    }

    #[tokio::test]
    async fn test_market_cap_history_default_date_range() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/history?coin_id=bitcoin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should handle default 365 days range
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::NOT_FOUND,
            "Expected 200 OK or 404, got: {}",
            response.status()
        );
    }

    #[tokio::test]
    async fn test_top_category_invalid_category_id() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("category_id cannot be empty"));
    }

    #[tokio::test]
    async fn test_top_category_top_out_of_range_below() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=defi&top=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("between 1 and 250"));
    }

    #[tokio::test]
    async fn test_top_category_top_out_of_range_above() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=defi&top=251")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("between 1 and 250"));
    }

    #[tokio::test]
    async fn test_top_category_future_date() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=defi&date=2030-01-01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("cannot be in the future"));
    }

    #[tokio::test]
    async fn test_top_category_invalid_date_format() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=defi&date=2024/01/01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Invalid date format"));
    }

    #[tokio::test]
    async fn test_top_category_not_found() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=nonexistent-category-xyz-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("not found in system"));
    }

    #[tokio::test]
    async fn test_top_category_default_top_value() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=decentralized-finance-defi")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should be 200 OK (empty array) or 404 if category doesn't exist
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::NOT_FOUND,
            "Expected 200 OK or 404, got: {}",
            response.status()
        );

        if response.status() == StatusCode::OK {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert!(body_str.contains("category_id"));
            assert!(body_str.contains("coins"));
            // Default top should be 10
            assert!(body_str.contains("\"top\":10"));
        }
    }

    #[tokio::test]
    async fn test_top_category_custom_top_and_date() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=decentralized-finance-defi&top=25&date=2024-06-01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should be 200 OK (empty or with data) or 404 if category doesn't exist
        assert!(
            response.status() == StatusCode::OK || response.status() == StatusCode::NOT_FOUND,
            "Expected 200 OK or 404, got: {}",
            response.status()
        );

        if response.status() == StatusCode::OK {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert!(body_str.contains("\"top\":25"));
            assert!(body_str.contains("2024-06-01"));
        }
    }

    #[tokio::test]
    async fn test_top_category_valid_response_structure() {
        let response = setup_test_app().await
            .oneshot(
                Request::builder()
                    .uri("/api/market-cap/top-category?category_id=layer-1&top=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        if response.status() == StatusCode::OK {
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            
            // Verify response structure
            assert!(json.get("category_id").is_some());
            assert!(json.get("category_name").is_some());
            assert!(json.get("date").is_some());
            assert!(json.get("top").is_some());
            assert!(json.get("coins").is_some());
            assert!(json["coins"].is_array());
            
            // If coins exist, verify coin structure
            if let Some(coins) = json["coins"].as_array() {
                if !coins.is_empty() {
                    let first_coin = &coins[0];
                    assert!(first_coin.get("rank").is_some());
                    assert!(first_coin.get("coin_id").is_some());
                    assert!(first_coin.get("symbol").is_some());
                    assert!(first_coin.get("name").is_some());
                    assert!(first_coin.get("market_cap").is_some());
                    assert!(first_coin.get("price").is_some());
                    assert!(first_coin.get("volume_24h").is_some());
                    
                    // Verify rank starts at 1
                    assert_eq!(first_coin["rank"], 1);
                }
            }
        }
    }
}
