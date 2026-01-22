//! SeaORM Entity for ITPs (Index Token Products)
//!
//! Stores ITP metadata for listing and discovery.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "itps")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// ITP contract address on Orbit chain (0x format, 42 chars)
    pub orbit_address: String,
    /// BridgedItp contract address on Arbitrum (0x format, 42 chars)
    pub arbitrum_address: Option<String>,
    /// Index ID linking to index_metadata table (optional)
    pub index_id: Option<i64>,
    /// ITP name (e.g., "Top 10 DeFi Index")
    pub name: String,
    /// ITP symbol (e.g., "DEFI10")
    pub symbol: String,
    /// Initial price in smallest USDC units (6 decimals)
    pub initial_price: Option<Decimal>,
    /// Current price in smallest USDC units (6 decimals)
    pub current_price: Option<Decimal>,
    /// Total supply in smallest token units (18 decimals)
    #[sea_orm(column_type = "Decimal(Some((78, 0)))")]
    pub total_supply: Option<Decimal>,
    /// State: 0=initiated, 1=active/approved, 2=paused, 3=deprecated
    pub state: i16,
    /// Timestamp when ITP was created
    pub created_at: Option<DateTimeWithTimeZone>,
    /// Timestamp when ITP was last updated
    pub updated_at: Option<DateTimeWithTimeZone>,
    /// Transaction hash of deployment
    pub deploy_tx_hash: Option<String>,
    /// Investment methodology
    pub methodology: Option<String>,
    /// ITP description
    pub description: Option<String>,
    /// Asset symbols as JSON array
    #[sea_orm(column_type = "JsonBinary")]
    pub assets: Option<Json>,
    /// Asset weights as JSON array
    #[sea_orm(column_type = "JsonBinary")]
    pub weights: Option<Json>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
