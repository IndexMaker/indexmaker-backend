use axum::extract::Path;
use chrono::{Datelike, Duration, NaiveDate, Utc};

use axum::{extract::{State, Query}, http::StatusCode, Json};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set};

use crate::{entities::{blockchain_events, coingecko_categories, coins, daily_prices, index_metadata, prelude::*, rebalances}, models::index::{ConstituentWeight, CurrentIndexWeightResponse, IndexLastPriceResponse, RemoveIndexRequest, RemoveIndexResponse}, services::coingecko::CoinGeckoService};
use crate::models::index::{
    CollateralToken, CreateIndexRequest, CreateIndexResponse, IndexConfigResponse, IndexListEntry, IndexListResponse, Performance, Ratings
};
use crate::models::index::{IndexPriceAtDateRequest, IndexPriceAtDateResponse, ConstituentPriceInfo};
use crate::services::rebalancing::CoinRebalanceInfo;

use crate::models::token::ErrorResponse;
use crate::AppState;
use crate::services::rebalancing::RebalancingService;



/// Shared calculation logic for index price
async fn calculate_index_price_internal(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    index_id: i32,
    target_date: NaiveDate,
) -> Result<
    (i64, f64, Vec<ConstituentPriceInfo>),
    (StatusCode, Json<ErrorResponse>),
> {
    // Get index metadata
    let index = IndexMetadata::find_by_id(index_id)
        .one(db)
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

    // Validate index has initial_date
    let initial_date = index.initial_date.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Index has no initial_date configured".to_string(),
            }),
        )
    })?;

    // Check if date is before index inception
    if target_date < initial_date {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "Date {} is before index inception date ({})",
                    target_date, initial_date
                ),
            }),
        ));
    }

    // Get last rebalance before or on target date (this is T0)
    let target_timestamp = target_date.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp();

    let last_rebalance = Rebalances::find()
        .filter(rebalances::Column::IndexId.eq(index_id))
        .filter(rebalances::Column::Timestamp.lte(target_timestamp))
        .order_by(rebalances::Column::Timestamp, Order::Desc)
        .limit(1)
        .one(db)
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
                    error: format!("No rebalance found on or before {}", target_date),
                }),
            )
        })?;

    // T0 timestamp and date (rebalance date)
    let t0_timestamp = last_rebalance.timestamp;
    let t0_date = chrono::DateTime::from_timestamp(t0_timestamp, 0)
        .unwrap()
        .date_naive();

    // Index price at T0 (from rebalance)
    let index_price_t0: f64 = last_rebalance.portfolio_value.to_string().parse().unwrap_or(0.0);

    tracing::debug!(
        "Calculating price for index {} on {} (last rebalance: {}, base price: {})",
        index_id,
        target_date,
        t0_date,
        index_price_t0
    );

    // Parse constituents from rebalance (these have Price_T0 stored)
    let coins: Vec<CoinRebalanceInfo> = serde_json::from_value(last_rebalance.coins)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to parse rebalance data: {}", e),
                }),
            )
        })?;

    // Calculate price change contribution for each constituent
    let mut constituent_prices = Vec::new();
    let mut total_price_change = 0.0;

    for coin in coins {
        // Price at T0 (stored in rebalance)
        let price_t0 = coin.price;

        // Quantity (from rebalance)
        let quantity: f64 = coin.quantity.parse().unwrap_or(0.0);
        let weight: f64 = coin.weight.parse().unwrap_or(0.0);

        // Get price at T1 (target date)
        let price_t1 = match get_or_fetch_price(db, coingecko, &coin.coin_id, target_date).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(
                    "Failed to get price for {} ({}) on {}: {}",
                    coin.symbol,
                    coin.coin_id,
                    target_date,
                    e
                );
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!(
                            "Failed to get price for {} on {}: {}",
                            coin.symbol, target_date, e
                        ),
                    }),
                ));
            }
        };

        // Calculate price change contribution
        // Formula: Quantity × (Price_T1 - Price_T0)
        let price_change = price_t1 - price_t0;
        let contribution = quantity * price_change;
        total_price_change += contribution;

        // Value at T1
        let value_t1 = weight * quantity * price_t1;

        constituent_prices.push(ConstituentPriceInfo {
            coin_id: coin.coin_id,
            symbol: coin.symbol.clone(),
            quantity: coin.quantity,
            weight: coin.weight,
            price: price_t1,
            value: value_t1,
        });

        tracing::debug!(
            "{}: Qty={}, Price T0={}, Price T1={}, Change={}, Contribution={}",
            coin.symbol,
            quantity,
            price_t0,
            price_t1,
            price_change,
            contribution
        );
    }

    // Final index price = Index Price at T0 + Total Price Change
    let index_price_t1 = index_price_t0 + total_price_change;

    tracing::debug!(
        "Index {} price on {}: Base={}, Change={}, Final={}",
        index_id,
        target_date,
        index_price_t0,
        total_price_change,
        index_price_t1
    );

    Ok((t0_timestamp, index_price_t1, constituent_prices))
}

