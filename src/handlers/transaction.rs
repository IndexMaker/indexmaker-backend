use axum::{extract::State, http::StatusCode, Json, extract::Path};
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rust_decimal::Decimal;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::entities::{blockchain_events, prelude::*};
use crate::models::token::ErrorResponse;
use crate::models::transaction::{TransactionAmount, UserTransaction, UserTransactionResponse};
use crate::AppState;

const NETWORK: &str = "base";

// Similar to @Get('/getUserTransactionData/:indexId') in old backend
pub async fn get_index_transactions(
    State(state): State<AppState>,
    Path(index_id): Path<i32>,
) -> Result<Json<UserTransactionResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Get index metadata from database
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

    let contract_address = index_data.address.to_lowercase();

    // Query blockchain events
    let events = BlockchainEvents::find()
        .filter(blockchain_events::Column::ContractAddress.eq(&contract_address))
        .filter(blockchain_events::Column::Network.eq(NETWORK))
        .filter(
            blockchain_events::Column::EventType
                .is_in(vec!["mint", "deposit", "withdraw"]),
        )
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

    // Parse to activity list
    let mut activities: Vec<UserTransaction> = events
        .into_iter()
        .enumerate()
        .map(|(i, event)| {
            let wallet = event.user_address.clone();
            let amount = event
                .amount
                .unwrap_or(Decimal::ZERO)
                .to_string()
                .parse::<f64>()
                .unwrap_or(0.0);
            
            let quantity = event
                .quantity
                .unwrap_or(Decimal::ZERO)
                .to_string();

            // Format datetime
            let date_time = if let Some(timestamp) = event.timestamp {
                format_datetime(timestamp)
            } else {
                "Unknown".to_string()
            };

            // Capitalize event type
            let transaction_type = capitalize(&event.event_type);

            UserTransaction {
                id: format!("{}-{}-{}", event.event_type, event.tx_hash, i),
                date_time,
                wallet,
                hash: event.tx_hash,
                transaction_type,
                amount: TransactionAmount {
                    amount,
                    currency: "USDC".to_string(),
                    amount_summary: format!("{} {}", quantity, index_data.symbol),
                },
            }
        })
        .collect();

    // Sort by date descending
    activities.sort_by(|a, b| {
        let date_a = parse_datetime(&a.date_time);
        let date_b = parse_datetime(&b.date_time);
        date_b.cmp(&date_a)
    });

    Ok(Json(activities))
}

// Helper: Format datetime as "YYYY-MM-DD HH:MM:SS"
fn format_datetime(timestamp: DateTime<FixedOffset>) -> String {
    timestamp.format("%Y-%m-%d %H:%M:%S").to_string()
}

// Helper: Parse datetime string back for sorting
fn parse_datetime(datetime_str: &str) -> i64 {
    NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or(0)
}

// Helper: Capitalize first letter
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
