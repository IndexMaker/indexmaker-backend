use axum::{extract::State, http::StatusCode, Json};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};

use crate::entities::{prelude::*, subscriptions};
use crate::models::subscription::{SubscribeRequest, SubscribeResponse};
use crate::models::token::ErrorResponse;
use crate::AppState;

pub async fn subscribe(
    State(state): State<AppState>,
    Json(payload): Json<SubscribeRequest>,
) -> Result<Json<SubscribeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let twitter = payload.twitter.unwrap_or_default();

    // Check if email already exists
    let existing = Subscriptions::find()
        .filter(subscriptions::Column::Email.eq(&payload.email))
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

    if let Some(existing_sub) = existing {
        // Update existing subscription
        let mut active_model = existing_sub.into_active_model();
        active_model.twitter = Set(Some(twitter));

        active_model.update(&state.db).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update subscription: {}", e),
                }),
            )
        })?;
    } else {
        // Insert new subscription
        let new_subscription = subscriptions::ActiveModel {
            email: Set(payload.email.clone()),
            twitter: Set(Some(twitter)),
            ..Default::default()
        };

        new_subscription.insert(&state.db).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to insert subscription: {}", e),
                }),
            )
        })?;
    }

    Ok(Json(SubscribeResponse { success: true }))
}