/// GET /indexes/{index_id}/price-at-date?date=YYYY-MM-DD
pub async fn get_index_price_at_date(
    State(state): State<AppState>,
    Path(index_id): Path<i32>,
    Query(params): Query<IndexPriceAtDateRequest>,
) -> Result<Json<IndexPriceAtDateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Parse date (T1 - target date)
    let target_date = NaiveDate::parse_from_str(&params.date, "%Y-%m-%d").map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid date format. Use YYYY-MM-DD".to_string(),
            }),
        )
    })?;

    // Validate date is not in future
    let today = Utc::now().date_naive();
    if target_date > today {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Date cannot be in the future (today is {})", today),
            }),
        ));
    }

    // Calculate price using shared logic
    let (_timestamp, price, constituents) =
        calculate_index_price_internal(&state.db, &state.coingecko, index_id, target_date).await?;

    Ok(Json(IndexPriceAtDateResponse {
        index_id,
        date: target_date.to_string(),
        price,
        constituents,
    }))
}

/// GET /indexes/{index_id}/last-price
pub async fn get_index_last_price(
    State(state): State<AppState>,
    Path(index_id): Path<i32>,
) -> Result<Json<IndexLastPriceResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Use today's date
    let today = Utc::now().date_naive();

    // Calculate price using shared logic
    let (timestamp, last_price, constituents) =
        calculate_index_price_internal(&state.db, &state.coingecko, index_id, today).await?;

    Ok(Json(IndexLastPriceResponse {
        index_id,
        timestamp,
        last_price,
        last_bid: None,  // Not implemented yet
        last_ask: None,  // Not implemented yet
        constituents,
    }))
}

