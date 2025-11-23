use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTransaction {
    pub id: String,
    pub date_time: String,
    pub wallet: Option<String>,
    pub hash: String,
    pub transaction_type: String,
    pub amount: TransactionAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionAmount {
    pub amount: f64,
    pub currency: String,
    pub amount_summary: String,
}

pub type UserTransactionResponse = Vec<UserTransaction>;
