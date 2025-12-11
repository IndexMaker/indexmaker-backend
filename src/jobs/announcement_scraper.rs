use chrono::{NaiveDateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, Set};
use tokio::time::{interval, Duration};

use crate::entities::{announcements, crypto_listings, prelude::*};
use crate::scrapers::binance::BinanceScraper;
use crate::scrapers::bitget::BitgetScraper;
use crate::scrapers::{ScrapedAnnouncement, ScrapedListing, ScraperConfig};

pub async fn start_announcement_scraper_job(
    db: DatabaseConnection,
    scraper_config: ScraperConfig,
) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(86400)); // Every day

        // Run immediately on startup
        tracing::info!("Running initial announcement scraper");
        if let Err(e) = scrape_all_exchanges(&db, &scraper_config).await {
            tracing::error!("Failed to run initial scrape: {}", e);
        }

        loop {
            interval.tick().await;
            tracing::info!("Starting scheduled announcement scraper");

            if let Err(e) = scrape_all_exchanges(&db, &scraper_config).await {
                tracing::error!("Failed to scrape announcements: {}", e);
            }
        }
    });
}

async fn scrape_all_exchanges(
    db: &DatabaseConnection,
    config: &ScraperConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get start dates using max(timestamp) logic
    let start_dates = get_scrape_start_dates(db).await?;

    tracing::info!(
        "Scraping since: Binance={}, Bitget={}",
        start_dates.binance,
        start_dates.bitget
    );

    // Scrape Binance
    let binance_scraper = BinanceScraper::new(config.clone(), db.clone());
    match binance_scraper.scrape_since(start_dates.binance).await {
        Ok((ann_count, list_count)) => {
            tracing::info!(
                "Binance complete: {} announcements, {} listings saved",
                ann_count,
                list_count
            );
        }
        Err(e) => {
            tracing::error!("Binance scraper error: {}", e);
        }
    }

    // // Scrape Bitget
    // let bitget_scraper = BitgetScraper::new(config.clone(), db.clone());
    // match bitget_scraper.scrape_since(start_dates.bitget).await {
    //     Ok((ann_count, list_count)) => {
    //         tracing::info!(
    //             "Bitget complete: {} announcements, {} listings saved",
    //             ann_count,
    //             list_count
    //         );
    //     }
    //     Err(e) => {
    //         tracing::error!("Bitget scraper error: {}", e);
    //     }
    // }

    Ok(())
}

struct ScraperStartDates {
    binance: NaiveDateTime,
    bitget: NaiveDateTime,
}

/// Get max(timestamp) from crypto_listings and announcements tables
async fn get_scrape_start_dates(
    db: &DatabaseConnection,
) -> Result<ScraperStartDates, Box<dyn std::error::Error + Send + Sync>> {
    // Binance
    let binance_listing_date = get_latest_crypto_listing_date(db, "binance").await?;
    let binance_announcement_date = get_latest_announcement_date(db, "binance").await?;
    let binance_start = std::cmp::max(binance_listing_date, binance_announcement_date);

    // Bitget
    let bitget_listing_date = get_latest_crypto_listing_date(db, "bitget").await?;
    let bitget_announcement_date = get_latest_announcement_date(db, "bitget").await?;
    let bitget_start = std::cmp::max(bitget_listing_date, bitget_announcement_date);

    // Log if there's a mismatch
    if binance_listing_date != binance_announcement_date {
        tracing::warn!(
            "Binance date mismatch: listings={}, announcements={}. Using {}",
            binance_listing_date,
            binance_announcement_date,
            binance_start
        );
    }

    if bitget_listing_date != bitget_announcement_date {
        tracing::warn!(
            "Bitget date mismatch: listings={}, announcements={}. Using {}",
            bitget_listing_date,
            bitget_announcement_date,
            bitget_start
        );
    }

    Ok(ScraperStartDates {
        binance: binance_start,
        bitget: bitget_start,
    })
}

async fn get_latest_crypto_listing_date(
    db: &DatabaseConnection,
    exchange: &str,
) -> Result<NaiveDateTime, Box<dyn std::error::Error + Send + Sync>> {
    let latest = CryptoListings::find()
        .filter(crypto_listings::Column::Exchange.eq(exchange))
        .filter(crypto_listings::Column::ListingDate.is_not_null())
        .order_by(crypto_listings::Column::ListingDate, Order::Desc)
        .limit(1)
        .one(db)
        .await?;

    Ok(latest
        .and_then(|l| l.listing_date)
        .unwrap_or_else(|| NaiveDateTime::from_timestamp_opt(0, 0).unwrap()))
}

async fn get_latest_announcement_date(
    db: &DatabaseConnection,
    source: &str,
) -> Result<NaiveDateTime, Box<dyn std::error::Error + Send + Sync>> {
    let latest = Announcements::find()
        .filter(announcements::Column::Source.eq(source))
        .order_by(announcements::Column::AnnounceDate, Order::Desc)
        .limit(1)
        .one(db)
        .await?;

    Ok(latest
        .map(|a| a.announce_date)
        .unwrap_or_else(|| NaiveDateTime::from_timestamp_opt(0, 0).unwrap()))
}

pub async fn save_scraped_data(
    db: &DatabaseConnection,
    announcements: Vec<ScrapedAnnouncement>,
    listings: Vec<ScrapedListing>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // for announcement in &announcements {
    //     println!("title: {:?}", announcement.title);
    // }
    // println!("listings: {:?}", listings);
    // return Ok(());

    // Save announcements
    for announcement in announcements {
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