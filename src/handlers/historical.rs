use axum::http::{HeaderMap, HeaderValue};
use axum::response::IntoResponse;
use axum::{extract::State, http::StatusCode, Json, extract::Path};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use reqwest::header;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};
use std::collections::{HashMap, HashSet};

use crate::entities::{daily_prices, historical_prices, prelude::*, rebalances};
use crate::models::historical::{DailyPriceDataEntry, HistoricalDataResponse, HistoricalEntry, IndexHistoricalDataResponse};
use crate::models::token::ErrorResponse;
use crate::AppState;
use crate::services::rebalancing::CoinRebalanceInfo;

pub async fn fetch_coin_historical_data(
    State(state): State<AppState>,
    Path(coin_id): Path<String>,
) -> Result<Json<HistoricalDataResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Default date range: last 365 days
    let end_date = Utc::now().date_naive();
    let start_date = end_date - chrono::Duration::days(365);

    // First, try to get data from database
    let db_prices = HistoricalPrices::find()
        .filter(historical_prices::Column::CoinId.eq(&coin_id))
        .filter(historical_prices::Column::Timestamp.gte(
            start_date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()
        ))
        .filter(historical_prices::Column::Timestamp.lte(
            end_date.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp()
        ))
        .order_by(historical_prices::Column::Timestamp, Order::Asc)
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

    let coin_price_data = if !db_prices.is_empty() {
        tracing::info!("Using {} cached prices for {} from database", db_prices.len(), coin_id);
        
        // Convert DB prices to (timestamp_ms, price) format
        db_prices
            .into_iter()
            .map(|p| (p.timestamp as i64 * 1000, p.price))
            .collect()
    } else {
        tracing::info!("No cached prices for {}, fetching from CoinGecko (365 days)", coin_id);
        
        // Fetch from CoinGecko (365 days only)
        let prices = state
            .coingecko
            .get_token_market_chart(&coin_id, "usd", 365)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to fetch CoinGecko data: {}", e),
                    }),
                )
            })?;

        // Store in database for future use (background task)
        let db_clone = state.db.clone();
        let coingecko_clone = state.coingecko.clone();
        let coin_id_clone = coin_id.clone();
        
        tokio::spawn(async move {
            use crate::jobs::historical_prices_sync::fetch_and_store_prices;
            
            if let Err(e) = fetch_and_store_prices(
                &db_clone, 
                &coingecko_clone, 
                &coin_id_clone, 
                start_date, 
                end_date
            ).await {
                tracing::error!("Failed to store prices for {}: {}", coin_id_clone, e);
            } else {
                tracing::info!("Stored prices for {} in background", coin_id_clone);
            }
        });

        prices
    };

    // Convert timestamps to seconds
    let start_timestamp = start_date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    let end_timestamp = end_date.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp();

    // Create price map for efficient lookup
    let mut price_map = std::collections::HashMap::new();
    for (timestamp_ms, price) in coin_price_data {
        let date_str = DateTime::from_timestamp_millis(timestamp_ms)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        price_map.insert(date_str, price);
    }

    // Calculate normalized values
    let mut historical_data = Vec::new();
    let mut base_value = 10000.0; // Starting value (100%)
    let mut prev_price: Option<f64> = None;

    // Iterate through each day
    let mut current_ts = start_timestamp;
    while current_ts <= end_timestamp {
        let date = DateTime::from_timestamp(current_ts, 0).unwrap();
        let date_str = date.format("%Y-%m-%d").to_string();

        if let Some(&price) = price_map.get(&date_str) {
            // Calculate normalized value
            if let Some(prev) = prev_price {
                base_value = base_value * (price / prev);
            }
            prev_price = Some(price);

            // Get coin name
            let name = match coin_id.as_str() {
                "bitcoin" => "Bitcoin (BTC)",
                "ethereum" => "Ethereum (ETH)",
                _ => &coin_id,
            };

            historical_data.push(HistoricalEntry {
                name: name.to_string(),
                date,
                price,
                value: base_value,
            });
        }

        current_ts += 86400; // Add one day
    }

    Ok(Json(HistoricalDataResponse {
        data: historical_data,
    }))
}

pub async fn download_daily_price_data(
    State(state): State<AppState>,
    Path(index_id): Path<i32>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    // Get daily price data
    let daily_price_data = get_daily_price_data(&state, index_id).await?;

    if daily_price_data.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "No daily price data found for this index".to_string(),
            }),
        ));
    }

    // Generate CSV
    let csv_content = generate_csv(daily_price_data);

    // Create headers
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv"));
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"daily_price_data_{}.csv\"",
            index_id
        ))
        .unwrap(),
    );

    // Return as downloadable file
    Ok((headers, csv_content))
}