/// Get price for a coin on a specific date, fetching from CoinGecko if not in database
async fn get_or_fetch_price(
    db: &DatabaseConnection,
    coingecko: &CoinGeckoService,
    coin_id: &str,
    date: NaiveDate,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    use crate::entities::{coins_historical_prices, prelude::*};
    use sea_orm::ActiveModelTrait;
    use rust_decimal::Decimal;

    // Try to get from database first
    let existing = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::CoinId.eq(coin_id))
        .filter(coins_historical_prices::Column::Date.eq(date))
        .one(db)
        .await?;

    if let Some(record) = existing {
        // Convert Decimal to f64
        let price = record.price.to_string().parse::<f64>().unwrap_or(0.0);
        tracing::debug!("Found price for {} on {} in database: {}", coin_id, date, price);
        return Ok(price);
    }

    // Not in database, fetch from CoinGecko
    tracing::info!("Fetching price for {} on {} from CoinGecko (on-the-fly)", coin_id, date);

    // Calculate days from target date to now
    let today = Utc::now().date_naive();
    let days_ago = (today - date).num_days() as u32;

    if days_ago == 0 {
        // For today, we still need to fetch (use days=1 to get latest)
        let prices = coingecko
            .get_token_market_chart(coin_id, "usd", 1)
            .await?;

        if prices.is_empty() {
            return Err(format!("No price data returned from CoinGecko for {}", coin_id).into());
        }

        // Use the latest price
        let price = prices.last().unwrap().1;

        // Convert f64 to Decimal for storage
        let price_decimal = Decimal::from_f64_retain(price)
            .ok_or("Failed to convert price to Decimal")?;

        // Store in database
        let new_record = coins_historical_prices::ActiveModel {
            coin_id: Set(coin_id.to_string()),
            symbol: Set(String::new()),
            date: Set(date),
            price: Set(price_decimal),
            market_cap: Set(None),
            volume: Set(None),
            ..Default::default()
        };

        match new_record.insert(db).await {
            Ok(_) => tracing::info!("Stored price for {} on {}: {}", coin_id, date, price),
            Err(e) => tracing::warn!("Failed to store price for {}: {}", coin_id, e),
        }

        return Ok(price);
    }

    // Fetch from CoinGecko
    let prices = coingecko
        .get_token_market_chart(coin_id, "usd", days_ago + 1)
        .await?;

    if prices.is_empty() {
        return Err(format!("No price data returned from CoinGecko for {}", coin_id).into());
    }

    // Find the price closest to our target date
    let target_timestamp = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp() * 1000;

    let mut closest_price = None;
    let mut min_diff = i64::MAX;

    for (timestamp_ms, price) in &prices {
        let diff = (timestamp_ms - target_timestamp).abs();
        if diff < min_diff {
            min_diff = diff;
            closest_price = Some(*price);
        }
    }

    let price = closest_price.ok_or("No suitable price found in CoinGecko data")?;

    // Convert f64 to Decimal for storage
    let price_decimal = Decimal::from_f64_retain(price)
        .ok_or("Failed to convert price to Decimal")?;

    // Store in database for future use
    let new_record = coins_historical_prices::ActiveModel {
        coin_id: Set(coin_id.to_string()),
        symbol: Set(String::new()), // Will be updated by sync job
        date: Set(date),
        price: Set(price_decimal),
        market_cap: Set(None),
        volume: Set(None),
        ..Default::default()
    };

    match new_record.insert(db).await {
        Ok(_) => {
            tracing::info!("Stored price for {} on {}: {}", coin_id, date, price);
        }
        Err(e) => {
            tracing::warn!("Failed to store price for {} on {}: {}", coin_id, date, e);
            // Continue anyway, we have the price
        }
    }

    Ok(price)
}


