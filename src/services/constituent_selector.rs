use chrono::NaiveDate;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect};
use std::sync::Arc;

use crate::entities::{
    category_membership, crypto_listings, index_constituents, prelude::*,
};
use crate::services::market_cap::MarketCapService;

/// Represents a constituent token with trading information
#[derive(Debug, Clone)]
pub struct ConstituentToken {
    pub coin_id: String,
    pub symbol: String,
    pub exchange: String,
    pub trading_pair: String,
}

/// Fixed constituents strategy - uses index_constituents table
pub struct FixedConstituentSelector {
    index_id: i32,
}

impl FixedConstituentSelector {
    pub fn new(index_id: i32) -> Self {
        Self { index_id }
    }

    pub async fn select_constituents(
        &self,
        db: &DatabaseConnection,
        _date: NaiveDate,
    ) -> Result<Vec<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::debug!("Selecting fixed constituents for index {}", self.index_id);

        // Query index_constituents table
        let constituents = IndexConstituents::find()
            .filter(index_constituents::Column::IndexId.eq(self.index_id))
            .filter(index_constituents::Column::RemovedAt.is_null())
            .all(db)
            .await?;

        // Map to ConstituentToken with coin_id lookup
        let mut tokens = Vec::new();
        for constituent in constituents {
            // Lookup coin_id from category_membership by symbol
            let membership = CategoryMembership::find()
                .filter(category_membership::Column::Symbol.eq(&constituent.token_symbol))
                .filter(category_membership::Column::RemovedDate.is_null())
                .limit(1)
                .one(db)
                .await?;

            match membership {
                Some(member) => {
                    tokens.push(ConstituentToken {
                        coin_id: member.coin_id,
                        symbol: constituent.token_symbol,
                        exchange: constituent.exchange,
                        trading_pair: constituent.trading_pair,
                    });
                }
                None => {
                    tracing::warn!(
                        "Could not find coin_id for symbol {}",
                        constituent.token_symbol
                    );
                }
            }
        }

        tracing::info!(
            "Selected {} fixed constituents for index {}",
            tokens.len(),
            self.index_id
        );

        Ok(tokens)
    }

    pub fn strategy_name(&self) -> &str {
        "Fixed Constituents"
    }
}

/// Top market cap strategy - selects top N tokens by market cap
pub struct TopMarketCapSelector {
    top_n: usize,
    market_cap_service: Arc<MarketCapService>,
}

impl TopMarketCapSelector {
    pub fn new(top_n: usize, market_cap_service: Arc<MarketCapService>) -> Self {
        Self {
            top_n,
            market_cap_service,
        }
    }

    pub async fn select_constituents(
        &self,
        db: &DatabaseConnection,
        date: NaiveDate,
    ) -> Result<Vec<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(
            "Selecting top {} tokens by market cap on {}",
            self.top_n,
            date
        );

        // 1. Get top N*3 by market cap (300 for top 100, provides buffer for tradeability)
        let top_coins = self
            .market_cap_service
            .get_top_tokens_by_market_cap(db, date, self.top_n * 3)
            .await?;

        // 2. Filter for tradeable tokens
        let mut tradeable = Vec::new();

        for market_cap_data in top_coins {
            // Try to find tradeable pair for this coin
            if let Some(token) = self
                .find_tradeable_token(
                    db,
                    &market_cap_data.coin_id,
                    &market_cap_data.symbol,
                    date,
                )
                .await?
            {
                tradeable.push(token);

                // Stop once we have enough
                if tradeable.len() >= self.top_n {
                    break;
                }
            }
        }

        tracing::info!(
            "Selected {} tradeable tokens from top {} by market cap",
            tradeable.len(),
            self.top_n * 3
        );

