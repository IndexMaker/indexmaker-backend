use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTokenRequest {
    pub symbol: String,
    pub logo_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTokenResponse {
    pub id: i32,
    pub symbol: String,
    pub logo_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTokensRequest {
    pub tokens: Vec<AddTokenRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTokensResponseItem {
    pub data: Option<AddTokenResponse>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTokensResponse {
    pub results: Vec<AddTokensResponseItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}