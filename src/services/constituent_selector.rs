use chrono::NaiveDate;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use lazy_static::lazy_static;
use std::collections::HashSet;

use crate::entities::{
    category_membership, coins_historical_prices, crypto_listings, index_constituents, prelude::*,
};
use crate::services::exchange_api::ExchangeApiService;

lazy_static! {
    /// Blacklisted CoinGecko categories (derivatives, wrapped tokens, stablecoins, etc.)
    static ref BLACKLISTED_CATEGORIES: HashSet<&'static str> = {
        let mut set = HashSet::new();
        // Stablecoins
        set.insert("stablecoins");
        set.insert("usd-stablecoin");
        set.insert("fiat-backed-stablecoin");
        set.insert("bridged-stablecoin");
        set.insert("yield-bearing-stablecoin");
        set.insert("bridged-usdt");
        set.insert("synthetic-dollar");
        set.insert("bridged-usdc");
        set.insert("us-treasury-backed-stablecoin");
        set.insert("eur-stablecoins");
        set.insert("algorithmic-stablecoins");
        set.insert("jpy-stablecoin");
        set.insert("sgd-stablcoin");
        set.insert("idr-stablecoin");
        set.insert("cny-stablecoin");
        set.insert("gbp-stablecoin");
        set.insert("bridged-frax");
        
        // Liquid Staking / Restaking
        set.insert("liquid-staked-eth");
        set.insert("liquid-staking-tokens");
        set.insert("liquid-staking");
        set.insert("liquid-staked-sol");
        set.insert("restaking");
        set.insert("liquid-restaking-tokens");
        set.insert("liquid-restaked-eth");
        set.insert("liquid-restaked-sol");
        set.insert("liquid-staked-btc");
        set.insert("liquid-staked-hype");
        set.insert("liquid-staked-sui");
        
        // Bridged / Wrapped Tokens
        set.insert("bridged-tokens");
        set.insert("wrapped-tokens");
        set.insert("tokenized-btc");
        set.insert("bridged-wbtc");
        set.insert("bridged-weth");
        set.insert("bridged-dai");
        set.insert("binance-peg-tokens");
        set.insert("bridged-wsteth");
        
        // Tokenized Assets
        set.insert("crypto-backed-tokens");
        set.insert("tokenized-gold");
        set.insert("tokenized-private-credit");
        set.insert("tokenized-assets");
        set.insert("tokenized-commodities");
        set.insert("tokenized-treasury-bills-t-bills");
        set.insert("tokenized-treasury-bonds-t-bonds");
        set.insert("tokenized-silver");
        set.insert("tokenized-stock");
        set.insert("tokenized-real-estate");
        set.insert("tokenized-exchange-traded-funds-etfs");
        
        // Protocol-Specific / Ecosystem Tokens
        set.insert("morpho-ecosystem");
        set.insert("hyperunit-ecosystem");
        set.insert("aave-tokens");
        set.insert("midas-liquid-yield-tokens");
        set.insert("compound-tokens");
        set.insert("backedfi-xstocks-ecosystem");
        set.insert("tokensets-ecosystem");
        set.insert("realt-tokens");
        
        // Yield / Synthetic / Index
        set.insert("yield-bearing");
        set.insert("lp-tokens");
        set.insert("btcfi-protocol");
        set.insert("seigniorage");
        set.insert("synthetic");
        set.insert("synthetic-asset");
        set.insert("breeding");
        set.insert("defi-index");
        set.insert("yield-tokenization-product");
        
        set
    };

    /// Whitelisted coin_ids that should be included even if in blacklisted categories
    static ref WHITELISTED_COIN_IDS: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.insert("morpho"); // MORPHO - even though in "morpho-ecosystem"
        set.insert("havven"); // SNX - coin_id for Synthetix
        set
    };
    
    /// Whitelisted symbols (backup check)
    static ref WHITELISTED_SYMBOLS: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.insert("MORPHO");
        set.insert("SNX");
        set
    };
}

/// Represents a constituent token with trading information
#[derive(Debug, Clone)]
pub struct ConstituentToken {
    pub coin_id: String,
    pub symbol: String,
    pub exchange: String,
    pub trading_pair: String,
}

/// Check if a coin is in any blacklisted category
async fn is_in_blacklisted_category(
    db: &DatabaseConnection,
    coin_id: &str,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Get all active categories for this coin
    let memberships = CategoryMembership::find()
        .filter(category_membership::Column::CoinId.eq(coin_id))
        .filter(category_membership::Column::RemovedDate.is_null())
        .all(db)
        .await?;

    // Check if any category is blacklisted
    for membership in memberships {
        let category_normalized = membership.category_id.to_lowercase();
        if BLACKLISTED_CATEGORIES.contains(category_normalized.as_str()) {
            tracing::debug!(
                "Coin {} is in blacklisted category: {}",
                coin_id,
                membership.category_id
            );
            return Ok(true);
        }
    }

    Ok(false)
}

