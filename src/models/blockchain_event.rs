use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBlockchainEventRequest {
    pub tx_hash: String,
    pub block_number: i32,
    pub log_index: i32,
    pub event_type: String,
    pub contract_address: String,
    pub network: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockchainEventResponse {
    pub id: i32,
    pub tx_hash: String,
    pub block_number: i32,
    pub log_index: i32,
    pub event_type: String,
    pub contract_address: String,
    pub network: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<NaiveDateTime>,
}