pub async fn get_index_list(
    State(state): State<AppState>,
) -> Result<Json<IndexListResponse>, (StatusCode, Json<ErrorResponse>)> {
    const INDEX_DECIMALS: u32 = 30;
    const NETWORK: &str = "base";
    // Fetch all indexes from database
    let indexes = IndexMetadata::find().all(&state.db).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Database error: {}", e),
            }),
        )
    })?;

    let mut index_list = Vec::new();

    for index in indexes {
        // Fetch token details for each token_id
        // Get collateral from last rebalance
        let collateral = get_collateral_from_last_rebalance(&state.db, index.index_id).await?;

        // Calculate total minted quantity from blockchain events
        let index_address = index.address.to_lowercase();
        let total_minted_quantity = calculate_total_minted_quantity(
            &state,
            &index_address,
            NETWORK,
            INDEX_DECIMALS,
        )
        .await?;

        // Get latest price from daily_prices
        let latest_price_row = DailyPrices::find()
            .filter(daily_prices::Column::IndexId.eq(index.index_id.to_string()))
            .order_by(daily_prices::Column::Date, Order::Desc)
            .limit(1)
            .one(&state.db)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Database error while fetching latest price: {}", e),
                    }),
                )
            })?;

        let latest_price = latest_price_row
            .as_ref()
            .and_then(|row| row.price.to_string().parse::<f64>().ok());

        // USD value of supply = total minted qty * latest index price
        let total_supply_usd = if let Some(price) = latest_price {
            total_minted_quantity * price
        } else {
            0.0
        };

        // Calculate performance metrics
        let ytd_return = calculate_ytd_return(&state, index.index_id).await?;
        let one_year_return = calculate_period_return(&state, index.index_id, 365).await?;
        let three_year_return = calculate_period_return(&state, index.index_id, 365 * 3).await?;
        let five_year_return = calculate_period_return(&state, index.index_id, 365 * 5).await?;
        let ten_year_return = calculate_period_return(&state, index.index_id, 365 * 10).await?;

        // Floor to 2 decimal places
        let ytd_return = (ytd_return * 100.0).floor() / 100.0;

        // Get inception date
        let inception_date = get_inception_date_for_index(&state, index.index_id).await?;

        // Map database model to API response model
        index_list.push(IndexListEntry {
            index_id: index.index_id,
            name: index.name,
            address: index.address,
            ticker: index.symbol,
            curator: "0xF7F7d5C0d394f75307B4D981E8DE2Bab9639f90F".to_string(),
            total_supply: total_minted_quantity,
            total_supply_usd,
            ytd_return,
            collateral,
            management_fee: 2,
            asset_class: index.asset_class,
            inception_date,
            category: index.category,
            ratings: Some(Ratings {
                overall_rating: "A+".to_string(),
                expense_rating: "B".to_string(),
                risk_rating: "C+".to_string(),
            }),
            performance: Some(Performance {
                ytd_return,
                one_year_return,
                three_year_return,
                five_year_return,
                ten_year_return,
            }),
            index_price: latest_price,
        });
    }

    Ok(Json(IndexListResponse {
        indexes: index_list,
    }))
}

/// Get collateral tokens from the last rebalance
/// Returns empty vec if no rebalance exists
async fn get_collateral_from_last_rebalance(
    db: &DatabaseConnection,
    index_id: i32,
) -> Result<Vec<CollateralToken>, (StatusCode, Json<ErrorResponse>)> {
    // Get last rebalance
    let last_rebalance = Rebalances::find()
        .filter(rebalances::Column::IndexId.eq(index_id))
        .order_by(rebalances::Column::Timestamp, Order::Desc)
        .limit(1)
        .one(db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error while fetching rebalance: {}", e),
                }),
            )
        })?;

    // If no rebalance exists, return empty collateral
    let Some(rebalance) = last_rebalance else {
        tracing::debug!("No rebalance found for index {}, returning empty collateral", index_id);
        return Ok(Vec::new());
    };

    // Parse coins from rebalance JSON
    let coins: Vec<CoinRebalanceInfo> = serde_json::from_value(rebalance.coins)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to parse rebalance coins: {}", e),
                }),
            )
        })?;

    // Query coins table for logo_address
    let coin_ids: Vec<String> = coins.iter().map(|c| c.coin_id.clone()).collect();
    
    let coins_data = Coins::find()
        .filter(coins::Column::CoinId.is_in(coin_ids))
        .all(db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error while fetching coins: {}", e),
                }),
            )
        })?;

    // Build map: coin_id -> logo_address
    let logo_map: std::collections::HashMap<String, Option<String>> = coins_data
        .into_iter()
        .map(|c| (c.coin_id, c.logo_address))
        .collect();

    // Build collateral list
    let collateral: Vec<CollateralToken> = coins
        .into_iter()
        .map(|coin| {
            let logo = logo_map
                .get(&coin.coin_id)
                .and_then(|opt| opt.clone())
                .unwrap_or_default();

            CollateralToken {
                name: coin.symbol,
                logo,
            }
        })
        .collect();

    Ok(collateral)
}


