use chrono::NaiveDateTime;
use reqwest::Client;
use sea_orm::DatabaseConnection;
use serde::Deserialize;

use super::{ListingType, ScrapedAnnouncement, ScrapedListing, ScraperConfig};
use crate::{jobs::announcement_scraper::save_scraped_data, scrapers::parser::{extract_pairs_from_html, is_valid_pair, parse_trading_pair}};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BinanceApiResponse {
    code: String,
    message: Option<String>,
    #[serde(rename = "messageDetail")]
    message_detail: Option<String>,
    data: BinanceData,
    success: bool,
}

#[derive(Debug, Deserialize)]
struct BinanceData {
    catalogs: Vec<BinanceCatalog>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BinanceCatalog {
    #[serde(rename = "catalogId")]
    catalog_id: i32,

    #[serde(rename = "parentCatalogId")]
    parent_catalog_id: Option<i32>,

    icon: Option<String>,

    #[serde(rename = "catalogName")]
    catalog_name: String,

    description: Option<String>,

    #[serde(rename = "catalogType")]
    catalog_type: Option<i32>,

    total: i32,
    articles: Vec<BinanceArticle>,
    catalogs: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct BinanceArticle {
    id: i64,
    code: String,
    title: String,
    #[serde(rename = "type")]
    article_type: i32,
    release_date: i64, // milliseconds
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BinanceArticleDetail {
    code: String,
    success: bool,
    data: BinanceArticleDetailData,
}

#[derive(Debug, Deserialize)]
struct BinanceArticleDetailData {
    body: Option<String>,
}

pub struct BinanceScraper {
    client: Client,
    config: ScraperConfig,
    db: DatabaseConnection,
}

impl BinanceScraper {
    pub fn new(config: ScraperConfig, db: DatabaseConnection) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .gzip(true)
            .deflate(true)
            .brotli(true)
            .build()
            .unwrap();

        Self { client, config, db }
    }

    pub async fn scrape_since(
        &self,
        since: NaiveDateTime,
    ) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>>  // ← Return counts instead of data
    {
        let mut total_announcements = 0;
        let mut total_listings = 0;

        // Scrape listings (catalogId=48)
        let (ann_count, list_count) = self.scrape_catalog(48, "listing", since).await?;
        total_announcements += ann_count;
        total_listings += list_count;

        // Scrape delistings (catalogId=161)
        let (ann_count, list_count) = self.scrape_catalog(161, "delisting", since).await?;
        total_announcements += ann_count;
        total_listings += list_count;

        Ok((total_announcements, total_listings))
    }

    async fn scrape_catalog(
        &self,
        catalog_id: u32,
        listing_type: &str,
        since: NaiveDateTime,
    ) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut total_announcements = 0;
        let mut total_listings = 0;
        let mut page = 1;
        let page_size = 10;
        let mut has_more = true;

        while has_more {
            tracing::info!("Scraping Binance {} catalog page {}", listing_type, page);

            let url = format!(
                "https://www.binance.com/bapi/apex/v1/public/apex/cms/article/list/query?type=1&pageNo={}&pageSize={}&catalogId={}",
                page, page_size, catalog_id
            );

            // Scrape this page
            let (page_announcements, page_listings, should_continue) = match self
                .scrape_single_page(&url, listing_type, since)
                .await
            {
                Ok(result) => result,
                Err(e) => {
                    tracing::error!("Failed to scrape page {}: {}. Continuing with next page.", page, e);
                    page += 1;
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    continue; // Skip this page, continue with next
                }
            };

            // Save immediately after each page!
            if !page_announcements.is_empty() || !page_listings.is_empty() {
                match save_scraped_data(&self.db, page_announcements.clone(), page_listings.clone()).await {
                    Ok(_) => {
                        total_announcements += page_announcements.len();
                        total_listings += page_listings.len();
                        tracing::info!(
                            "Saved page {}: {} announcements, {} listings",
                            page,
                            page_announcements.len(),
                            page_listings.len()
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to save page {} data: {}. Data lost for this page.", page, e);
                        // Continue anyway - don't fail entire scrape
                    }
                }
            }

            if !should_continue {
                has_more = false;
            }

            page += 1;
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        }

        Ok((total_announcements, total_listings))
    }

    /// Scrape a single page and return data + whether to continue
    async fn scrape_single_page(
        &self,
        url: &str,
        listing_type: &str,
        since: NaiveDateTime,
    ) -> Result<(Vec<ScrapedAnnouncement>, Vec<ScrapedListing>, bool), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut announcements = Vec::new();
        let mut listings = Vec::new();
        let mut should_continue = true;

        let response = self.fetch_with_retry(url).await?;
        let api_response: BinanceApiResponse = response.json().await?;

        if api_response.code != "000000" || !api_response.success {
            tracing::error!("Binance API error: {:?}", api_response);
            return Ok((announcements, listings, false));
        }

        let catalogs = api_response.data.catalogs;
        if catalogs.is_empty() {
            return Ok((announcements, listings, false));
        }

        let articles = &catalogs[0].articles;
        if articles.is_empty() {
            return Ok((announcements, listings, false));
        }

        for article in articles {
            let article_date = chrono::DateTime::from_timestamp_millis(article.release_date)
                .ok_or("Invalid timestamp")?
                .naive_utc();

            // Stop if article is older than 'since'
            if article_date <= since {
                should_continue = false;
                tracing::info!("Reached articles older than {}, stopping", since);
                break;
            }

            // Fetch article detail
            let detail_url = format!(
                "https://www.binance.com/bapi/apex/v1/public/cms/article/detail/query?articleCode={}",
                article.code
            );

            let detail_response = match self.fetch_with_retry(&detail_url).await {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::error!("Failed to fetch detail for article {}: {}", article.code, e);
                    continue; // Skip this article, continue with next
                }
            };

            let detail_text = detail_response.text().await?;
            let detail: BinanceArticleDetail = serde_json::from_str(&detail_text)?;
            let content_html = detail.data.body.unwrap_or_default();

            // Store announcement
            announcements.push(ScrapedAnnouncement {
                title: article.title.clone(),
                source: "binance".to_string(),
                announce_date: article_date,
                content: content_html.clone(),
                parsed: false,
            });

            // Parse pairs from content
            let pairs = extract_pairs_from_html(&content_html);

            for pair in pairs {
                if !is_valid_pair(&pair) {
                    continue;
                }

                if let Some((token, trading_pair)) = parse_trading_pair(&pair) {
                    listings.push(ScrapedListing {
                        token: pair.clone(),
                        token_name: token.clone(),
                        symbol: token,
                        trading_pair,
                        announcement_date: article_date,
                        listing_date: if listing_type == "listing" {
                            Some(article_date)
                        } else {
                            None
                        },
                        delisting_date: if listing_type == "delisting" {
                            Some(article_date)
                        } else {
                            None
                        },
                        source: "binance".to_string(),
                        listing_type: if listing_type == "listing" {
                            ListingType::Listing
                        } else {
                            ListingType::Delisting
                        },
                    });
                }
            }
        }

        Ok((announcements, listings, should_continue))
    }

    async fn fetch_with_retry(
        &self,
        url: &str,
    ) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
        let mut delay = tokio::time::Duration::from_millis(self.config.retry_delay_ms);

        for attempt in 0..self.config.retry_max {
            match self.client.get(url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        return Ok(response);
                    }

                    if attempt == self.config.retry_max - 1 {
                        return Err(format!("HTTP error: {}", response.status()).into());
                    }
                }
                Err(e) => {
                    if attempt == self.config.retry_max - 1 {
                        return Err(e.into());
                    }
                }
            }

            tracing::warn!("Retry {}/{} for {}. Waiting {:?}", attempt + 1, self.config.retry_max, url, delay);
            tokio::time::sleep(delay).await;
            delay *= 2; // Exponential backoff
        }

        Err("Max retries exceeded".into())
    }
}