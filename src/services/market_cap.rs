use chrono::NaiveDate;
use moka::future::Cache;
use reqwest::Client;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use crate::entities::{market_cap_rankings, prelude::*};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinMarketData {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub current_price: f64,
    pub market_cap: f64,
    pub market_cap_rank: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct MarketCapRanking {
    pub coin_id: String,
    pub symbol: String,
    pub market_cap: f64,
    pub price: f64,
    pub rank: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct HistoricalResponse {
    market_data: HistoricalMarketData,
}

#[derive(Debug, Deserialize)]
struct HistoricalMarketData {
    market_cap: CurrencyValue,
    current_price: CurrencyValue,
}

#[derive(Debug, Deserialize)]
struct CurrencyValue {
    usd: f64,
}

pub struct MarketCapService {
    client: Client,
    api_key: String,
    base_url: String,
    cache: Arc<Cache<String, Vec<MarketCapRanking>>>,
}

impl MarketCapService {
    pub fn new(api_key: String, base_url: String) -> Self {
        let cache = Cache::builder()
            .max_capacity(1000)
            .time_to_live(Duration::from_secs(86400)) // 24 hours
            .build();

        Self {
            client: Client::new(),
            api_key,
            base_url,
            cache: Arc::new(cache),
        }
    }

    /// Get top N tokens by market cap for a specific date
    pub async fn get_top_tokens_by_market_cap(
        &self,
        db: &DatabaseConnection,
        date: NaiveDate,
        top_n: usize,
    ) -> Result<Vec<MarketCapRanking>, Box<dyn std::error::Error + Send + Sync>> {
        let cache_key = format!("top_{}_date_{}", top_n, date);

        // Check cache first
        if let Some(cached) = self.cache.get(&cache_key).await {
            tracing::debug!("Cache hit for top {} on {}", top_n, date);
            return Ok(cached);
        }

        tracing::info!("Fetching top {} market cap rankings for {}", top_n, date);

        // Check database cache
        let db_rankings = self.get_rankings_from_db(db, date, top_n).await?;
        
        if db_rankings.len() >= top_n {
            tracing::info!("Found {} rankings in database for {}", db_rankings.len(), date);
            self.cache.insert(cache_key, db_rankings.clone()).await;
            return Ok(db_rankings);
        }

        // Not in cache, need to fetch from CoinGecko
        tracing::info!("No cache found, fetching from CoinGecko for {}", date);
        
        // Get current top coins as candidates
        let current_top = self.fetch_current_top_coins(top_n * 2).await?;
        
        // Fetch historical market cap for each
        let historical_rankings = self
            .fetch_historical_market_caps_batch(current_top, date)
            .await?;

        // Sort by market cap
        let mut sorted = historical_rankings;
        sorted.sort_by(|a, b| b.market_cap.total_cmp(&a.market_cap));

        // Assign ranks
        for (i, ranking) in sorted.iter_mut().enumerate() {
            ranking.rank = Some((i + 1) as i32);
        }

        // Store in database (fire and forget - don't block)
        let db_clone = db.clone();
        let date_clone = date;
        let sorted_clone = sorted.clone();
        tokio::spawn(async move {
            if let Err(e) = store_rankings_in_db_internal(&db_clone, date_clone, &sorted_clone).await {
                tracing::error!("Failed to store rankings in database: {}", e);
            }
        });
        
        // Cache in memory
        self.cache.insert(cache_key, sorted.clone()).await;

        Ok(sorted)
    }

    /// Fetch current top N coins from CoinGecko
    async fn fetch_current_top_coins(
        &self,
        top_n: usize,
    ) -> Result<Vec<CoinMarketData>, Box<dyn std::error::Error + Send + Sync>> {
        let per_page = 250;
        let pages_needed = (top_n + per_page - 1) / per_page;

        let mut all_coins = Vec::new();

        for page in 1..=pages_needed {
            let url = format!("{}/coins/markets", self.base_url);

            let response = self
                .client
                .get(&url)
                .header("x-cg-pro-api-key", &self.api_key)
                .query(&[
                    ("vs_currency", "usd"),
                    ("order", "market_cap_desc"),
                    ("per_page", &per_page.to_string()),
                    ("page", &page.to_string()),
                ])
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await?;
                return Err(format!("CoinGecko API error {}: {}", status, error_text).into());
            }

            let coins: Vec<CoinMarketData> = response.json().await?;
            all_coins.extend(coins);

            // Rate limiting
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        all_coins.truncate(top_n);
        Ok(all_coins)
    }

    /// Fetch historical market caps for multiple coins
    async fn fetch_historical_market_caps_batch(
        &self,
        coins: Vec<CoinMarketData>,
        date: NaiveDate,
    ) -> Result<Vec<MarketCapRanking>, Box<dyn std::error::Error + Send + Sync>> {
        let date_str = date.format("%d-%m-%Y").to_string();
        let mut results = Vec::new();

        for coin in coins {
            match self
                .fetch_single_historical_market_cap(&coin.id, &coin.symbol, &date_str)
                .await
            {
                Ok(ranking) => results.push(ranking),
                Err(e) => {
                    tracing::warn!(
                        "Failed to fetch historical market cap for {}: {}",
                        coin.id,
                        e
                    );
                    // Continue with others
                }
            }

            // Rate limiting: 100ms between calls
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        Ok(results)
    }

    /// Fetch single coin historical market cap
    async fn fetch_single_historical_market_cap(
        &self,
        coin_id: &str,
        symbol: &str,
        date: &str, // dd-mm-yyyy
    ) -> Result<MarketCapRanking, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/coins/{}/history", self.base_url, coin_id);

        let response = self
            .client
            .get(&url)
            .header("x-cg-pro-api-key", &self.api_key)
            .query(&[("date", date), ("localization", "false")])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            return Err(format!("CoinGecko API error {}: {}", status, error_text).into());
        }

        let data: HistoricalResponse = response.json().await?;

        Ok(MarketCapRanking {
            coin_id: coin_id.to_string(),
            symbol: symbol.to_uppercase(),
            market_cap: data.market_data.market_cap.usd,
            price: data.market_data.current_price.usd,
            rank: None, // Will be assigned later
        })
    }

    /// Get rankings from database
    async fn get_rankings_from_db(
        &self,
        db: &DatabaseConnection,
        date: NaiveDate,
        limit: usize,
    ) -> Result<Vec<MarketCapRanking>, Box<dyn std::error::Error + Send + Sync>> {
        use crate::entities::{market_cap_rankings, prelude::*};
        use sea_orm::QueryOrder;
    
        // Query market_cap_rankings table for the specific date
        let results = MarketCapRankings::find()
            .filter(market_cap_rankings::Column::Date.eq(date))
            .order_by(market_cap_rankings::Column::Rank, sea_orm::Order::Asc)
            .limit(limit as u64)
            .all(db)
            .await?;
    
        // Convert database models to MarketCapRanking
        let rankings: Vec<MarketCapRanking> = results
            .into_iter()
            .map(|row| MarketCapRanking {
                coin_id: row.coin_id,
                symbol: row.symbol,
                market_cap: row.market_cap.to_string().parse().unwrap_or(0.0),
                price: row.price.to_string().parse().unwrap_or(0.0),
                rank: row.rank,
            })
            .collect();
        
        Ok(rankings)
    }

    /// Store rankings in database
    pub async fn store_rankings_in_db(
        &self,
        db: &DatabaseConnection,
        date: NaiveDate,
        rankings: &[MarketCapRanking],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for ranking in rankings {
            let model = market_cap_rankings::ActiveModel {
                date: Set(date),
                coin_id: Set(ranking.coin_id.clone()),
                symbol: Set(ranking.symbol.clone()),
                market_cap: Set(Decimal::from_f64_retain(ranking.market_cap)
                    .ok_or("Invalid market cap")?),
                price: Set(Decimal::from_f64_retain(ranking.price)
                    .ok_or("Invalid price")?),
                rank: Set(ranking.rank),
                ..Default::default()
            };

            // Insert or ignore if exists
            let _ = model.insert(db).await;
        }

        Ok(())
    }

    /// Get market caps for specific coins on a date (for category-based selection)
    pub async fn get_market_caps_for_coins(
        &self,
        db: &DatabaseConnection,  // ADD THIS PARAMETER
        coin_ids: Vec<String>,
        date: NaiveDate,
    ) -> Result<Vec<MarketCapRanking>, Box<dyn std::error::Error + Send + Sync>> {
        use crate::entities::{market_cap_rankings, prelude::*};
    
        // First, try to get from database
        let db_results = MarketCapRankings::find()
            .filter(market_cap_rankings::Column::Date.eq(date))
            .filter(market_cap_rankings::Column::CoinId.is_in(coin_ids.clone()))
            .all(db)
            .await?;
    
        // Convert to MarketCapRanking
        let mut rankings: Vec<MarketCapRanking> = db_results
            .into_iter()
            .map(|row| MarketCapRanking {
                coin_id: row.coin_id,
                symbol: row.symbol,
                market_cap: row.market_cap.to_string().parse().unwrap_or(0.0),
                price: row.price.to_string().parse().unwrap_or(0.0),
                rank: row.rank,
            })
            .collect();
        
        // Find missing coin_ids
        let found_ids: Vec<String> = rankings.iter().map(|r| r.coin_id.clone()).collect();
        let missing_ids: Vec<String> = coin_ids
            .into_iter()
            .filter(|id| !found_ids.contains(id))
            .collect();
        
        // If we have missing coins, fetch from CoinGecko
        if !missing_ids.is_empty() {
            tracing::info!(
                "Fetching {} missing coins from CoinGecko for {}",
                missing_ids.len(),
                date
            );
        
            let date_str = date.format("%d-%m-%Y").to_string();
            
            for coin_id in missing_ids {
                // Use coin_id as symbol fallback
                let symbol = coin_id.to_uppercase();
                
                match self
                    .fetch_single_historical_market_cap(&coin_id, &symbol, &date_str)
                    .await
                {
                    Ok(ranking) => {
                        // Store in database
                        let db_clone = db.clone();
                        let date_clone = date;
                        let ranking_clone = ranking.clone();
                        tokio::spawn(async move {
                            if let Err(e) = store_rankings_in_db_internal(
                                &db_clone, 
                                date_clone, 
                                &[ranking_clone]
                            ).await {
                                tracing::error!("Failed to store ranking: {}", e);
                            }
                        });
                        
                        rankings.push(ranking);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to fetch market cap for {}: {}",
                            coin_id,
                            e
                        );
                    }
                }
            
                // Rate limiting
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
    
        Ok(rankings)
    }
}

/// Internal helper to store rankings in database
async fn store_rankings_in_db_internal(
    db: &DatabaseConnection,
    date: NaiveDate,
    rankings: &[MarketCapRanking],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::entities::{market_cap_rankings, prelude::*};

    for ranking in rankings {
        // Check if already exists
        let exists = MarketCapRankings::find()
            .filter(market_cap_rankings::Column::Date.eq(date))
            .filter(market_cap_rankings::Column::CoinId.eq(&ranking.coin_id))
            .one(db)
            .await?;

        if exists.is_some() {
            tracing::debug!("Ranking for {} on {} already exists, skipping", ranking.coin_id, date);
            continue;
        }

        let model = market_cap_rankings::ActiveModel {
            date: Set(date),
            coin_id: Set(ranking.coin_id.clone()),
            symbol: Set(ranking.symbol.clone()),
            market_cap: Set(Decimal::from_f64_retain(ranking.market_cap)
                .ok_or("Invalid market cap")?),
            price: Set(Decimal::from_f64_retain(ranking.price)
                .ok_or("Invalid price")?),
            rank: Set(ranking.rank),
            ..Default::default()
        };

        model.insert(db).await?;
    }

    Ok(())
}
