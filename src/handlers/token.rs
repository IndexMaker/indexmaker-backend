use axum::{extract::State, http::StatusCode, Json};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::entities::{prelude::*, token_metadata};
use crate::models::token::{
    AddTokenRequest, AddTokenResponse, AddTokensRequest, AddTokensResponse,
    AddTokensResponseItem, ErrorResponse,
};
use crate::AppState;

pub async fn add_token(
    State(state): State<AppState>,
    Json(payload): Json<AddTokenRequest>,
) -> Result<(StatusCode, Json<AddTokenResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Check if token already exists
    let existing = TokenMetadata::find()
        .filter(token_metadata::Column::Symbol.eq(&payload.symbol))
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
        return Err((StatusCode::CONFLICT, Json(ErrorResponse { error: "".to_string() })));
    }

    // Insert new token
    let new_token = token_metadata::ActiveModel {
        symbol: Set(payload.symbol.clone()),
        logo_address: Set(payload.logo_address.clone()),
        ..Default::default()
    };

    let result = new_token.insert(&state.db).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to insert token: {}", e),
            }),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(AddTokenResponse {
            id: result.id,
            symbol: result.symbol,
            logo_address: result.logo_address,
        }),
    ))
}

pub async fn add_tokens(
    State(state): State<AppState>,
    Json(payload): Json<AddTokensRequest>,
) -> (StatusCode, Json<AddTokensResponse>) {
    let mut results = Vec::new();

    for token_req in payload.tokens {
        // Check if token already exists
        match TokenMetadata::find()
            .filter(token_metadata::Column::Symbol.eq(&token_req.symbol))
            .one(&state.db)
            .await
        {
            Ok(Some(_existing)) => {
                // Token already exists
                results.push(AddTokensResponseItem {
                    data: None,
                    message: "is_duplicate".to_string(),
                });
            }
            Ok(None) => {
                // Insert new token
                let new_token = token_metadata::ActiveModel {
                    symbol: Set(token_req.symbol.clone()),
                    logo_address: Set(token_req.logo_address.clone()),
                    ..Default::default()
                };

                match new_token.insert(&state.db).await {
                    Ok(result) => {
                        results.push(AddTokensResponseItem {
                            data: Some(AddTokenResponse {
                                id: result.id,
                                symbol: result.symbol,
                                logo_address: result.logo_address,
                            }),
                            message: "".to_string(),
                        });
                    }
                    Err(e) => {
                        results.push(AddTokensResponseItem {
                            data: None,
                            message: format!("Failed to insert: {}", e),
                        });
                    }
                }
            }
            Err(e) => {
                results.push(AddTokensResponseItem {
                    data: None,
                    message: format!("Database error: {}", e),
                });
            }
        }
    }

    (StatusCode::OK, Json(AddTokensResponse { results }))
}
