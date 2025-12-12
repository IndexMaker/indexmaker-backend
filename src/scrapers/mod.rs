pub mod binance;
pub mod bitget;
pub mod parser;
pub mod coin_resolver;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapedListing {
    pub token: String,
    pub token_name: String,
    pub symbol: String,
    pub trading_pair: String,
    pub announcement_date: NaiveDateTime,
    pub listing_date: Option<NaiveDateTime>,
    pub delisting_date: Option<NaiveDateTime>,
    pub source: String,
    pub listing_type: ListingType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ListingType {
    Listing,
    Delisting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapedAnnouncement {
    pub title: String,
    pub source: String,
    pub announce_date: NaiveDateTime,
    pub content: String,
    pub parsed: bool,
}

#[derive(Clone)]
pub struct ScraperConfig {
    pub scrape_api_key: String,
    pub retry_max: u32,
    pub retry_delay_ms: u64,
}

impl Default for ScraperConfig {
    fn default() -> Self {
        Self {
            scrape_api_key: String::new(),
            retry_max: 3,
            retry_delay_ms: 1000,
        }
    }
}