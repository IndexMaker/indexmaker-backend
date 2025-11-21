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
}
