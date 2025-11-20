use axum::{extract::State, http::StatusCode, Json};
use rust_decimal::Decimal;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::entities::{blockchain_events, prelude::*};
use crate::models::index_maker::IndexMakerInfoResponse;
use crate::models::token::ErrorResponse;
use crate::AppState;

const DECIMALS: u32 = 18;

pub async fn get_index_maker_info(
    State(state): State<AppState>,
) -> Result<Json<IndexMakerInfoResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Query all mint events
    let rows = BlockchainEvents::find()
        .filter(blockchain_events::Column::EventType.eq("mint"))
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

    // Sum all amounts
    let mut total_volume_raw = Decimal::ZERO;

    for row in rows {
        if let Some(amount) = row.amount {
            // Convert decimal to base units (multiply by 10^DECIMALS)
            let amount_in_base_units = decimal_to_bigint(amount, DECIMALS);
            total_volume_raw += amount_in_base_units;
        }
    }

    // Format results (divide by 10^DECIMALS to get human-readable values)
    let total_volume = format_units(Decimal::ZERO, 6); // Always "0" as per your code
    let total_managed = format_units(total_volume_raw, DECIMALS);

    Ok(Json(IndexMakerInfoResponse {
        total_volume,
        total_managed,
    }))
}

// Convert decimal to base units (like toBigIntUnits)
fn decimal_to_bigint(value: Decimal, decimals: u32) -> Decimal {
    let multiplier = Decimal::from(10_u64.pow(decimals));
    value * multiplier
}

// Format units back to human-readable string (like ethers.formatUnits)
fn format_units(value: Decimal, decimals: u32) -> String {
    let divisor = Decimal::from(10_u64.pow(decimals));
    let result = value / divisor;
    
    // Format as string, removing trailing zeros
    let formatted = result.to_string();
    
    // Remove trailing zeros after decimal point
    if formatted.contains('.') {
        formatted.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        formatted
    }
}
