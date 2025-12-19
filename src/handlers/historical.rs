use axum::extract::Query;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::IntoResponse;
use axum::{extract::State, http::StatusCode, Json, extract::Path};
use chrono::{DateTime, NaiveDate, Utc};

use reqwest::header;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder};
use std::collections::{HashMap, HashSet};
use rust_decimal::prelude::ToPrimitive;

use crate::entities::{coins_historical_prices, daily_prices, prelude::*, rebalances};
use crate::models::historical::{ChartDataEntry, DailyPriceDataEntry, HistoricalDataResponse, HistoricalEntry, IndexHistoricalDataQuery, IndexHistoricalDataResponse};
use crate::models::token::ErrorResponse;
use crate::AppState;
use crate::services::coingecko::CoinGeckoService;
use crate::services::price_utils::get_coins_historical_price_for_date;
use crate::services::rebalancing::CoinRebalanceInfo;

pub async fn fetch_coin_historical_data(
    State(state): State<AppState>,
    Path(coin_id): Path<String>,
) -> Result<Json<HistoricalDataResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Default date range: from 2019-01-01 to today
    let end_date = Utc::now().date_naive();
    let start_date = NaiveDate::from_ymd_opt(2019, 1, 1)
        .unwrap_or_else(|| end_date - chrono::Duration::days(365));

    // Query from NEW table: coins_historical_prices
    let db_prices = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.eq(&coin_id))
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

    // If no data found, return error with helpful message
    if db_prices.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!(
                    "No historical price data found for coin_id '{}' in the last 365 days. \
                    Please ensure the coin exists in the system and has price data synced.",
                    coin_id
                ),
            }),
        ));
    }

    tracing::info!(
        "Found {} price records for {} (last 365 days)",
        db_prices.len(),
        coin_id
    );

    // Build price map for efficient lookup
    let mut price_map = HashMap::new();
    let coin_name = coin_id.clone();
    
    for record in db_prices {
        let date_str = record.date.format("%Y-%m-%d").to_string();
        // Convert Decimal to f64
        let price_f64 = record.price.to_string().parse::<f64>()
            .unwrap_or(0.0);
        price_map.insert(date_str, price_f64);
    }

    // Calculate normalized values
    let mut historical_data = Vec::new();
    let mut base_value = 10000.0; // Starting value (100%)
    let mut prev_price: Option<f64> = None;

    // Iterate through each day in range
    let mut current_date = start_date;
    while current_date <= end_date {
        let date_str = current_date.format("%Y-%m-%d").to_string();

        if let Some(&price) = price_map.get(&date_str) {
            // Calculate normalized value
            if let Some(prev) = prev_price {
                base_value = base_value * (price / prev);
            }
            prev_price = Some(price);

            // Convert to DateTime for response
            let date_time = current_date
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();

            historical_data.push(HistoricalEntry {
                name: coin_name.clone(),
                date: date_time,
                price,
                value: base_value,
            });
        }

        current_date = current_date + chrono::Duration::days(1);
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

    // Get date range
    let dates: Vec<NaiveDate> = parsed_data
        .iter()
        .map(|(date, _, _)| *date)
        .collect();

    let min_date = *dates.iter().min().unwrap();
    let max_date = *dates.iter().max().unwrap();

    // Query coins_historical_prices in batch (using DATE column now!)
    let coin_id_list: Vec<String> = all_coin_ids.into_iter().collect();

    let historical = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.is_in(coin_id_list))
        .filter(coins_historical_prices::Column::Date.between(
            min_date - chrono::Duration::days(1),
            max_date + chrono::Duration::days(1),
        ))
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

    // Build price map: { coinId => [(date, price)] }
    let mut price_map: HashMap<String, Vec<(NaiveDate, f64)>> = HashMap::new();
    for row in historical {
        // Convert Decimal to f64 using ToPrimitive trait
        if let Some(price_f64) = row.price.to_f64() {
            price_map
                .entry(row.coin_id)
                .or_insert_with(Vec::new)
                .push((row.date, price_f64));
        } else {
            tracing::warn!("Failed to convert price to f64 for coin {}", row.coin_id);
        }
    }

    // Sort each coin's price list by date
    for list in price_map.values_mut() {
        list.sort_by_key(|&(date, _)| date);
    }

    // Build final output
    let results: Vec<DailyPriceDataEntry> = parsed_data
        .into_iter()
        .map(|(date, price, quantities)| {
            let mut daily_coin_prices = HashMap::new();
            for coin_id in quantities.keys() {
                if let Some(price_val) = find_nearest_price_by_date(&price_map, coin_id, date) {
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

fn find_nearest_price_by_date(
    price_map: &HashMap<String, Vec<(NaiveDate, f64)>>,
    coin_id: &str,
    target_date: NaiveDate,
) -> Option<f64> {
    let prices = price_map.get(coin_id)?;
    if prices.is_empty() {
        return None;
    }

    // Try exact match first
    for (date, price) in prices {
        if *date == target_date {
            return Some(*price);
        }
    }

    // Find nearest date within ±3 days
    let mut nearest: Option<(NaiveDate, f64)> = None;
    let mut min_diff = i64::MAX;

    for (date, price) in prices {
        let diff = (*date - target_date).num_days().abs();
        
        // Only consider dates within ±3 days
        if diff <= 3 && diff < min_diff {
            min_diff = diff;
            nearest = Some((*date, *price));
        }
    }

    nearest.map(|(_, price)| price)
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
    Query(params): Query<IndexHistoricalDataQuery>,
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

    // Parse start_date from query params or use initial_date
    let start_date = if let Some(start_str) = params.start_date {
        NaiveDate::parse_from_str(&start_str, "%Y-%m-%d").map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid start_date format. Use YYYY-MM-DD".to_string(),
                }),
            )
        })?
    } else {
        initial_date
    };

    // Parse end_date from query params or use today
    let end_date = if let Some(end_str) = params.end_date {
        NaiveDate::parse_from_str(&end_str, "%Y-%m-%d").map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid end_date format. Use YYYY-MM-DD".to_string(),
                }),
            )
        })?
    } else {
        today
    };

    // Validate date range
    if start_date < initial_date {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "start_date cannot be before index initial_date ({})",
                    initial_date
                ),
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

    if start_date > end_date {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "start_date must be before or equal to end_date".to_string(),
            }),
        ));
    }

    tracing::info!(
        "Fetching historical data for index {} ({}) from {} to {}",
        index_id,
        index.symbol,
        start_date,
        end_date
    );

    // Fetch historical data from daily_prices table
    let chart_data = get_chart_data_from_daily_prices(
        &state.db,
        index_id,
        &index.name,
        start_date,
        end_date,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to fetch historical data: {}", e),
            }),
        )
    })?;

    Ok(Json(IndexHistoricalDataResponse {
        name: index.name.clone(),
        index_id,
        chart_data,
        formatted_transactions: vec![], // Empty for now
    }))
}

