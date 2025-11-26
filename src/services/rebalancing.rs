use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::entities::{
    coingecko_categories, crypto_listings, historical_prices, index_metadata, rebalances,
    prelude::*,
};
use crate::services::coingecko::CoinGeckoService;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinRebalanceInfo {
    pub coin_id: String,
    pub symbol: String,
    pub quantity: String,
    pub weight: String,
    pub price: f64,
    pub exchange: String,
    pub trading_pair: String,
}

#[derive(Debug, Clone)]
pub enum RebalanceReason {
    Initial,
    Periodic,
    Delisting(String),
}

impl RebalanceReason {
    pub fn as_str(&self) -> &str {
        match self {
            RebalanceReason::Initial => "initial",
            RebalanceReason::Periodic => "periodic",
            RebalanceReason::Delisting(_) => "delisting",
        }
    }
}

pub struct RebalancingService {
    db: DatabaseConnection,
    coingecko: CoinGeckoService,
}

impl RebalancingService {
    pub fn new(db: DatabaseConnection, coingecko: CoinGeckoService) -> Self {
        Self { db, coingecko }
    }

    /// Backfill all historical rebalances for an index from initial_date to current_date
    pub async fn backfill_historical_rebalances(
        &self,
        index_id: i32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Get index metadata
        let index = IndexMetadata::find_by_id(index_id)
            .one(&self.db)
            .await?
            .ok_or("Index not found")?;

        let initial_date = index
            .initial_date
            .ok_or("Index has no initial_date")?;
        let rebalance_period = index
            .rebalance_period
            .ok_or("Index has no rebalance_period")?;

        let rebalance_dates = self.calculate_rebalance_dates(
            initial_date,
            rebalance_period,
            Utc::now().date_naive(),
        );

        tracing::info!(
            "Backfilling {} rebalances for index {}",
            rebalance_dates.len(),
            index_id
        );

        for (i, date) in rebalance_dates.iter().enumerate() {
            tracing::info!(
                "Backfilling rebalance {}/{} for index {} on {}",
                i + 1,
                rebalance_dates.len(),
                index_id,
                date
            );

            let reason = if i == 0 {
                RebalanceReason::Initial
            } else {
                RebalanceReason::Periodic
            };

            // Add delay to avoid rate limiting (500ms)
            if i > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }

            // Perform rebalance with retry
            match self.perform_rebalance_with_retry(index_id, *date, reason).await {
                Ok(_) => tracing::info!("Successfully created rebalance for {}", date),
                Err(e) => {
                    tracing::error!("Failed to create rebalance for {}: {}", date, e);
                    // Continue with next date instead of failing entire backfill
                }
            }
        }

