use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryResponse {
    pub category_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryWithCountResponse {
    pub category_id: String,
    pub name: String,
    pub tradeable_count: u32,
}

pub type CategoriesListResponse = Vec<CategoryResponse>;
pub type CategoriesWithCountResponse = Vec<CategoryWithCountResponse>;
