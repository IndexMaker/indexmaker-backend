use chrono::NaiveDateTime;
use reqwest::Client;
use serde::Deserialize;

use super::{ListingType, ScrapedAnnouncement, ScrapedListing, ScraperConfig};
use crate::scrapers::parser::{extract_pairs_from_html, is_valid_pair, parse_trading_pair};

#[derive(Debug, Deserialize)]
struct BinanceApiResponse {
    code: String,
    success: bool,
    data: BinanceData,
}

#[derive(Debug, Deserialize)]
struct BinanceData {
    catalogs: Vec<BinanceCatalog>,
}

#[derive(Debug, Deserialize)]
struct BinanceCatalog {
    articles: Vec<BinanceArticle>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceArticle {
    title: String,
    code: String,
    release_date: i64, // milliseconds
}

#[derive(Debug, Deserialize)]
struct BinanceArticleDetail {
    data: BinanceArticleDetailData,
}

#[derive(Debug, Deserialize)]
struct BinanceArticleDetailData {
    body: Option<String>,
}

pub struct BinanceScraper {
    client: Client,
    config: ScraperConfig,
}

impl BinanceScraper {
    pub fn new(config: ScraperConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();

        Self { client, config }
    }

    pub async fn scrape_since(
        &self,
        since: NaiveDateTime,
    ) -> Result<(Vec<ScrapedAnnouncement>, Vec<ScrapedListing>), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut all_announcements = Vec::new();
        let mut all_listings = Vec::new();

        // Scrape listings (catalogId=48)
        let (listings_ann, listings_data) = self.scrape_catalog(48, "listing", since).await?;
        all_announcements.extend(listings_ann);
        all_listings.extend(listings_data);

        // Scrape delistings (catalogId=161)
        let (delistings_ann, delistings_data) = self.scrape_catalog(161, "delisting", since).await?;
        all_announcements.extend(delistings_ann);
        all_listings.extend(delistings_data);

        Ok((all_announcements, all_listings))
    }

    async fn scrape_catalog(
        &self,
        catalog_id: u32,
        listing_type: &str,
        since: NaiveDateTime,
    ) -> Result<(Vec<ScrapedAnnouncement>, Vec<ScrapedListing>), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut announcements = Vec::new();
        let mut listings = Vec::new();
        let mut page = 1;
        let page_size = 10;
        let mut has_more = true;

        while has_more {
            tracing::info!("Scraping Binance {} catalog page {}", listing_type, page);

            let url = format!(
                "https://www.binance.com/bapi/apex/v1/public/apex/cms/article/list/query?type=1&pageNo={}&pageSize={}&catalogId={}",
                page, page_size, catalog_id
            );

            let response = self.fetch_with_retry(&url).await?;
            let api_response: BinanceApiResponse = response.json().await?;

            if api_response.code != "000000" || !api_response.success {
                tracing::error!("Binance API error: {:?}", api_response);
                break;
            }

            let catalogs = api_response.data.catalogs;
            if catalogs.is_empty() {
                break;
            }

            let articles = &catalogs[0].articles;
            if articles.is_empty() {
                break;
            }

            let mut found_new_articles = false;

            for article in articles {
                let article_date = NaiveDateTime::from_timestamp_millis(article.release_date)
                    .ok_or("Invalid timestamp")?;

                // Stop if article is older than 'since'
                if article_date <= since {
                    has_more = false;
                    tracing::info!("Reached articles older than {}, stopping", since);
                    break;
                }

                found_new_articles = true;

                // Fetch article detail
                let detail_url = format!(
                    "https://www.binance.com/bapi/apex/v1/public/cms/article/detail/query?articleCode={}",
                    article.code
                );

                let detail_response = self.fetch_with_retry(&detail_url).await?;
                let detail: BinanceArticleDetail = detail_response.json().await?;

                let content_html = detail.data.body.unwrap_or_default();

                // Store announcement
                let announcement = ScrapedAnnouncement {
                    title: article.title.clone(),
                    source: "binance".to_string(),
                    announce_date: article_date,
                    content: content_html.clone(),
                    parsed: false,
                };
                announcements.push(announcement);

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

            if !found_new_articles {
                has_more = false;
            }

            page += 1;

            // Rate limiting
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        Ok((announcements, listings))
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