use axum::{extract::State, http::StatusCode, Json};
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder};
use std::collections::{HashMap, HashSet};

use crate::entities::{category_membership, coingecko_categories, prelude::*};
use crate::models::category::{CategoriesListResponse, CategoriesWithCountResponse, CategoryResponse, CategoryWithCountResponse};
use crate::models::token::ErrorResponse;
use crate::AppState;

/// Normalize an exchange symbol to match CoinGecko conventions
/// Examples: "1000SHIB" -> "shib", "1000FLOKI" -> "floki", "BTC" -> "btc"
fn normalize_symbol(symbol: &str) -> String {
    let s = symbol.to_lowercase();
    s.strip_prefix("1000")
        .or_else(|| s.strip_prefix("10000"))
        .or_else(|| s.strip_prefix("100000"))
        .unwrap_or(&s)
        .to_string()
}

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

/// Get categories with tradeable token counts
/// This endpoint returns categories filtered to only those with at least one tradeable token
pub async fn get_categories_with_counts(
    State(state): State<AppState>,
) -> Result<Json<CategoriesWithCountResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Step 1: Get all tradeable symbols from exchange API
    let tradeable_tokens = state.exchange_api.get_all_tradeable_symbols().await
        .map_err(|e| {
            tracing::error!("Failed to fetch tradeable symbols: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to fetch tradeable symbols: {}", e),
                }),
            )
        })?;

    // Build set of tradeable symbols (both original and normalized)
    let mut tradeable_symbols: HashSet<String> = HashSet::new();
    for token in &tradeable_tokens {
        let upper = token.symbol.to_uppercase();
        let lower = token.symbol.to_lowercase();
        let normalized = normalize_symbol(&token.symbol);
        tradeable_symbols.insert(upper);
        tradeable_symbols.insert(lower);
        tradeable_symbols.insert(normalized);
    }

    tracing::info!("Built tradeable symbols set with {} entries from {} tokens",
        tradeable_symbols.len(), tradeable_tokens.len());

    // Step 2: Get all categories
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

    // Step 3: Get all category memberships with symbols
    let all_memberships = CategoryMembership::find()
        .filter(category_membership::Column::Symbol.is_not_null())
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

    // Build category -> tradeable count map
    let mut category_counts: HashMap<String, u32> = HashMap::new();
    for membership in all_memberships {
        if let Some(symbol) = membership.symbol {
            let symbol_lower = symbol.to_lowercase();
            let symbol_upper = symbol.to_uppercase();
            let symbol_normalized = normalize_symbol(&symbol);

            // Check if this symbol is tradeable
            if tradeable_symbols.contains(&symbol_lower)
                || tradeable_symbols.contains(&symbol_upper)
                || tradeable_symbols.contains(&symbol_normalized)
            {
                *category_counts.entry(membership.category_id).or_insert(0) += 1;
            }
        }
    }

    // Step 4: Build response with counts, filtering out categories with 0 tradeable tokens
    let response: Vec<CategoryWithCountResponse> = categories
        .into_iter()
        .filter_map(|cat| {
            let count = category_counts.get(&cat.category_id).copied().unwrap_or(0);
            if count > 0 {
                Some(CategoryWithCountResponse {
                    category_id: cat.category_id,
                    name: cat.name,
                    tradeable_count: count,
                })
            } else {
                None
            }
        })
        .collect();

    tracing::info!("Returning {} categories with tradeable tokens (filtered from total)", response.len());

    Ok(Json(response))
}
