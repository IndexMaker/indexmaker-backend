use axum::{extract::State, http::StatusCode, Json};
use chrono::{FixedOffset, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};

use crate::entities::{blockchain_events, prelude::*};
use crate::models::blockchain_event::{BlockchainEventResponse, CreateBlockchainEventRequest};
use crate::models::token::ErrorResponse;
use crate::AppState;

pub async fn save_blockchain_event(
    State(state): State<AppState>,
    Json(payload): Json<CreateBlockchainEventRequest>,
) -> Result<(StatusCode, Json<BlockchainEventResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Check if event with this tx_hash already exists
    let existing = BlockchainEvents::find()
        .filter(blockchain_events::Column::TxHash.eq(&payload.tx_hash))
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

    let result = if let Some(existing_event) = existing {
        // Update existing event
        let mut active_model = existing_event.into_active_model();
        
        active_model.block_number = Set(payload.block_number);
        active_model.log_index = Set(payload.log_index);
        active_model.event_type = Set(payload.event_type.clone());
        active_model.contract_address = Set(payload.contract_address.clone());
        active_model.network = Set(payload.network.clone());
        active_model.user_address = Set(payload.user_address.clone());
        active_model.amount = Set(payload.amount);
        active_model.quantity = Set(payload.quantity);
        active_model.timestamp = Set(Some(Utc::now().with_timezone(&FixedOffset::east_opt(0).unwrap())));

        active_model.update(&state.db).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update blockchain event: {}", e),
                }),
            )
        })?
    } else {
        // Insert new event
        let new_event = blockchain_events::ActiveModel {
            tx_hash: Set(payload.tx_hash.clone()),
            block_number: Set(payload.block_number),
            log_index: Set(payload.log_index),
            event_type: Set(payload.event_type.clone()),
            contract_address: Set(payload.contract_address.clone()),
            network: Set(payload.network.clone()),
            user_address: Set(payload.user_address.clone()),
            amount: Set(payload.amount),
            quantity: Set(payload.quantity),
            timestamp: Set(Some(Utc::now().with_timezone(&FixedOffset::east_opt(0).unwrap()))),
            ..Default::default()
        };

        new_event.insert(&state.db).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to insert blockchain event: {}", e),
                }),
            )
        })?
    };

    Ok((
        StatusCode::CREATED,
        Json(BlockchainEventResponse {
            id: result.id,
            tx_hash: result.tx_hash,
            block_number: result.block_number,
            log_index: result.log_index,
            event_type: result.event_type,
            contract_address: result.contract_address,
            network: result.network,
            user_address: result.user_address,
            amount: result.amount,
            quantity: result.quantity,
            timestamp: result.timestamp.map(|dt| dt.naive_utc()),
        }),
    ))
}