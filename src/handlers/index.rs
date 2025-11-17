use axum::{extract::State, Json};
use crate::models::index::IndexListResponse;
use crate::AppState;

pub async fn get_index_list(State(_state): State<AppState>) -> Json<IndexListResponse> {
    // TODO: Implement actual query from database
    Json(IndexListResponse::default())
}