        Ok(tradeable)
    }

    pub fn strategy_name(&self) -> &str {
        "Top Market Cap"
    }

    /// Find tradeable token info with priority: Binance USDC > USDT > Bitget USDC > USDT
    async fn find_tradeable_token(
        &self,
        db: &DatabaseConnection,
        coin_id: &str,
        symbol: &str,
        date: NaiveDate,
    ) -> Result<Option<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        let priorities = [
            ("binance", "usdc"),
            ("binance", "usdt"),
            ("bitget", "usdc"),
            ("bitget", "usdt"),
        ];

        for (exchange, pair) in priorities {
            let listing = CryptoListings::find()
                .filter(crypto_listings::Column::CoinId.eq(coin_id))
                .filter(crypto_listings::Column::Exchange.eq(exchange))
                .filter(crypto_listings::Column::TradingPair.eq(pair))
                .one(db)
                .await?;

            if let Some(listing) = listing {
                // Check if listed on this date
                let is_listed = listing
                    .listing_date
                    .map(|d| d.date() <= date)
                    .unwrap_or(false);

                // Check if NOT delisted yet
                let not_delisted = listing
                    .delisting_date
                    .map(|d| d.date() > date)
                    .unwrap_or(true);

                if is_listed && not_delisted {
                    return Ok(Some(ConstituentToken {
                        coin_id: coin_id.to_string(),
                        symbol: symbol.to_string(),
                        exchange: exchange.to_string(),
                        trading_pair: pair.to_string(),
                    }));
                }
            }
        }

        Ok(None)
    }
}

/// Category-based strategy - selects tokens from a specific CoinGecko category
pub struct CategoryBasedSelector {
    category_id: String,
    market_cap_service: Arc<MarketCapService>,
}

impl CategoryBasedSelector {
    pub fn new(category_id: String, market_cap_service: Arc<MarketCapService>) -> Self {
        Self {
            category_id,
            market_cap_service,
        }
    }

    pub async fn select_constituents(
        &self,
        db: &DatabaseConnection,
        date: NaiveDate,
    ) -> Result<Vec<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(
            "Selecting tokens from category '{}' on {}",
            self.category_id,
            date
        );

        // 1. Get tokens in this category on this date
        let category_tokens = self.get_category_tokens_for_date(db, date).await?;

        if category_tokens.is_empty() {
            tracing::warn!("No tokens found in category '{}'", self.category_id);
            return Ok(Vec::new());
        }

        // 2. Get market cap for these tokens on this date
        let with_market_cap = self
            .market_cap_service
            .get_market_caps_for_coins(db, category_tokens, date)
            .await?;

        // 3. Sort by market cap (highest first)
        let mut sorted = with_market_cap;
        sorted.sort_by(|a, b| b.market_cap.total_cmp(&a.market_cap));

        // 4. Filter for tradeability
        let mut tradeable = Vec::new();

        for coin_data in sorted {
            if let Some(token) = self
                .find_tradeable_token(db, &coin_data.coin_id, &coin_data.symbol, date)
                .await?
            {
                tradeable.push(token);
            }
        }

        tracing::info!(
            "Selected {} tradeable tokens from category '{}'",
            tradeable.len(),
            self.category_id
        );

        Ok(tradeable)
    }

    pub fn strategy_name(&self) -> &str {
        "Category Based"
    }

    async fn get_category_tokens_for_date(
        &self,
        db: &DatabaseConnection,
        date: NaiveDate,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let date_time = date.and_hms_opt(0, 0, 0).unwrap();

        let memberships = CategoryMembership::find()
            .filter(category_membership::Column::CategoryId.eq(&self.category_id))
            .filter(category_membership::Column::AddedDate.lte(date_time))
            .filter(
                category_membership::Column::RemovedDate
                    .is_null()
                    .or(category_membership::Column::RemovedDate.gt(date_time)),
            )
            .all(db)
            .await?;

        Ok(memberships.into_iter().map(|m| m.coin_id).collect())
    }

    async fn find_tradeable_token(
        &self,
        db: &DatabaseConnection,
        coin_id: &str,
        symbol: &str,
        date: NaiveDate,
    ) -> Result<Option<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        let priorities = [
            ("binance", "usdc"),
            ("binance", "usdt"),
            ("bitget", "usdc"),
            ("bitget", "usdt"),
        ];

        for (exchange, pair) in priorities {
            let listing = CryptoListings::find()
                .filter(crypto_listings::Column::CoinId.eq(coin_id))
                .filter(crypto_listings::Column::Exchange.eq(exchange))
                .filter(crypto_listings::Column::TradingPair.eq(pair))
                .one(db)
                .await?;

            if let Some(listing) = listing {
                let is_listed = listing
                    .listing_date
                    .map(|d| d.date() <= date)
                    .unwrap_or(false);

                let not_delisted = listing
                    .delisting_date
                    .map(|d| d.date() > date)
                    .unwrap_or(true);

                if is_listed && not_delisted {
                    return Ok(Some(ConstituentToken {
                        coin_id: coin_id.to_string(),
                        symbol: symbol.to_string(),
                        exchange: exchange.to_string(),
                        trading_pair: pair.to_string(),
                    }));
                }
            }
        }

        Ok(None)
    }
}

