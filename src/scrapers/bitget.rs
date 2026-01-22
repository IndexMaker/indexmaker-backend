use chrono::NaiveDateTime;
use reqwest::Client;
use serde::Deserialize;
use sea_orm::DatabaseConnection;

use super::{ListingType, ScrapedAnnouncement, ScrapedListing, ScraperConfig};
use crate::scrapers::parser::{extract_pairs_from_html, is_valid_pair, parse_trading_pair};

#[derive(Debug, Deserialize)]
struct BitgetApiResponse {
    data: BitgetData,
}

#[derive(Debug, Deserialize)]
struct BitgetData {
    items: Vec<BitgetItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct BitgetItem {
    content_id: String,
    title: String,
    show_time: String, // timestamp as string
}

#[derive(Debug, Deserialize)]
struct BitgetDetailResponse {
    data: BitgetDetail,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BitgetDetail {
    title: String,
    content: String,
    #[serde(rename = "showTime")]
    show_time: String,
}

pub struct BitgetScraper {
    client: Client,
    config: ScraperConfig,
    db: DatabaseConnection,
}

impl BitgetScraper {
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
    ) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut total_announcements = 0;
        let mut total_listings = 0;

        // Scrape listings (sectionId=5955813039257)
        let (ann_count, list_count) = self
            .scrape_section("5955813039257", "listing", since)
            .await?;
        total_announcements += ann_count;
        total_listings += list_count;

        // Scrape delistings (sectionId=12508313443290)
        let (ann_count, list_count) = self
            .scrape_section("12508313443290", "delisting", since)
            .await?;
        total_announcements += ann_count;
        total_listings += list_count;

        Ok((total_announcements, total_listings))
    }

    async fn scrape_section(
        &self,
        section_id: &str,
        listing_type: &str,
        since: NaiveDateTime,
    ) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut total_announcements = 0;
        let mut total_listings = 0;
        let mut page_num = 1;
        let page_size = 20;
        let mut has_more = true;
        let mut consecutive_failures = 0;
        let max_consecutive_failures = 5; // Stop after 5 consecutive failures

        while has_more {
            tracing::info!("Scraping Bitget {} section page {}", listing_type, page_num);

            let target_url =
                "https://www.bitget.com/v1/cms/helpCenter/content/section/helpContentDetail";
            let proxy_url = format!(
                "https://api.scrape.do/?token={}&url={}",
                self.config.scrape_api_key, target_url
            );

            let request_body = serde_json::json!({
                "pageNum": page_num,
                "pageSize": page_size,
                "params": {
                    "sectionId": section_id,
                    "languageId": 0,
                    "firstSearchTime": chrono::Utc::now().timestamp_millis(),
                }
            });

            // Scrape this page
            let (page_announcements, page_listings, should_continue) = match self
                .scrape_single_page(&proxy_url, &request_body, listing_type, since)
                .await
            {
                Ok(result) => {
                    consecutive_failures = 0; // Reset on success
                    result
                }
                Err(e) => {
                    consecutive_failures += 1;
                    tracing::error!(
                        "Failed to scrape page {}: {}. Consecutive failures: {}/{}",
                        page_num, e, consecutive_failures, max_consecutive_failures
                    );
                    
                    if consecutive_failures >= max_consecutive_failures {
                        tracing::error!(
                            "Stopping {} section scrape after {} consecutive failures",
                            listing_type, consecutive_failures
                        );
                        break; // Exit the loop
                    }
                    
                    page_num += 1;
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    continue; // Skip this page, continue with next
                }
            };

            // Save immediately after each page! ✅
            if !page_announcements.is_empty() || !page_listings.is_empty() {
                match save_scraped_data(&self.db, page_announcements.clone(), page_listings.clone()).await {
                    Ok(_) => {
                        total_announcements += page_announcements.len();
                        total_listings += page_listings.len();
                        tracing::info!(
                            "✅ Saved page {}: {} announcements, {} listings",
                            page_num,
                            page_announcements.len(),
                            page_listings.len()
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to save page {} data: {}. Data lost for this page.", page_num, e);
                        // Continue anyway - don't fail entire scrape
                    }
                }
            }

            if !should_continue {
                has_more = false;
            }

            page_num += 1;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        Ok((total_announcements, total_listings))
    }

    /// Scrape a single page and return data + whether to continue
    async fn scrape_single_page(
        &self,
        proxy_url: &str,
        request_body: &serde_json::Value,
        listing_type: &str,
        since: NaiveDateTime,
    ) -> Result<(Vec<ScrapedAnnouncement>, Vec<ScrapedListing>, bool), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut announcements = Vec::new();
        let mut listings = Vec::new();
        let mut should_continue = true;

        let response = self.fetch_with_retry(proxy_url, request_body).await?;
        let api_response: BitgetApiResponse = response.json().await?;

        let items = api_response.data.items;
        if items.is_empty() {
            return Ok((announcements, listings, false));
        }

        for item in items {
            let timestamp_ms: i64 = item.show_time.parse()?;
            let item_date = chrono::DateTime::from_timestamp_millis(timestamp_ms)
                .ok_or("Invalid timestamp")?
                .naive_utc();

            // Stop if item is older than 'since'
            if item_date <= since {
                should_continue = false;
                tracing::info!("Reached items older than {}, stopping", since);
                break;
            }

            // Fetch detail
            let detail = match self.fetch_detail(&item.content_id).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("Failed to fetch detail for content_id {}: {}", item.content_id, e);
                    continue; // Skip this item, continue with next
                }
            };