/// Fetch chart data from daily_prices table and calculate cumulative returns
async fn get_chart_data_from_daily_prices(
    db: &DatabaseConnection,
    index_id: i32,
    index_name: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<ChartDataEntry>, Box<dyn std::error::Error + Send + Sync>> {
    // Query daily_prices table
    let existing_prices = DailyPrices::find()
        .filter(daily_prices::Column::IndexId.eq(index_id.to_string()))
        .filter(daily_prices::Column::Date.gte(start_date))
        .filter(daily_prices::Column::Date.lte(end_date))
        .order_by(daily_prices::Column::Date, Order::Asc)
        .all(db)
        .await?;

    if existing_prices.is_empty() {
        return Err(format!(
            "No historical prices found for index {} from {} to {}. Backfilling may still be in progress.",
            index_id, start_date, end_date
        ).into());
    }

    tracing::info!(
        "Found {} daily prices for index {} from {} to {}",
        existing_prices.len(),
        index_id,
        start_date,
        end_date
    );

    // Calculate cumulative returns starting from base value 10000
    let mut base_value = 10000.0;
    let mut chart_data = Vec::new();

    for (i, price_row) in existing_prices.iter().enumerate() {
        // Convert Decimal to f64
        let price: f64 = price_row.price.to_string().parse().unwrap_or(0.0);

        if i == 0 {
            // First entry: value = base_value (10000)
            chart_data.push(ChartDataEntry {
                name: index_name.to_string(),
                date: price_row.date.and_hms_opt(0, 0, 0).unwrap().and_utc(),
                price,
                value: base_value,
            });
        } else {
            // Calculate return percentage from previous day
            let prev_price: f64 = existing_prices[i - 1]
                .price
                .to_string()
                .parse()
                .unwrap_or(0.0);

            if prev_price > 0.0 {
                let return_pct = (price - prev_price) / prev_price;
                base_value = base_value * (1.0 + return_pct);
            }

            chart_data.push(ChartDataEntry {
                name: index_name.to_string(),
                date: price_row.date.and_hms_opt(0, 0, 0).unwrap().and_utc(),
                price,
                value: base_value,
            });
        }
    }

    tracing::debug!(
        "Calculated cumulative returns for {} days (final value: {})",
        chart_data.len(),
        base_value
    );

    Ok(chart_data)
}