async fn calculate_total_minted_quantity(
    state: &AppState,
    index_address: &str,
    network: &str,
    index_decimal: u32
) -> Result<f64, (StatusCode, Json<ErrorResponse>)> {
    // Query all mint events for this index
    let mint_events = BlockchainEvents::find()
        .filter(blockchain_events::Column::ContractAddress.eq(index_address))
        .filter(blockchain_events::Column::Network.eq(network))
        .filter(blockchain_events::Column::EventType.eq("mint"))
        .all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error while fetching blockchain events: {}", e),
                }),
            )
        })?;

    // Sum up quantities in base units (with INDEX_DECIMALS precision)
    let mut total_qty_base = Decimal::ZERO;

    for event in mint_events {
        if let Some(quantity) = event.quantity {
            // Convert quantity to base units (multiply by 10^INDEX_DECIMALS)
            let qty_decimal = to_bigint_units(quantity, index_decimal);
            total_qty_base += qty_decimal;
        }
    }

    // Convert back to decimal representation (divide by 10^INDEX_DECIMALS)
    let total_minted_quantity = from_bigint_units(total_qty_base, index_decimal);

    Ok(total_minted_quantity)
}

// Convert decimal to base units (like toBigIntUnits in TypeScript)
fn to_bigint_units(value: Decimal, decimals: u32) -> Decimal {
    let multiplier = Decimal::from(10_u64.pow(decimals));
    value * multiplier
}

// Convert base units to decimal (like ethers.formatUnits)
fn from_bigint_units(value: Decimal, decimals: u32) -> f64 {
    let divisor = Decimal::from(10_u64.pow(decimals));
    let result = value / divisor;
    
    // Convert Decimal to f64
    result.to_string().parse::<f64>().unwrap_or(0.0)
}

// Helper function to calculate YTD return
async fn calculate_ytd_return(
    state: &AppState,
    index_id: i32,
) -> Result<f64, (StatusCode, Json<ErrorResponse>)> {
    let now = Utc::now();
    
    // Previous day at midnight UTC
    let previous_day = (now - Duration::days(1))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    
    // January 1st of current year at midnight UTC
    let jan1 = NaiveDate::from_ymd_opt(now.year(), 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();

    let latest_price = get_price_for_date(state, index_id, previous_day.date()).await?;
    let jan1_price = get_price_for_date(state, index_id, jan1.date()).await?;

    if jan1_price.is_none() || jan1_price.unwrap() == 0.0 {
        return Ok(0.0);
    }

    let jan1_price_val = jan1_price.unwrap();
    let latest_price_val = latest_price.unwrap_or(0.0);

    Ok(((latest_price_val - jan1_price_val) / jan1_price_val) * 100.0)
}

// Helper function to calculate period return
async fn calculate_period_return(
    state: &AppState,
    index_id: i32,
    days: i64,
) -> Result<f64, (StatusCode, Json<ErrorResponse>)> {
    let now = Utc::now();
    
    // End date: 2 days ago at midnight UTC
    let end_date = (now - Duration::days(2))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .date();
    
    // Start date: days ago at midnight UTC
    let start_date = (now - Duration::days(days))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .date();

    let end_price = get_price_for_date(state, index_id, end_date).await?;
    let start_price = get_price_for_date(state, index_id, start_date).await?;

    if start_price.is_none() || start_price.unwrap() == 0.0 {
        return Ok(0.0);
    }

    let start_price_val = start_price.unwrap();
    let end_price_val = end_price.unwrap_or(0.0);

    Ok(((end_price_val - start_price_val) / start_price_val) * 100.0)
}

// Helper function to get price for a specific date
async fn get_price_for_date(
    state: &AppState,
    index_id: i32,
    target_date: NaiveDate,
) -> Result<Option<f64>, (StatusCode, Json<ErrorResponse>)> {
    let price_row = DailyPrices::find()
        .filter(daily_prices::Column::IndexId.eq(index_id.to_string()))
        .filter(daily_prices::Column::Date.eq(target_date))
        .order_by(daily_prices::Column::Date, Order::Desc)
        .limit(1)
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error while fetching price: {}", e),
                }),
            )
        })?;

    Ok(price_row.and_then(|row| row.price.to_string().parse::<f64>().ok()))
}

