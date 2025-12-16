use chrono::{Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, Set};
use serde::{Deserialize, Serialize};

use crate::entities::{
    rebalances,
    prelude::*,
};
use crate::services::coingecko::CoinGeckoService;

use crate::services::constituent_selector::ConstituentSelectorFactory;
use crate::services::exchange_api::ExchangeApiService;
use crate::services::price_utils::get_coins_historical_price_for_date;


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
    selector_factory: ConstituentSelectorFactory,
    exchange_api: Option<ExchangeApiService>,
}

impl RebalancingService {
    pub fn new(
        db: DatabaseConnection,
        coingecko: CoinGeckoService,
        exchange_api: Option<ExchangeApiService>,
    ) -> Self {
        Self {
            db,
            coingecko,
            selector_factory: ConstituentSelectorFactory::new(),
            exchange_api,
        }
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

        // Find the last existing rebalance (if any)
        let last_rebalance = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index_id))
            .order_by(rebalances::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(&self.db)
            .await?;

        let not_partial = last_rebalance.is_none();
        // Determine starting point for backfill
        let start_date = match last_rebalance {
            Some(rb) => {
                // Convert timestamp to date
                let last_date = chrono::DateTime::from_timestamp(rb.timestamp, 0)
                    .unwrap()
                    .date_naive();

                // Start from the NEXT period after last rebalance
                let next_date = last_date + Duration::days(rebalance_period as i64);

                tracing::info!(
                    "Resuming backfill for index {} from {} (last rebalance: {})",
                    index_id,
                    next_date,
                    last_date
                );

                next_date
            }
            None => {
                tracing::info!(
                    "Starting fresh backfill for index {} from {}",
                    index_id,
                    initial_date
                );

                initial_date
            }
        };

        let today = Utc::now().date_naive();

        // Calculate rebalance dates from start_date to today
        let rebalance_dates = self.calculate_rebalance_dates(
            start_date,
            rebalance_period,
            today,
        );

        if rebalance_dates.is_empty() {
            tracing::info!("No rebalances needed for index {} (already up to date)", index_id);
            return Ok(());
        }

        tracing::info!(
            "Backfilling {} rebalances for index {} (from {} to {})",
            rebalance_dates.len(),
            index_id,
            start_date,
            today
        );

        for (i, date) in rebalance_dates.iter().enumerate() {
            tracing::info!(
                "Backfilling rebalance {}/{} for index {} on {}",
                i + 1,
                rebalance_dates.len(),
                index_id,
                date
            );

            let reason = if i == 0 && not_partial {
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

        let selector = self.selector_factory
            .create_selector(&self.db, &index)
            .await?;

        tracing::info!(
            "Using {} strategy for index {} ({})",
            selector.strategy_name(),
            index.index_id,
            index.symbol
        );

        // Determine mode: scheduled uses exchange_api, backfill uses None
        let use_live_apis = matches!(reason, RebalanceReason::Periodic) && self.exchange_api.is_some();

        if use_live_apis {
            tracing::info!("🔴 LIVE MODE: Using exchange APIs for real-time tradeability checks");
        } else {
            tracing::info!("📊 BACKFILL MODE: Using crypto_listings for historical data");
        }

        // Pass exchange_api only for scheduled rebalances
        let exchange_api_ref = if use_live_apis {
            self.exchange_api.as_ref()
        } else {
            None
        };

        // Get constituents using the strategy
        let constituents = selector
            .select_constituents(&self.db, exchange_api_ref, date)
            .await?;

        if constituents.is_empty() {
            return Err(format!("No constituents found for index {}", index_id).into());
        }

        tracing::info!(
            "Selected {} constituents for index {} on {}",
            constituents.len(),
            index.index_id,
            date
        );

        // Calculate total number for weight calculation
        let total_category_tokens = constituents.len();

        // Calculate weights based on ACTUAL category size
        let weight = Decimal::from(total_category_tokens) / Decimal::from(constituents.len());

        // Get portfolio value BEFORE fees
        let portfolio_value_before_fees = if matches!(reason, RebalanceReason::Initial) {
            index.initial_price.ok_or("Index has no initial_price")?
        } else {
            self.calculate_current_portfolio_value(index_id, date).await?
        };

        // Calculate quantities
        let target_value_per_token = portfolio_value_before_fees / Decimal::from(constituents.len());

        let mut coins_info = Vec::new();

        for token_info in constituents {
            // Use SELF-HEALING function that auto-fetches missing prices
            let price = crate::services::price_utils::get_or_fetch_coins_historical_price(
                &self.db,
                &self.coingecko,
                &token_info.coin_id,
                &token_info.symbol,
                date
            )
            .await?;

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

        // Calculate fees
        let total_fees = if matches!(reason, RebalanceReason::Initial) {
            // Initial: all positions are BUYs
            self.calculate_initial_fees(&coins_info, &index).await?
        } else {
            // Periodic: compare with previous rebalance to detect BUY/SELL
            self.calculate_rebalance_fees(index_id, date, &coins_info, &index).await?
        };

        // Apply fees to portfolio value
        let portfolio_value_after_fees = portfolio_value_before_fees - total_fees;

        tracing::info!(
            "💰 Fees for index {} on {}: Portfolio ${} → ${} (fees: ${})",
            index_id,
            date,
            portfolio_value_before_fees,
            portfolio_value_after_fees,
            total_fees
        );

        // Save to database with AFTER-FEES value
        let coins_json = serde_json::to_value(&coins_info)?;
        let total_weight = weight * Decimal::from(coins_info.len());

        let new_rebalance = rebalances::ActiveModel {
            index_id: Set(index_id),
            coins: Set(coins_json),
            portfolio_value: Set(portfolio_value_after_fees), // ← Store after-fees value
            total_weight: Set(total_weight),
            timestamp: Set(timestamp),
            rebalance_type: Set(reason.as_str().to_string()),
            deployed: Set(Some(false)),
            ..Default::default()
        };

        new_rebalance.insert(&self.db).await?;

        tracing::info!(
            "Created {} rebalance for index {} on {} with {} tokens (portfolio value after fees: ${})",
            reason.as_str(),
            index_id,
            date,
            coins_info.len(),
            portfolio_value_after_fees
        );

        Ok(())
    }

    /// Calculate fees for initial rebalance (all positions are BUYs)
    async fn calculate_initial_fees(
        &self,
        coins_info: &[CoinRebalanceInfo],
        index: &crate::entities::index_metadata::Model,
    ) -> Result<Decimal, Box<dyn std::error::Error + Send + Sync>> {
        let trading_fee = index.exchange_trading_fees.ok_or("No trading fees configured")?;
        let spread = index.exchange_avg_spread.ok_or("No spread configured")?;

        // TODO: Investigate asymmetric fees (different rates for buy vs sell)
        // Currently using symmetric formula: fee = quantity × price × (trading_fee + spread/2)
        let fee_rate = trading_fee + (spread / Decimal::from(2));

        let mut total_fees = Decimal::ZERO;

        for coin in coins_info {
            let quantity = coin.quantity.parse::<Decimal>()?;
            let price = Decimal::from_f64_retain(coin.price).ok_or("Invalid price")?;
            let weight = coin.weight.parse::<Decimal>()?;

            // All positions are BUYs on initial rebalance
            let position_value = weight * quantity * price;
            let fee = position_value * fee_rate;

            total_fees += fee;

            tracing::debug!(
                "  BUY {} qty={} price={} value={} fee={}",
                coin.symbol,
                quantity,
                price,
                position_value,
                fee
            );
        }

        Ok(total_fees)
    }

    /// Calculate fees for periodic rebalance (compare with previous to detect BUY/SELL/HOLD)
    async fn calculate_rebalance_fees(
        &self,
        index_id: i32,
        date: NaiveDate,
        new_coins: &[CoinRebalanceInfo],
        index: &crate::entities::index_metadata::Model,
    ) -> Result<Decimal, Box<dyn std::error::Error + Send + Sync>> {
        // Get previous rebalance
        let timestamp = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        
        let previous_rebalance = Rebalances::find()
            .filter(rebalances::Column::IndexId.eq(index_id))
            .filter(rebalances::Column::Timestamp.lt(timestamp))
            .order_by(rebalances::Column::Timestamp, Order::Desc)
            .limit(1)
            .one(&self.db)
            .await?
            .ok_or("No previous rebalance found")?;

        let old_coins: Vec<CoinRebalanceInfo> = serde_json::from_value(previous_rebalance.coins)?;

        let trading_fee = index.exchange_trading_fees.ok_or("No trading fees configured")?;
        let spread = index.exchange_avg_spread.ok_or("No spread configured")?;

        // TODO: Investigate asymmetric fees (different rates for buy vs sell)
        // Currently using symmetric formula: fee = |quantity_changed| × price × (trading_fee + spread/2)
        let fee_rate = trading_fee + (spread / Decimal::from(2));

        let mut total_fees = Decimal::ZERO;

        // Build map of old positions
        let mut old_positions: std::collections::HashMap<String, Decimal> = std::collections::HashMap::new();
        for coin in old_coins {
            let quantity = coin.quantity.parse::<Decimal>().unwrap_or(Decimal::ZERO);
            old_positions.insert(coin.coin_id.clone(), quantity);
        }

        // Compare new vs old positions
        for new_coin in new_coins {
            let new_quantity = new_coin.quantity.parse::<Decimal>()?;
            let old_quantity = old_positions.get(&new_coin.coin_id).copied().unwrap_or(Decimal::ZERO);
            
            let quantity_change = new_quantity - old_quantity;

            if quantity_change == Decimal::ZERO {
                // HOLD: no change, no fees
                tracing::debug!("  HOLD {} (no change)", new_coin.symbol);
                continue;
            }

            let price = Decimal::from_f64_retain(new_coin.price).ok_or("Invalid price")?;
            let weight = new_coin.weight.parse::<Decimal>()?;

            // Calculate fee on the changed amount (absolute value)
            let change_value = weight * quantity_change.abs() * price;
            let fee = change_value * fee_rate;

            total_fees += fee;

            if quantity_change > Decimal::ZERO {
                tracing::debug!(
                    "  BUY {} qty_change={} price={} value={} fee={}",
                    new_coin.symbol,
                    quantity_change,
                    price,
                    change_value,
                    fee
                );
            } else {
                tracing::debug!(
                    "  SELL {} qty_change={} price={} value={} fee={}",
                    new_coin.symbol,
                    quantity_change.abs(),
                    price,
                    change_value,
                    fee
                );
            }
        }

        Ok(total_fees)
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
            // Use SELF-HEALING function that auto-fetches missing prices
            let current_price = crate::services::price_utils::get_or_fetch_coins_historical_price(
                &self.db,
                &self.coingecko,
                &coin.coin_id,
                &coin.symbol,
                date
            )
            .await?;

            let quantity = coin.quantity.parse::<Decimal>()?;
            let price_decimal = Decimal::from_f64_retain(current_price)
                .ok_or("Invalid price")?;
            let weight = coin.weight.parse::<Decimal>()?;

            total_value += weight * quantity * price_decimal;
        }

        Ok(total_value)
    }
}