            let content_html = detail.content;

            // Store announcement
            announcements.push(ScrapedAnnouncement {
                title: detail.title.clone(),
                source: "bitget".to_string(),
                announce_date: item_date,
                content: content_html.clone(),
                parsed: false,
            });

            // Parse pairs
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
                        announcement_date: item_date,
                        listing_date: if listing_type == "listing" {
                            Some(item_date)
                        } else {
                            None
                        },
                        delisting_date: if listing_type == "delisting" {
                            Some(item_date)
                        } else {
                            None
                        },
                        source: "bitget".to_string(),
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

    async fn fetch_detail(&self, content_id: &str) -> Result<BitgetDetail, Box<dyn std::error::Error + Send + Sync>> {
        let target_url = "https://www.bitget.com/v1/cms/helpCenter/content/get/helpContentDetail";
        let proxy_url = format!(
            "https://api.scrape.do/?token={}&url={}",
            self.config.scrape_api_key, target_url
        );

        let request_body = serde_json::json!({
            "contentId": content_id,
            "languageId": 0,
        });

        let response = self.fetch_with_retry(&proxy_url, &request_body).await?;
        let detail_response: BitgetDetailResponse = response.json().await?;

        Ok(detail_response.data)
    }

    async fn fetch_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
        let mut delay = tokio::time::Duration::from_millis(self.config.retry_delay_ms);

        for attempt in 0..self.config.retry_max {
            match self.client.post(url).json(body).send().await {
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

            tracing::warn!("Retry {}/{} for Bitget API. Waiting {:?}", attempt + 1, self.config.retry_max, delay);
            tokio::time::sleep(delay).await;
            delay *= 2;
        }

        Err("Max retries exceeded".into())
    }
}

// Helper function to save data (moved from announcement_scraper.rs or made public there)
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use crate::entities::{announcements, crypto_listings, prelude::*};

async fn save_scraped_data(
    db: &DatabaseConnection,
    announcements_list: Vec<ScrapedAnnouncement>,
    listings: Vec<ScrapedListing>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Save announcements
    for announcement in announcements_list {
        // Check if exists
        let existing = Announcements::find()
            .filter(announcements::Column::Title.eq(&announcement.title))
            .filter(announcements::Column::Source.eq(&announcement.source))
            .filter(announcements::Column::AnnounceDate.eq(announcement.announce_date))
            .one(db)
            .await?;

        if existing.is_none() {
            let new_announcement = announcements::ActiveModel {
                title: Set(announcement.title),
                source: Set(announcement.source),
                announce_date: Set(announcement.announce_date),
                content: Set(announcement.content),
                parsed: Set(Some(announcement.parsed)),
                ..Default::default()
            };

            new_announcement.insert(db).await?;
        }
    }

    // Save listings to crypto_listings
    for listing in listings {
        // Check if exists
        let existing = CryptoListings::find()
            .filter(crypto_listings::Column::CoinId.eq(&listing.token.to_lowercase()))
            .filter(crypto_listings::Column::Exchange.eq(&listing.source))
            .filter(crypto_listings::Column::TradingPair.eq(&listing.trading_pair))
            .one(db)
            .await?;

        if let Some(existing_listing) = existing {
            // Update existing
            let mut active: crypto_listings::ActiveModel = existing_listing.into();

            if listing.listing_date.is_some() {
                active.listing_announcement_date = Set(Some(listing.announcement_date));
                active.listing_date = Set(listing.listing_date);
            }

            if listing.delisting_date.is_some() {
                active.delisting_announcement_date = Set(Some(listing.announcement_date));
                active.delisting_date = Set(listing.delisting_date);
                active.status = Set("delisted".to_string());
            }

            active.updated_at = Set(Some(Utc::now().naive_utc()));
            active.update(db).await?;
        } else {
            // Insert new
            let new_listing = crypto_listings::ActiveModel {
                coin_id: Set(listing.token.to_lowercase()),
                symbol: Set(listing.symbol),
                token_name: Set(listing.token_name),
                exchange: Set(listing.source),
                trading_pair: Set(listing.trading_pair),
                listing_announcement_date: Set(if listing.listing_date.is_some() {
                    Some(listing.announcement_date)
                } else {
                    None
                }),
                listing_date: Set(listing.listing_date),
                delisting_announcement_date: Set(if listing.delisting_date.is_some() {
                    Some(listing.announcement_date)
                } else {
                    None
                }),
                delisting_date: Set(listing.delisting_date),
                status: Set(if listing.delisting_date.is_some() {
                    "delisted".to_string()
                } else {
                    "active".to_string()
                }),
                ..Default::default()
            };

            new_listing.insert(db).await?;
        }
    }

    Ok(())
}