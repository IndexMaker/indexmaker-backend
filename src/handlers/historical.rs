use axum::{extract::State, http::StatusCode, Json, extract::Path};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use crate::models::historical::{HistoricalDataResponse, HistoricalEntry};
use crate::models::token::ErrorResponse;
use crate::AppState;

pub async fn fetch_coin_historical_data(
    State(state): State<AppState>,
    Path(coin_id): Path<String>,
) -> Result<Json<HistoricalDataResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Default date range: 2019-01-01 to today
    let start_date = Utc
        .from_utc_datetime(
            &NaiveDate::from_ymd_opt(2019, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        );
    let end_date = Utc::now();

    // Fetch data from CoinGecko
    let coin_price_data = state
        .coingecko
        .get_token_market_chart(&coin_id, "usd", 3000)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to fetch CoinGecko data: {}", e),
                }),
            )
        })?;

    // Convert timestamps to seconds
    let end_timestamp = end_date.timestamp();
    let start_timestamp = start_date.timestamp();

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
