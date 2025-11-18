use axum::{extract::State, http::StatusCode, Json};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::entities::{prelude::*, index_metadata, token_metadata};
use crate::models::index::{
    AddIndexRequest, AddIndexResponse,
    CollateralToken, IndexListEntry, IndexListResponse, Performance, Ratings,
};
use crate::models::token::ErrorResponse;
use crate::AppState;

pub async fn get_index_list(State(state): State<AppState>) -> Result<Json<IndexListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Fetch all indexes from database
    let indexes = IndexMetadata::find()
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

        // Map database model to API response model
        index_list.push(IndexListEntry {
            index_id: index.index_id,
            name: index.name,
            address: index.address,
            ticker: index.symbol,
            curator: "0xF7F7d5C0d394f75307B4D981E8DE2Bab9639f90F".to_string(),
            total_supply: 0.00002010588139611647,
            total_supply_usd: 6.195548738217032,
            ytd_return: -11.49,
            collateral,
            management_fee: 2,
            asset_class: index.asset_class,
            inception_date: Some("2019-01-01".to_string()),
            category: index.category,
            ratings: Some(Ratings {
                overall_rating: "A+".to_string(),
                expense_rating: "B".to_string(),
                risk_rating: "C+".to_string(),
            }),
            performance: Some(Performance {
                ytd_return: -11.49,
                one_year_return: 76.38137132434154,
                three_year_return: 237.1885256621526,
                five_year_return: 1738.3370284019127,
                ten_year_return: 0.0,
            }),
            index_price: Some(308146.09),
        });
    }

    Ok(Json(IndexListResponse {
        indexes: index_list,
    }))
}

pub async fn add_index(
    State(state): State<AppState>,
    Json(payload): Json<AddIndexRequest>,
) -> Result<(StatusCode, Json<AddIndexResponse>), (StatusCode, Json<ErrorResponse>)> {
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

    // Insert new index
    let new_index = index_metadata::ActiveModel {
        index_id: Set(payload.index_id),
        name: Set(payload.name.clone()),
        symbol: Set(payload.symbol.clone()),
        address: Set(payload.address.clone()),
        category: Set(payload.category.clone()),
        asset_class: Set(payload.asset_class.clone()),
        token_ids: Set(token_ids.clone()),
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

    Ok((
        StatusCode::CREATED,
        Json(AddIndexResponse {
            index_id: result.index_id,
            name: result.name,
            symbol: result.symbol,
            address: result.address,
            category: result.category,
            asset_class: result.asset_class,
            token_ids,
        }),
    ))
}