async fn get_daily_price_data(
    state: &AppState,
    index_id: i32,
) -> Result<Vec<DailyPriceDataEntry>, (StatusCode, Json<ErrorResponse>)> {
    // Get index metadata
    let index_data = IndexMetadata::find_by_id(index_id)
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Index {} not found", index_id),
                }),
            )
        })?;

    // Query daily_prices
    let existing_prices = DailyPrices::find()
        .filter(daily_prices::Column::IndexId.eq(index_id.to_string()))
        .order_by(daily_prices::Column::Date, Order::Asc)
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

    if existing_prices.is_empty() {
        return Ok(vec![]);
    }

    // Parse quantities and collect all unique coin IDs
    let mut all_coin_ids = HashSet::new();
    let parsed_data: Vec<_> = existing_prices
        .into_iter()
        .map(|row| {
            let quantities: HashMap<String, f64> = row
                .quantities
                .as_ref()
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            for coin_id in quantities.keys() {
                all_coin_ids.insert(coin_id.clone());
            }

            (row.date, row.price, quantities)
        })
        .collect();

    // Get timestamp range
    let timestamps: Vec<i64> = parsed_data
        .iter()
        .map(|(date, _, _)| date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
        .collect();

    let min_timestamp = *timestamps.iter().min().unwrap_or(&0);
    let max_timestamp = *timestamps.iter().max().unwrap_or(&0);

    // Query historical prices in batch
    let coin_id_list: Vec<String> = all_coin_ids.into_iter().collect();

    let historical = HistoricalPrices::find()
        .filter(historical_prices::Column::CoinId.is_in(coin_id_list))
        .filter(historical_prices::Column::Timestamp.between(min_timestamp - 86400, max_timestamp + 86400))
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

    // Build price map: { coinId => [(timestamp, price)] }
    let mut price_map: HashMap<String, Vec<(i64, f64)>> = HashMap::new();
    for row in historical {
        price_map
            .entry(row.coin_id)
            .or_insert_with(Vec::new)
            .push((row.timestamp as i64, row.price));
    }

    // Sort each coin's price list by timestamp
    for list in price_map.values_mut() {
        list.sort_by_key(|&(ts, _)| ts);
    }

    // Build final output
    let results: Vec<DailyPriceDataEntry> = parsed_data
        .into_iter()
        .map(|(date, price, quantities)| {
            let target_ts = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

            let mut daily_coin_prices = HashMap::new();
            for coin_id in quantities.keys() {
                if let Some(price_val) = find_nearest_price(&price_map, coin_id, target_ts) {
                    daily_coin_prices.insert(coin_id.clone(), price_val);
                }
            }

            DailyPriceDataEntry {
                index: index_data.name.clone(),
                index_id,
                date: date.format("%Y-%m-%d").to_string(),
                quantities,
                price: price.to_string().parse().unwrap_or(0.0),
                value: price.to_string().parse().unwrap_or(0.0),
                coin_prices: daily_coin_prices,
            }
        })
        .collect();

    Ok(results)
}

fn find_nearest_price(
    price_map: &HashMap<String, Vec<(i64, f64)>>,
    coin_id: &str,
    target_ts: i64,
) -> Option<f64> {
    let prices = price_map.get(coin_id)?;
    if prices.is_empty() {
        return None;
    }

    // Binary search for nearest timestamp
    let mut left = 0;
    let mut right = prices.len() - 1;

    while left < right {
        let mid = (left + right) / 2;
        if prices[mid].0 < target_ts {
            left = mid + 1;
        } else {
            right = mid;
        }
    }

    let best = prices[left];

    // Compare with previous to find closer one
    if left > 0 {
        let prev = prices[left - 1];
        if (prev.0 - target_ts).abs() < (best.0 - target_ts).abs() {
            return Some(prev.1);
        }
    }

    Some(best.1)
}

fn generate_csv(data: Vec<DailyPriceDataEntry>) -> String {
    let mut csv = String::new();

    // Headers
    csv.push_str("Index,IndexId,Date,Price,Asset Quantities,Asset Prices\n");

    // Data rows
    for entry in data {
        let quantities = serde_json::to_string(&entry.quantities)
            .unwrap_or_default()
            .replace('"', "");

        let coin_prices = serde_json::to_string(&entry.coin_prices)
            .unwrap_or_default()
            .replace('"', "");

        csv.push_str(&format!(
            "{},{},{},{},\"{}\",\"{}\"\n",
            entry.index, entry.index_id, entry.date, entry.price, quantities, coin_prices
        ));
    }

    csv
}

