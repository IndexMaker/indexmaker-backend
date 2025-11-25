use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryResponse {
    pub category_id: String,
    pub name: String,
}

pub type CategoriesListResponse = Vec<CategoryResponse>;