// Helper function to get inception date for an index
async fn get_inception_date_for_index(
    state: &AppState,
    index_id: i32,
) -> Result<Option<String>, (StatusCode, Json<ErrorResponse>)> {
    let earliest_price = DailyPrices::find()
        .filter(daily_prices::Column::IndexId.eq(index_id.to_string()))
        .order_by(daily_prices::Column::Date, Order::Asc)
        .limit(1)
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error while fetching inception date: {}", e),
                }),
            )
        })?;

    Ok(earliest_price.map(|row| row.date.to_string()))
}

// Add new create_index handler
pub async fn create_index(
    State(state): State<AppState>,
    Json(payload): Json<CreateIndexRequest>,
) -> Result<(StatusCode, Json<CreateIndexResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Check if index already exists
    let existing = IndexMetadata::find()
        .filter(index_metadata::Column::IndexId.eq(payload.index_id))
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

    if existing.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "Index already exists".to_string(),
            }),
        ));
    }

    // Validate weight_strategy
    if !["equal", "marketCap"].contains(&payload.weight_strategy.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid weight_strategy. Must be 'equal' or 'marketCap'".to_string(),
            }),
        ));
    }

    // Validate weight_threshold
    if let Some(threshold) = payload.weight_threshold {
        // Must be between 0.1 and 100.0
        if threshold < dec!(0.1) || threshold > dec!(100.0) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "weight_threshold must be between 0.1 and 100.0".to_string(),
                }),
            ));
        }

        // Threshold only valid for marketCap strategy
        if payload.weight_strategy == "equal" {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "weight_threshold is only applicable with 'marketCap' strategy".to_string(),
                }),
            ));
        }
    }

    // Validate blacklisted categories
    if let Some(ref blacklist) = payload.blacklisted_categories {
        if blacklist.is_empty() {
            // Empty array is allowed (same as NULL)
            tracing::debug!("Empty blacklist array provided, treating as no blacklist");
        } else {
            // Validate each category exists in coingecko_categories table
            for category_id in blacklist {
                let category_exists = CoingeckoCategories::find()
                    .filter(coingecko_categories::Column::CategoryId.eq(category_id))
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

                if category_exists.is_none() {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!(
                                "Invalid blacklisted category: '{}'. Use /coingecko-categories to see valid categories",
                                category_id
                            ),
                        }),
                    ));
                }
            }
            
            tracing::info!(
                "Validated {} blacklisted categories for index {}",
                blacklist.len(),
                payload.index_id
            );
        }
    }

    // Look up token IDs from symbols
    // Serialize exchanges_allowed to JSON
    let exchanges_json = serde_json::to_value(&payload.exchanges_allowed).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to serialize exchanges: {}", e),
            }),
        )
    })?;

    // Serialize blacklisted_categories to JSON (NEW)
    let blacklisted_categories_json = if let Some(ref blacklist) = payload.blacklisted_categories {
        if blacklist.is_empty() {
            None  // Treat empty array as NULL
        } else {
            Some(serde_json::to_value(blacklist).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to serialize blacklisted categories: {}", e),
                    }),
                )
            })?)
        }
    } else {
        None
    };

    // Insert new index
    let new_index = index_metadata::ActiveModel {
        index_id: Set(payload.index_id),
        name: Set(payload.name.clone()),
        symbol: Set(payload.symbol.clone()),
        address: Set(payload.address.clone()),
        category: Set(payload.category.clone()),
        asset_class: Set(payload.asset_class.clone()),
        initial_date: Set(Some(payload.initial_date)),
        initial_price: Set(Some(payload.initial_price)),
        coingecko_category: Set(Some(payload.coingecko_category.clone())),
        exchanges_allowed: Set(Some(exchanges_json)),
        exchange_trading_fees: Set(Some(payload.exchange_trading_fees)),
        exchange_avg_spread: Set(Some(payload.exchange_avg_spread)),
        rebalance_period: Set(Some(payload.rebalance_period)),
        weight_strategy: Set(payload.weight_strategy.clone()),
        weight_threshold: Set(payload.weight_threshold),
        blacklisted_categories: Set(blacklisted_categories_json),
        ..Default::default()
    };

    let result = new_index.insert(&state.db).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to insert index: {}", e),
            }),
        )
    })?;

    let index_id = result.index_id;

    // Spawn background task for backfilling
    let db_clone = state.db.clone();
    let coingecko_clone = state.coingecko.clone();
    tokio::spawn(async move {
        tracing::info!("Starting backfill for index {}", index_id);
        let rebalancing_service = RebalancingService::new(db_clone, coingecko_clone, None);
        
        match rebalancing_service.backfill_historical_rebalances(index_id).await {
            Ok(_) => tracing::info!("Successfully completed backfill for index {}", index_id),
            Err(e) => tracing::error!("Failed to backfill index {}: {}", index_id, e),
        }
    });

    // Parse blacklisted_categories from result for response
    let blacklisted_categories_response = result.blacklisted_categories
        .as_ref()
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok());

    // Return response immediately
    Ok((
        StatusCode::CREATED,
        Json(CreateIndexResponse {
            index_id: result.index_id,
            name: result.name,
            symbol: result.symbol,
            address: result.address,
            category: result.category,
            asset_class: result.asset_class,
            initial_date: result.initial_date.unwrap(),
            initial_price: result.initial_price.unwrap().to_string(),
            coingecko_category: result.coingecko_category.unwrap(),
            exchanges_allowed: payload.exchanges_allowed,
            exchange_trading_fees: result.exchange_trading_fees.unwrap().to_string(),
            exchange_avg_spread: result.exchange_avg_spread.unwrap().to_string(),
            rebalance_period: result.rebalance_period.unwrap(),
            weight_strategy: result.weight_strategy,
            weight_threshold: result.weight_threshold.map(|d| d.to_string()),
            blacklisted_categories: blacklisted_categories_response,
        }),
    ))
}

