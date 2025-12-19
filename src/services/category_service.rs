use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::entities::{category_membership, coingecko_categories, prelude::*};

/// Get the primary category/sector for a coin
/// Returns the human-readable category name (e.g., "Layer 1 (L1)")
pub async fn get_coin_category(
    db: &DatabaseConnection,
    coin_id: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Get active category membership
    let membership = CategoryMembership::find()
        .filter(category_membership::Column::CoinId.eq(coin_id))
        .filter(category_membership::Column::RemovedDate.is_null())
        .one(db)
        .await?;

    if let Some(membership) = membership {
        // Get category name from coingecko_categories
        let category = CoingeckoCategories::find()
            .filter(coingecko_categories::Column::CategoryId.eq(&membership.category_id))
            .one(db)
            .await?;

        if let Some(category) = category {
            return Ok(category.name);
        }
    }

    // Default if no category found
    Ok("Uncategorized".to_string())
}