/// Check if a coin should be whitelisted (override blacklist)
fn is_whitelisted(coin_id: &str, symbol: &str) -> bool {
    let coin_id_lower = coin_id.to_lowercase();
    let symbol_upper = symbol.to_uppercase();
    
    WHITELISTED_COIN_IDS.contains(coin_id_lower.as_str()) 
        || WHITELISTED_SYMBOLS.contains(symbol_upper.as_str())
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
        exchange_api: Option<&ExchangeApiService>,
        date: NaiveDate,
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
                    // Use exchange_api if provided (scheduled mode)
                    if exchange_api.is_some() {
                        // For scheduled mode, verify tradeability via live API
                        if let Some(token) = find_tradeable_token(
                            db,
                            exchange_api,
                            &member.coin_id,
                            &constituent.token_symbol,
                            date,
                        )
                        .await?
                        {
                            tokens.push(token);
                        } else {
                            tracing::warn!(
                                "Fixed constituent {} not tradeable on any exchange (live check)",
                                constituent.token_symbol
                            );
                        }
                    } else {
                        // For backfill mode, use stored exchange/pair from constituents table
                        tokens.push(ConstituentToken {
                            coin_id: member.coin_id,
                            symbol: constituent.token_symbol,
                            exchange: constituent.exchange,
                            trading_pair: constituent.trading_pair,
                        });
                    }
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
}

impl TopMarketCapSelector {
    pub fn new(top_n: usize) -> Self {
        Self { top_n }
    }

    pub async fn select_constituents(
        &self,
        db: &DatabaseConnection,
        exchange_api: Option<&ExchangeApiService>,
        date: NaiveDate,
    ) -> Result<Vec<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(
            "Selecting top {} tokens by market cap on {}",
            self.top_n,
            date
        );

        // 1. Query coins_historical_prices for top N*50 by market cap (extra buffer for filtering)
        let top_coins = query_top_coins_by_market_cap(db, date, self.top_n * 5).await?;

        if top_coins.is_empty() {
            tracing::warn!("No coins with market cap data found for {}", date);
            return Ok(Vec::new());
        }

        tracing::debug!(
            "Found {} coins with market cap data on {}",
            top_coins.len(),
            date
        );

        // 2. Filter out blacklisted categories (stablecoins, wrapped tokens, etc.)
        let mut white_coins = Vec::new();
        
        for coin_data in top_coins {
            // Check whitelist first (overrides blacklist)
            if is_whitelisted(&coin_data.coin_id, &coin_data.symbol) {
                tracing::debug!(
                    "Whitelisted: {} ({}) - included despite category",
                    coin_data.symbol,
                    coin_data.coin_id
                );
                white_coins.push(coin_data);
                continue;
            }

            // Check if in blacklisted category
            match is_in_blacklisted_category(db, &coin_data.coin_id).await {
                Ok(true) => {
                    tracing::debug!(
                        "Filtered out: {} ({}) - in blacklisted category",
                        coin_data.symbol,
                        coin_data.coin_id
                    );
                    continue;
                }
                Ok(false) => {
                    white_coins.push(coin_data);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to check categories for {} ({}): {}",
                        coin_data.symbol,
                        coin_data.coin_id,
                        e
                    );
                    // Include on error (fail-open)
                    white_coins.push(coin_data);
                }
            }
        }

        tracing::info!(
            "After category filtering: {} coins remaining from {} candidates",
            white_coins.len(),
            self.top_n * 5
        );

        // 3. Filter for tradeable tokens
        let mut tradeable = Vec::new();

        for coin_data in white_coins {
            if let Some(token) = find_tradeable_token(
                db,
                exchange_api,
                &coin_data.coin_id,
                &coin_data.symbol,
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
            "Selected {} tradeable tokens from top market cap (target: {})",
            tradeable.len(),
            self.top_n
        );

        if tradeable.len() < self.top_n {
            tracing::warn!(
                "Only found {} tradeable tokens out of {} requested",
                tradeable.len(),
                self.top_n
            );
        }

        Ok(tradeable)
    }

    pub fn strategy_name(&self) -> &str {
        "Top Market Cap"
    }
}

/// Category-based strategy - selects tokens from a specific CoinGecko category
pub struct CategoryBasedSelector {
    category_id: String,
}

impl CategoryBasedSelector {
    pub fn new(category_id: String) -> Self {
        Self { category_id }
    }

    pub async fn select_constituents(
        &self,
        db: &DatabaseConnection,
        exchange_api: Option<&ExchangeApiService>,
        date: NaiveDate,
    ) -> Result<Vec<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(
            "Selecting tokens from category '{}' on {}",
            self.category_id,
            date
        );

        // 1. Get tokens in this category on this date
        let category_coin_ids = self.get_category_tokens_for_date(db, date).await?;

        if category_coin_ids.is_empty() {
            tracing::warn!("No tokens found in category '{}'", self.category_id);
            return Ok(Vec::new());
        }

        // 2. Get market cap for these tokens on this date from coins_historical_prices
        let with_market_cap = query_market_caps_for_coins(db, category_coin_ids, date).await?;

        if with_market_cap.is_empty() {
            tracing::warn!(
                "No market cap data found for category '{}' on {}",
                self.category_id,
                date
            );
            return Ok(Vec::new());
        }

