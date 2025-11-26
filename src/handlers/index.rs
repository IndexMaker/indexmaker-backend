use chrono::{Datelike, Duration, NaiveDate, Utc};

use axum::{extract::State, http::StatusCode, Json};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, Set};

use crate::entities::{blockchain_events, coingecko_categories, daily_prices, index_metadata, prelude::*, token_metadata};
use crate::models::index::{
    CollateralToken, CreateIndexRequest, CreateIndexResponse, IndexListEntry, IndexListResponse, Performance, Ratings
};
use crate::models::token::ErrorResponse;
use crate::AppState;
use crate::services::rebalancing::RebalancingService;


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
        let mut collateral = Vec::new();

        for token_id in &index.token_ids {
            let token = TokenMetadata::find_by_id(*token_id)
                .one(&state.db)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Database error while fetching token: {}", e),
                        }),
                    )
                })?;

            if let Some(token) = token {
                collateral.push(CollateralToken {
                    name: token.symbol,
                    logo: token.logo_address.unwrap_or_default(),
                });
            }
        }

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
    // Validate CoinGecko category
    let category_exists = CoingeckoCategories::find()
        .filter(coingecko_categories::Column::CategoryId.eq(&payload.coingecko_category))
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
                    "Invalid coingecko_category: '{}'. Please use a valid category from /coingecko-categories",
                    payload.coingecko_category
                ),
            }),
        ));
    }

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

    // Look up token IDs from symbols
    let mut token_ids = Vec::new();
    for symbol in &payload.tokens {
        let token = TokenMetadata::find()
            .filter(token_metadata::Column::Symbol.eq(symbol))
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

        match token {
            Some(t) => token_ids.push(t.id),
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Token symbol '{}' not found in token_metadata", symbol),
                    }),
                ));
            }
        }
    }

    // Serialize exchanges_allowed to JSON
    let exchanges_json = serde_json::to_value(&payload.exchanges_allowed).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to serialize exchanges: {}", e),
            }),
        )
    })?;

    // Insert new index
    let new_index = index_metadata::ActiveModel {
        index_id: Set(payload.index_id),
        name: Set(payload.name.clone()),
        symbol: Set(payload.symbol.clone()),
        address: Set(payload.address.clone()),
        category: Set(payload.category.clone()),
        asset_class: Set(payload.asset_class.clone()),
        token_ids: Set(token_ids.clone()),
        initial_date: Set(Some(payload.initial_date)),
        initial_price: Set(Some(payload.initial_price)),
        coingecko_category: Set(Some(payload.coingecko_category.clone())),
        exchanges_allowed: Set(Some(exchanges_json)),
        exchange_trading_fees: Set(Some(payload.exchange_trading_fees)),
        exchange_avg_spread: Set(Some(payload.exchange_avg_spread)),
        rebalance_period: Set(Some(payload.rebalance_period)),
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
        let rebalancing_service = RebalancingService::new(db_clone, coingecko_clone);
        
        match rebalancing_service.backfill_historical_rebalances(index_id).await {
            Ok(_) => tracing::info!("Successfully completed backfill for index {}", index_id),
            Err(e) => tracing::error!("Failed to backfill index {}: {}", index_id, e),
        }
    });

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
            token_ids,
            initial_date: result.initial_date.unwrap(),
            initial_price: result.initial_price.unwrap().to_string(),
            coingecko_category: result.coingecko_category.unwrap(),
            exchanges_allowed: payload.exchanges_allowed,
            exchange_trading_fees: result.exchange_trading_fees.unwrap().to_string(),
            exchange_avg_spread: result.exchange_avg_spread.unwrap().to_string(),
            rebalance_period: result.rebalance_period.unwrap(),
        }),
    ))
}