        Ok(())
    }

    /// Perform rebalance with exponential backoff retry
    async fn perform_rebalance_with_retry(
        &self,
        index_id: i32,
        date: NaiveDate,
        reason: RebalanceReason,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let max_retries = 5;
        let mut delay = tokio::time::Duration::from_secs(1);

        for attempt in 0..max_retries {
            match self.perform_rebalance_for_date(index_id, date, reason.clone()).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    if attempt == max_retries - 1 {
                        return Err(e);
                    }

                    tracing::warn!(
                        "Rebalance attempt {} failed: {}. Retrying in {:?}",
                        attempt + 1,
                        e,
                        delay
                    );

                    tokio::time::sleep(delay).await;
                    delay *= 2; // Exponential backoff
                }
            }
        }

        Err("Max retries exceeded".into())
    }

    /// Perform rebalance for a specific date
    pub async fn perform_rebalance_for_date(
        &self,
        index_id: i32,
        date: NaiveDate,
        reason: RebalanceReason,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Check if rebalance already exists
        let timestamp = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        let existing = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index_id))
            .filter(rebalances::Column::Timestamp.eq(timestamp))
            .one(&self.db)
            .await?;

        if existing.is_some() {
            tracing::debug!("Rebalance already exists for index {} on {}", index_id, date);
            return Ok(());
        }

        // Get index metadata
        let index = IndexMetadata::find_by_id(index_id)
            .one(&self.db)
            .await?
            .ok_or("Index not found")?;

        let category_id = index
            .coingecko_category
            .ok_or("Index has no coingecko_category")?;

        // Get all tokens in category (from CoinGecko)
        let category_tokens = self.fetch_category_tokens(&category_id).await?;
        tracing::debug!("Found {} tokens in category {}", category_tokens.len(), category_id);

        // Filter tradeable tokens
        let tradeable_tokens = self
            .filter_tradeable_tokens(category_tokens, date)
            .await?;
        tracing::debug!("Found {} tradeable tokens on {}", tradeable_tokens.len(), date);

        if tradeable_tokens.is_empty() {
            return Err("No tradeable tokens found".into());
        }

        // Calculate weights
        let total_category_tokens = 100; // Assuming category has 100 tokens conceptually
        let weight = Decimal::from(total_category_tokens) / Decimal::from(tradeable_tokens.len());

        // Get portfolio value
        let portfolio_value = if matches!(reason, RebalanceReason::Initial) {
            index.initial_price.ok_or("Index has no initial_price")?
        } else {
            self.calculate_current_portfolio_value(index_id, date).await?
        };

        // Calculate quantities
        let target_value_per_token = portfolio_value / Decimal::from(tradeable_tokens.len());

        let mut coins_info = Vec::new();

        for token_info in tradeable_tokens {
            let price = self
                .get_price_for_date(&token_info.coin_id, date)
                .await?
                .ok_or(format!("No price found for {} on {}", token_info.coin_id, date))?;

            let price_decimal = Decimal::from_f64_retain(price)
                .ok_or("Invalid price")?;

            let quantity = target_value_per_token / (weight * price_decimal);

            coins_info.push(CoinRebalanceInfo {
                coin_id: token_info.coin_id,
                symbol: token_info.symbol,
                quantity: quantity.to_string(),
                weight: weight.to_string(),
                price,
                exchange: token_info.exchange,
                trading_pair: token_info.trading_pair,
            });
        }

        // Save to database
        let coins_json = serde_json::to_value(&coins_info)?;
        let total_weight = weight * Decimal::from(coins_info.len());

        let new_rebalance = rebalances::ActiveModel {
            index_id: Set(index_id),
            coins: Set(coins_json),
            portfolio_value: Set(portfolio_value),
            total_weight: Set(total_weight),
            timestamp: Set(timestamp),
            rebalance_type: Set(reason.as_str().to_string()),
            deployed: Set(Some(false)),
            ..Default::default()
        };

        new_rebalance.insert(&self.db).await?;

        tracing::info!(
            "Created {} rebalance for index {} on {} with {} tokens",
            reason.as_str(),
            index_id,
            date,
            coins_info.len()
        );

        Ok(())
    }

    /// Calculate rebalance dates from initial_date to current_date
    fn calculate_rebalance_dates(
        &self,
        initial_date: NaiveDate,
        period_days: i32,
        current_date: NaiveDate,
    ) -> Vec<NaiveDate> {
        let mut dates = vec![initial_date];
        let mut next_date = initial_date + Duration::days(period_days as i64);

        while next_date <= current_date {
            dates.push(next_date);
            next_date = next_date + Duration::days(period_days as i64);
        }

        dates
    }

    /// Fetch tokens in a CoinGecko category
    async fn fetch_category_tokens(
        &self,
        category_id: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: Call CoinGecko API to get tokens in category
        // For now, return mock data
        // In production: self.coingecko.fetch_tokens_by_category(category_id).await
        
        tracing::warn!("Using mock category tokens - implement CoinGecko API call");
        Ok(vec![
            "bitcoin".to_string(),
            "ethereum".to_string(),
            "solana".to_string(),
        ])
    }

    /// Filter tradeable tokens on a specific date
    async fn filter_tradeable_tokens(
        &self,
        coin_ids: Vec<String>,
        date: NaiveDate,
    ) -> Result<Vec<TradeableTokenInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let mut tradeable = Vec::new();

        // Priority: Binance USDC > USDT > Bitget USDC > USDT
        let priorities = [
            ("binance", "usdc"),
            ("binance", "usdt"),
            ("bitget", "usdc"),
            ("bitget", "usdt"),
        ];

        for coin_id in coin_ids {
            for (exchange, pair) in priorities {
                if let Some(info) = self
                    .check_tradeable(&coin_id, exchange, pair, date)
                    .await?
                {
                    tradeable.push(info);
                    break; // Found best pair, move to next token
                }
            }
        }

        Ok(tradeable)
    }

    /// Check if token is tradeable on specific exchange/pair/date
    async fn check_tradeable(
        &self,
        coin_id: &str,
        exchange: &str,
        trading_pair: &str,
        date: NaiveDate,
    ) -> Result<Option<TradeableTokenInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let listing = CryptoListings::find()
            .filter(crypto_listings::Column::CoinId.eq(coin_id))
            .filter(crypto_listings::Column::Exchange.eq(exchange))
            .filter(crypto_listings::Column::TradingPair.eq(trading_pair))
            .one(&self.db)
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
                return Ok(Some(TradeableTokenInfo {
                    coin_id: listing.coin_id,
                    symbol: listing.symbol,
                    exchange: listing.exchange,
                    trading_pair: listing.trading_pair,
                }));
            }
        }

        Ok(None)
    }

    /// Get price for a coin on a specific date
    async fn get_price_for_date(
        &self,
        coin_id: &str,
        date: NaiveDate,
    ) -> Result<Option<f64>, Box<dyn std::error::Error + Send + Sync>> {
        let target_timestamp = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

        let price_row = HistoricalPrices::find()
            .filter(historical_prices::Column::CoinId.eq(coin_id))
            .filter(historical_prices::Column::Timestamp.lte(target_timestamp))
            .order_by(historical_prices::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(&self.db)
            .await?;

        Ok(price_row.map(|p| p.price))
    }

    /// Calculate current portfolio value
    async fn calculate_current_portfolio_value(
        &self,
        index_id: i32,
        date: NaiveDate,
    ) -> Result<Decimal, Box<dyn std::error::Error + Send + Sync>> {
        // Get last rebalance before this date
        let timestamp = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

        let last_rebalance = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index_id))
            .filter(rebalances::Column::Timestamp.lt(timestamp))
            .order_by(rebalances::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(&self.db)
            .await?
            .ok_or("No previous rebalance found")?;

        let coins: Vec<CoinRebalanceInfo> = serde_json::from_value(last_rebalance.coins)?;

        let mut total_value = Decimal::ZERO;

        for coin in coins {
            let current_price = self
                .get_price_for_date(&coin.coin_id, date)
                .await?
                .ok_or(format!("No price for {} on {}", coin.coin_id, date))?;

            let quantity = coin.quantity.parse::<Decimal>()?;
            let price_decimal = Decimal::from_f64_retain(current_price)
                .ok_or("Invalid price")?;
            let weight = coin.weight.parse::<Decimal>()?;

            total_value += weight * quantity * price_decimal;
        }

        Ok(total_value)
    }
}

#[derive(Debug, Clone)]
struct TradeableTokenInfo {
    coin_id: String,
    symbol: String,
    exchange: String,
    trading_pair: String,
}
