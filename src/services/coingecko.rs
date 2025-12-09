use chrono::{DateTime, Utc};
use moka::future::Cache;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct CoinGeckoService {
    client: Client,
    api_key: String,
    base_url: String,
    cache: Arc<Cache<String, Vec<(i64, f64)>>>,
}

#[derive(Debug, Deserialize)]
struct MarketChartResponse {
    prices: Vec<(i64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryInfo {
    pub category_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinInCategory {
    pub id: String,
    pub symbol: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinListEntry {
    pub id: String,
    pub symbol: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinListItem {
    pub id: String,
    pub symbol: String,
    pub name: String,
    #[serde(default)]
    pub platforms: serde_json::Value, // Could be {} or {"ethereum": "0x...", ...}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCoinListItem {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub activated_at: i64, // Unix timestamp
}

impl CoinGeckoService {
    pub fn new(api_key: String, base_url: String) -> Self {
        let cache = Cache::builder()
            .max_capacity(100) // Store up to 100 different coins
            .time_to_live(Duration::from_secs(3600)) // 1 hour TTL
            .build();

        Self {
            client: Client::new(),
            api_key,
            base_url,
            cache: Arc::new(cache),
        }
    }

    pub async fn get_token_market_chart(
        &self,
        coin_id: &str,
        currency: &str,
        days: u32,
    ) -> Result<Vec<(i64, f64)>, Box<dyn std::error::Error + Send + Sync>> {
        let cache_key = format!("{}_{}_{}",coin_id, currency, days);

        // Check cache first
        if let Some(cached_data) = self.cache.get(&cache_key).await {
            tracing::debug!("Cache hit for {}", cache_key);
            return Ok(cached_data);
        }

        tracing::info!("Fetching market chart for {} from CoinGecko", coin_id);

        // Fetch from API
        let url = format!("{}/coins/{}/market_chart", self.base_url, coin_id);
        
        let response = self
            .client
            .get(&url)
            .header("accept", "application/json")
            .header("x-cg-pro-api-key", &self.api_key)
            .query(&[("vs_currency", currency), ("days", &days.to_string())])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            return Err(format!("CoinGecko API error {}: {}", status, error_text).into());
        }

        let data: MarketChartResponse = response.json().await?;

        // Store in cache
        self.cache
            .insert(cache_key, data.prices.clone())
            .await;

        if let Some(last_price) = data.prices.last() {
            let last_date = DateTime::from_timestamp_millis(last_price.0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "Invalid date".to_string());
            tracing::debug!(
                "Fetched {} prices for {}, last: {} @ {}",
                data.prices.len(),
                coin_id,
                last_price.1,
                last_date
            );
        }

        Ok(data.prices)
    }

    pub async fn fetch_categories(&self) -> Result<Vec<CategoryInfo>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Fetching categories from CoinGecko");

        let url = format!("{}/coins/categories/list", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("accept", "application/json")
            .header("x-cg-pro-api-key", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            return Err(format!("CoinGecko API error {}: {}", status, error_text).into());
        }

        let categories: Vec<CategoryInfo> = response.json().await?;

        tracing::info!("Fetched {} categories from CoinGecko", categories.len());

        Ok(categories)
    }

    /// Fetch all coins in a specific category
    pub async fn fetch_coins_by_category(
        &self,
        category_id: &str,
    ) -> Result<Vec<CoinInCategory>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Fetching coins in category '{}' from CoinGecko", category_id);

        let url = format!("{}/coins/markets", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("accept", "application/json")
            .header("x-cg-pro-api-key", &self.api_key)
            .query(&[
                ("vs_currency", "usd"),
                ("category", category_id),
                ("per_page", "250"), // Max per page
                ("page", "1"),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            return Err(format!("CoinGecko API error {}: {}", status, error_text).into());
        }

        let coins: Vec<CoinInCategory> = response.json().await?;

        tracing::info!("Fetched {} coins in category '{}'", coins.len(), category_id);

        Ok(coins)
    }

    /// Fetch ALL coins from CoinGecko (initial sync)
    pub async fn fetch_all_coins_list(
        &self,
        status: &str, // "active" or "inactive"
    ) -> Result<Vec<CoinListItem>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Fetching {} coins from CoinGecko /coins/list", status);

        let url = format!("{}/coins/list", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("accept", "application/json")
            .header("x-cg-pro-api-key", &self.api_key)
            .query(&[("status", status)])
            .send()
            .await?;

        if !response.status().is_success() {
            let status_code = response.status();
            let error_text = response.text().await?;
            return Err(format!("CoinGecko API error {}: {}", status_code, error_text).into());
        }

        let coins: Vec<CoinListItem> = response.json().await?;

        tracing::info!("Fetched {} {} coins from CoinGecko", coins.len(), status);

        Ok(coins)
    }

    /// Fetch only NEW coins from CoinGecko (incremental sync)
    pub async fn fetch_new_coins_list(
        &self,
    ) -> Result<Vec<NewCoinListItem>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Fetching NEW coins from CoinGecko /coins/list/new");

        let url = format!("{}/coins/list/new", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("accept", "application/json")
            .header("x-cg-pro-api-key", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            return Err(format!("CoinGecko API error {}: {}", status, error_text).into());
        }

        let new_coins: Vec<NewCoinListItem> = response.json().await?;

        tracing::info!("Fetched {} new coins from CoinGecko", new_coins.len());

        Ok(new_coins)
    }
}
