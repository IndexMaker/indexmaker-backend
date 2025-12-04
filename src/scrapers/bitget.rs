use chrono::NaiveDateTime;
use reqwest::Client;
use serde::Deserialize;

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
struct BitgetDetail {
    title: String,
    content: String,
    #[serde(rename = "showTime")]
    show_time: String,
}

pub struct BitgetScraper {
    client: Client,
    config: ScraperConfig,
}

impl BitgetScraper {
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

        // Scrape listings (sectionId=5955813039257)
        let (listings_ann, listings_data) = self
            .scrape_section("5955813039257", "listing", since)
            .await?;
        all_announcements.extend(listings_ann);
        all_listings.extend(listings_data);

        // Scrape delistings (sectionId=12508313443290)
        let (delistings_ann, delistings_data) = self
            .scrape_section("12508313443290", "delisting", since)
            .await?;
        all_announcements.extend(delistings_ann);
        all_listings.extend(delistings_data);

        Ok((all_announcements, all_listings))
    }

    async fn scrape_section(
        &self,
        section_id: &str,
        listing_type: &str,
        since: NaiveDateTime,
    ) -> Result<(Vec<ScrapedAnnouncement>, Vec<ScrapedListing>), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut announcements = Vec::new();
        let mut listings = Vec::new();
        let mut page_num = 1;
        let page_size = 20;
        let mut has_more = true;

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

            let response = self.fetch_with_retry(&proxy_url, &request_body).await?;
            let api_response: BitgetApiResponse = response.json().await?;

            let items = api_response.data.items;
            if items.is_empty() {
                has_more = false;
                break;
            }

            let mut found_new_items = false;

            for item in items {
                let timestamp_ms: i64 = item.show_time.parse()?;
                let item_date = NaiveDateTime::from_timestamp_millis(timestamp_ms)
                    .ok_or("Invalid timestamp")?;

                // Stop if item is older than 'since'
                if item_date <= since {
                    has_more = false;
                    tracing::info!("Reached items older than {}, stopping", since);
                    break;
                }

                found_new_items = true;

                // Fetch detail
                let detail = self.fetch_detail(&item.content_id).await?;
                let content_html = detail.content;

                // Store announcement
                let announcement = ScrapedAnnouncement {
                    title: detail.title.clone(),
                    source: "bitget".to_string(),
                    announce_date: item_date,
                    content: content_html.clone(),
                    parsed: false,
                };
                announcements.push(announcement);

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

            if !found_new_items {
                has_more = false;
            }

            page_num += 1;

            // Rate limiting
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        Ok((announcements, listings))
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