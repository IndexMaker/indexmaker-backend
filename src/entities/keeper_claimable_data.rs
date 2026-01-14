//! SeaORM Entity for keeper claimable data time-series storage

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "keeper_claimable_data")]
pub struct Model {
    /// Keeper address in 0x format
    #[sea_orm(primary_key, auto_increment = false)]
    pub keeper_address: String,
    /// Timestamp when data was recorded
    #[sea_orm(primary_key, auto_increment = false)]
    pub recorded_at: DateTime,
    /// First value from getClaimableAcquisition tuple
    #[sea_orm(column_name = "acquisition_value1")]
    pub acquisition_value_1: Decimal,
    /// Second value from getClaimableAcquisition tuple
    #[sea_orm(column_name = "acquisition_value2")]
    pub acquisition_value_2: Decimal,
    /// First value from getClaimableDisposal tuple
    #[sea_orm(column_name = "disposal_value1")]
    pub disposal_value_1: Decimal,
    /// Second value from getClaimableDisposal tuple
    #[sea_orm(column_name = "disposal_value2")]
    pub disposal_value_2: Decimal,
    /// Raw response data for debugging
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub raw_response: Option<Json>,
    pub created_at: Option<DateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
