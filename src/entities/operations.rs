//! SeaORM Entity for operations table
//!
//! Story 3.2 - AC #10, NFR16: Operation state persistence

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "operations")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_address: String,
    pub operation_type: String,
    pub nonce: i64,
    pub status: String,
    pub arb_tx_hash: Option<String>,
    pub orbit_tx_hash: Option<String>,
    pub completion_tx_hash: Option<String>,
    pub amount: Option<String>,
    pub itp_amount: Option<String>,
    pub itp_address: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub retryable: bool,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
