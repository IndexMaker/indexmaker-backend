//! `SeaORM` Entity for sync_status table

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "sync_status")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub job_name: String,
    pub last_success_at: Option<DateTime>,
    pub last_attempt_at: Option<DateTime>,
    pub last_error: Option<String>,
    pub success_count: i64,
    pub error_count: i64,
    pub min_interval_secs: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
