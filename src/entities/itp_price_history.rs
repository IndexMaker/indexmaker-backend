//! SeaORM Entity for ITP price history time-series storage

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "itp_price_history")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// ITP vault address (0x + 64 hex chars)
    pub itp_id: String,
    /// Price at snapshot time (high precision)
    #[sea_orm(column_type = "Decimal(Some((78, 18)))")]
    pub price: Decimal,
    /// Optional trading volume
    #[sea_orm(column_type = "Decimal(Some((78, 18)))", nullable)]
    pub volume: Option<Decimal>,
    /// Timestamp of the price snapshot
    pub timestamp: DateTimeWithTimeZone,
    /// Granularity: '5min', 'hourly', 'daily'
    pub granularity: String,
    /// When the record was created
    pub created_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