pub async fn fetch_index_historical_data(
    State(state): State<AppState>,
    Path(index_id): Path<i32>,
) -> Result<Json<IndexHistoricalDataResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Get index metadata
    let index = IndexMetadata::find_by_id(index_id)
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Index {} not found", index_id),
                }),
            )
        })?;

    // Validate dates
    let initial_date = index.initial_date.ok_or((
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: "Index has no initial_date".to_string(),
        }),
    ))?;

    let today = Utc::now().date_naive();

    // Date range: from initial_date to today
    let start_date = initial_date;
    let end_date = today;

    tracing::info!(
        "Fetching historical data for index {} from {} to {}",
        index_id,
        start_date,
        end_date
    );

    // Calculate index prices for each day
    let historical_data = calculate_index_historical_prices(
        &state.db,
        index_id,
        start_date,
        end_date,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to calculate historical prices: {}", e),
            }),
        )
    })?;

    Ok(Json(IndexHistoricalDataResponse {
        data: historical_data,
    }))
}

/// Calculate index historical prices for a date range
async fn calculate_index_historical_prices(
    db: &DatabaseConnection,
    index_id: i32,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<(i64, f64)>, Box<dyn std::error::Error + Send + Sync>> {
    let mut result = Vec::new();

    // Get all rebalances for this index
    let rebalances_list = Rebalances::find()
        .filter(rebalances::Column::IndexId.eq(index_id))
        .order_by(rebalances::Column::Timestamp, Order::Asc)
        .all(db)
        .await?;

    if rebalances_list.is_empty() {
        return Err("No rebalances found for this index. Backfilling may still be in progress.".into());
    }

    // Iterate through each day
    let mut current_date = start_date;
    while current_date <= end_date {
        let timestamp = current_date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();

        // Find the nearest rebalance before or on this date
        let nearest_rebalance = rebalances_list
            .iter()
            .filter(|r| r.timestamp <= timestamp)
            .max_by_key(|r| r.timestamp);

        if let Some(rebalance) = nearest_rebalance {
            // Parse coins from rebalance
            let coins: Vec<CoinRebalanceInfo> = serde_json::from_value(rebalance.coins.clone())?;

            // Calculate index price for this date
            let mut index_price = 0.0;
            let mut has_all_prices = true;

            for coin in coins {
                // Get price for this coin on this date
                let price_opt = get_price_for_date_helper(db, &coin.coin_id, current_date).await?;

                match price_opt {
                    Some(price) => {
                        let weight: f64 = coin.weight.parse().unwrap_or(0.0);
                        let quantity: f64 = coin.quantity.parse().unwrap_or(0.0);
                        
                        index_price += weight * quantity * price;
                    }
                    None => {
                        tracing::warn!(
                            "Missing price for {} on {}, skipping this date",
                            coin.coin_id,
                            current_date
                        );
                        has_all_prices = false;
                        break;
                    }
                }
            }

            // Only add if we have all prices
            if has_all_prices {
                result.push((timestamp, index_price));
            }
        } else {
            tracing::debug!("No rebalance found before {}, skipping", current_date);
        }

        current_date = current_date + chrono::Duration::days(1);
    }

    Ok(result)
}

/// Helper to get price for a coin on a specific date
async fn get_price_for_date_helper(
    db: &DatabaseConnection,
    coin_id: &str,
    date: NaiveDate,
) -> Result<Option<f64>, Box<dyn std::error::Error + Send + Sync>> {
    let target_timestamp = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

    // Try exact match first
    let exact_match = HistoricalPrices::find()
        .filter(historical_prices::Column::CoinId.eq(coin_id))
        .filter(historical_prices::Column::Timestamp.eq(target_timestamp))
        .one(db)
        .await?;

    if let Some(price_row) = exact_match {
        return Ok(Some(price_row.price));
    }

    // Fall back to nearest price within ±3 days
    let start_timestamp = (date - chrono::Duration::days(3))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_timestamp = (date + chrono::Duration::days(3))
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    let nearest = HistoricalPrices::find()
        .filter(historical_prices::Column::CoinId.eq(coin_id))
        .filter(historical_prices::Column::Timestamp.gte(start_timestamp))
        .filter(historical_prices::Column::Timestamp.lte(end_timestamp))
        .order_by(historical_prices::Column::Timestamp, Order::Asc)
        .limit(1)
        .one(db)
        .await?;

    Ok(nearest.map(|p| p.price))
}
