use axum::{extract::State, http::StatusCode, Json};
use sea_orm::{EntityTrait, Order, QueryOrder};

use crate::entities::{coingecko_categories, prelude::*};
use crate::models::category::{CategoriesListResponse, CategoryResponse};
use crate::models::token::ErrorResponse;
use crate::AppState;

pub async fn get_coingecko_categories(
    State(state): State<AppState>,
) -> Result<Json<CategoriesListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let categories = CoingeckoCategories::find()
        .order_by(coingecko_categories::Column::Name, Order::Asc)
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

    let response: Vec<CategoryResponse> = categories
        .into_iter()
        .map(|cat| CategoryResponse {
            category_id: cat.category_id,
            name: cat.name,
        })
        .collect();

    Ok(Json(response))
}