pub async fn get_index_config(
    State(state): State<AppState>,
    Path(index_id): Path<i32>,
) -> Result<Json<IndexConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Get index metadata from database
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

    // Validate required fields
    let initial_date = index.initial_date.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Index {} has no initial_date configured", index_id),
            }),
        )
    })?;

    let initial_price = index.initial_price.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Index {} has no initial_price configured", index_id),
            }),
        )
    })?;

    let exchanges_allowed_json = index.exchanges_allowed.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Index {} has no exchanges_allowed configured", index_id),
            }),
        )
    })?;

    let exchange_trading_fees = index.exchange_trading_fees.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Index {} has no exchange_trading_fees configured", index_id),
            }),
        )
    })?;

    let exchange_avg_spread = index.exchange_avg_spread.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Index {} has no exchange_avg_spread configured", index_id),
            }),
        )
    })?;

    let rebalance_period = index.rebalance_period.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Index {} has no rebalance_period configured", index_id),
            }),
        )
    })?;

    // Parse exchanges_allowed from JSON
    let exchanges_allowed: Vec<String> = serde_json::from_value(exchanges_allowed_json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to parse exchanges_allowed: {}", e),
                }),
            )
        })?;

    // Build response
    let response = IndexConfigResponse {
        index_id: index.index_id,
        symbol: index.symbol,
        name: index.name,
        address: index.address,
        initial_date,
        initial_price: initial_price.to_string(),
        exchanges_allowed,
        exchange_trading_fees: exchange_trading_fees.to_string(),
        exchange_avg_spread: exchange_avg_spread.to_string(),
        rebalance_period,
    };

    Ok(Json(response))
}