/// Enum that holds all constituent selector types
pub enum ConstituentSelectorEnum {
    Fixed(FixedConstituentSelector),
    TopMarketCap(TopMarketCapSelector),
    CategoryBased(CategoryBasedSelector),
}

impl ConstituentSelectorEnum {
    /// Select constituents using the appropriate strategy
    pub async fn select_constituents(
        &self,
        db: &DatabaseConnection,
        date: NaiveDate,
    ) -> Result<Vec<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            ConstituentSelectorEnum::Fixed(selector) => selector.select_constituents(db, date).await,
            ConstituentSelectorEnum::TopMarketCap(selector) => selector.select_constituents(db, date).await,
            ConstituentSelectorEnum::CategoryBased(selector) => selector.select_constituents(db, date).await,
        }
    }

    /// Get strategy name
    pub fn strategy_name(&self) -> &str {
        match self {
            ConstituentSelectorEnum::Fixed(selector) => selector.strategy_name(),
            ConstituentSelectorEnum::TopMarketCap(selector) => selector.strategy_name(),
            ConstituentSelectorEnum::CategoryBased(selector) => selector.strategy_name(),
        }
    }
}

/// Factory for creating appropriate constituent selectors
pub struct ConstituentSelectorFactory {
    market_cap_service: Arc<MarketCapService>,
}

impl ConstituentSelectorFactory {
    pub fn new(market_cap_service: Arc<MarketCapService>) -> Self {
        Self { market_cap_service }
    }

    pub async fn create_selector(
        &self,
        db: &DatabaseConnection,
        index: &crate::entities::index_metadata::Model,
    ) -> Result<ConstituentSelectorEnum, Box<dyn std::error::Error + Send + Sync>> {
        // Strategy 1: Check for fixed constituents
        let has_fixed = IndexConstituents::find()
            .filter(index_constituents::Column::IndexId.eq(index.index_id))
            .filter(index_constituents::Column::RemovedAt.is_null())
            .count(db)
            .await?
            > 0;

        if has_fixed {
            tracing::info!(
                "Index {} ({}) uses Fixed Constituents strategy",
                index.index_id,
                index.symbol
            );
            return Ok(ConstituentSelectorEnum::Fixed(
                FixedConstituentSelector::new(index.index_id)
            ));
        }

        // Strategy 2: Check for category-based (has coingecko_category)
        if let Some(ref category) = index.coingecko_category {
            // Special case: if symbol starts with "SY" followed by digits, it's market cap based
            if index.symbol.starts_with("SY")
                && index.symbol.len() > 2
                && index.symbol[2..].chars().all(|c| c.is_numeric())
            {
                // Extract number: SY100 -> 100
                let top_n: usize = index.symbol[2..]
                    .parse()
                    .map_err(|_| format!("Invalid SY format: {}", index.symbol))?;

                tracing::info!(
                    "Index {} ({}) uses Top {} Market Cap strategy",
                    index.index_id,
                    index.symbol,
                    top_n
                );
                return Ok(ConstituentSelectorEnum::TopMarketCap(
                    TopMarketCapSelector::new(top_n, self.market_cap_service.clone())
                ));
            }

            // Otherwise, it's category-based
            tracing::info!(
                "Index {} ({}) uses Category-Based strategy (category: {})",
                index.index_id,
                index.symbol,
                category
            );
            return Ok(ConstituentSelectorEnum::CategoryBased(
                CategoryBasedSelector::new(category.clone(), self.market_cap_service.clone())
            ));
        }

        Err(format!(
            "Cannot determine constituent selection strategy for index {} ({})",
            index.index_id, index.symbol
        )
        .into())
    }
}