        // 3. Already sorted by market cap DESC from query
        tracing::debug!(
            "Found {} tokens with market cap in category '{}'",
            with_market_cap.len(),
            self.category_id
        );

        // 4. Filter for tradeability
        let mut tradeable = Vec::new();

        for coin_data in with_market_cap {
            if let Some(token) = find_tradeable_token(
                db,
                exchange_api,
                &coin_data.coin_id,
                &coin_data.symbol,
                date,
            )
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
        exchange_api: Option<&ExchangeApiService>,
        date: NaiveDate,
    ) -> Result<Vec<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            ConstituentSelectorEnum::Fixed(selector) => {
                selector.select_constituents(db, exchange_api, date).await
            }
            ConstituentSelectorEnum::TopMarketCap(selector) => {
                selector.select_constituents(db, exchange_api, date).await
            }
            ConstituentSelectorEnum::CategoryBased(selector) => {
                selector.select_constituents(db, exchange_api, date).await
            }
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
pub struct ConstituentSelectorFactory;

impl ConstituentSelectorFactory {
    pub fn new() -> Self {
        Self
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
                    TopMarketCapSelector::new(top_n)
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
                CategoryBasedSelector::new(category.clone())
            ));
        }

        Err(format!(
            "Cannot determine constituent selection strategy for index {} ({})",
            index.index_id, index.symbol
        )
        .into())
    }
}

// ========== HELPER FUNCTIONS ==========

/// Struct to hold coin data from coins_historical_prices
#[derive(Debug, Clone)]
struct CoinMarketCapData {
    coin_id: String,
    symbol: String,
}

/// Query top N coins by market cap for a specific date
async fn query_top_coins_by_market_cap(
    db: &DatabaseConnection,
    date: NaiveDate,
    limit: usize,
) -> Result<Vec<CoinMarketCapData>, Box<dyn std::error::Error + Send + Sync>> {
    let records = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::Date.eq(date))
        .filter(coins_historical_prices::Column::MarketCap.is_not_null())
        .order_by(coins_historical_prices::Column::MarketCap, Order::Desc)
        .limit(limit as u64)
        .all(db)
        .await?;

    Ok(records
        .into_iter()
        .map(|r| CoinMarketCapData {
            coin_id: r.coin_id,
            symbol: r.symbol,
        })
        .collect())
}

/// Query market caps for specific coins on a date, sorted by market cap DESC
async fn query_market_caps_for_coins(
    db: &DatabaseConnection,
    coin_ids: Vec<String>,
    date: NaiveDate,
) -> Result<Vec<CoinMarketCapData>, Box<dyn std::error::Error + Send + Sync>> {
    let records = CoinsHistoricalPrices::find()
        .filter(coins_historical_prices::Column::Date.eq(date))
        .filter(coins_historical_prices::Column::CoinId.is_in(coin_ids))
        .filter(coins_historical_prices::Column::MarketCap.is_not_null())
        .order_by(coins_historical_prices::Column::MarketCap, Order::Desc)
        .all(db)
        .await?;

    Ok(records
        .into_iter()
        .map(|r| CoinMarketCapData {
            coin_id: r.coin_id,
            symbol: r.symbol,
        })
        .collect())
}

/// Find tradeable token info with priority: Binance USDC > USDT > Bitget USDC > USDT
/// 
/// If exchange_api is provided (scheduled mode), uses live APIs
/// If exchange_api is None (backfill mode), uses crypto_listings table
async fn find_tradeable_token(
    db: &DatabaseConnection,
    exchange_api: Option<&ExchangeApiService>,
    coin_id: &str,
    symbol: &str,
    date: NaiveDate,
) -> Result<Option<ConstituentToken>, Box<dyn std::error::Error + Send + Sync>> {
    // If exchange_api is provided, use live APIs (scheduled mode)
    if let Some(api) = exchange_api {
        tracing::debug!(
            "Using live exchange APIs to check tradeability for {} ({})",
            symbol,
            coin_id
        );

        // Query live exchange APIs
        match api.get_tradeable_tokens(vec![symbol.to_string()]).await {
            Ok(tradeable_tokens) => {
                if let Some(token) = tradeable_tokens.first() {
                    return Ok(Some(ConstituentToken {
                        coin_id: coin_id.to_string(),
                        symbol: token.symbol.clone(),
                        exchange: token.exchange.clone(),
                        trading_pair: token.trading_pair.clone(),
                    }));
                } else {
                    tracing::debug!("Symbol {} not tradeable on any exchange (live check)", symbol);
                    return Ok(None);
                }
            }
            Err(e) => {
                // Log error but don't fail - this will cause rebalance to skip
                tracing::error!(
                    "Exchange API failed for {}: {}. Cannot determine tradeability.",
                    symbol,
                    e
                );
                return Err(format!("Exchange API failure: {}", e).into());
            }
        }
    }

    // Otherwise, use crypto_listings (backfill/historical mode)
    tracing::debug!(
        "Using crypto_listings to check tradeability for {} ({})",
        symbol,
        coin_id
    );

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