pub async fn remove_index(
    State(state): State<AppState>,
    Json(payload): Json<RemoveIndexRequest>,
) -> Result<Json<RemoveIndexResponse>, (StatusCode, Json<ErrorResponse>)> {
    let index_id = payload.index_id;

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
        })?;

    let index = match index {
        Some(idx) => idx,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Index {} not found", index_id),
                }),
            ));
        }
    };

    // Check if index is deployed
    if index.deployment_data.is_some() {
        return Ok(Json(RemoveIndexResponse {
            success: false,
            message: format!(
                "Cannot remove index {} ({}): Index is deployed. Undeploy it first before removal.",
                index_id,
                index.name
            ),
            index_id,
        }));
    }

    // Index is not deployed - safe to remove
    tracing::info!("Removing undeployed index {} ({})", index_id, index.name);

    // Count rebalances before deletion (for logging)
    let rebalances_count = Rebalances::find()
        .filter(rebalances::Column::IndexId.eq(index_id))
        .count(&state.db)
        .await
        .unwrap_or(0);

    // Delete the index (rebalances will cascade automatically via FK)
    IndexMetadata::delete_by_id(index_id)
        .exec(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to delete index: {}", e),
                }),
            )
        })?;

    tracing::info!(
        "Successfully removed index {} ({}) and {} associated rebalances (cascade)",
        index_id,
        index.name,
        rebalances_count
    );

    Ok(Json(RemoveIndexResponse {
        success: true,
        message: format!(
            "Successfully removed index {} ({}) and {} associated rebalances",
            index_id,
            index.name,
            rebalances_count
        ),
        index_id,
    }))
}

pub async fn get_current_index_weight(
    State(state): State<AppState>,
    Path(index_id): Path<i32>,
) -> Result<Json<CurrentIndexWeightResponse>, (StatusCode, Json<ErrorResponse>)> {
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
        })?;

    let index = match index {
        Some(idx) => idx,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Index {} not found", index_id),
                }),
            ));
        }
    };

    // Get last rebalance
    let last_rebalance = Rebalances::find()
        .filter(rebalances::Column::IndexId.eq(index_id))
        .order_by(rebalances::Column::Timestamp, Order::Desc)
        .limit(1)
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

    let last_rebalance = match last_rebalance {
        Some(rb) => rb,
        None => {
            // No rebalance found - return appropriate message
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!(
                        "No rebalances found for index {} ({}). Index may not be initialized yet.",
                        index_id, index.name
                    ),
                }),
            ));
        }
    };

    // Parse constituents from last rebalance
    let coins: Vec<crate::services::rebalancing::CoinRebalanceInfo> =
        serde_json::from_value(last_rebalance.coins.clone())
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to parse rebalance data: {}", e),
                    }),
                )
            })?;

    // Calculate total weight for percentage calculations
    let total_weight: f64 = last_rebalance.total_weight
        .to_string()
        .parse()
        .unwrap_or(0.0);

    // Build constituent weights with percentage
    let mut constituents = Vec::new();

    for coin in coins {
        let weight: f64 = coin.weight.parse().unwrap_or(0.0);
        let quantity: f64 = coin.quantity.parse().unwrap_or(0.0);
        let price = coin.price;
        let value = weight * quantity * price;

        // Calculate weight percentage (weight / total_weight * 100)
        let weight_percentage = if total_weight > 0.0 {
            (weight / total_weight) * 100.0
        } else {
            0.0
        };

        constituents.push(ConstituentWeight {
            coin_id: coin.coin_id,
            symbol: coin.symbol,
            weight: coin.weight,
            weight_percentage,
            quantity: coin.quantity,
            price,
            value,
            exchange: coin.exchange,
            trading_pair: coin.trading_pair,
        });
    }

    // Sort by weight percentage descending (largest holdings first)
    constituents.sort_by(|a, b| {
        b.weight_percentage
            .partial_cmp(&a.weight_percentage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Format rebalance date
    let rebalance_date = chrono::DateTime::from_timestamp(last_rebalance.timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    Ok(Json(CurrentIndexWeightResponse {
        index_id,
        index_name: index.name,
        index_symbol: index.symbol,
        last_rebalance_date: rebalance_date,
        portfolio_value: last_rebalance.portfolio_value.to_string(),
        total_weight: last_rebalance.total_weight.to_string(),
        constituents,
    }))
}
