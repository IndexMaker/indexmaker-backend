// src/bin/import_announcements_listings.rs

use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set, Database};
use regex::Regex;
use chrono::NaiveDateTime;


use indexmaker_backend::entities::{announcements, crypto_listings, token_metadata, prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Usage: cargo run --bin import_announcements_listings -- dump.sql
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <dump.sql>", args[0]);
        std::process::exit(1);
    }

    let file_path = &args[1];
    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    
    let db = Database::connect(&database_url).await?;

    println!(" Parsing {}...", file_path);
    let (announcements_data, listings_data) = parse_dump_file(file_path)?;
    
    println!(" Found {} announcements", announcements_data.len());
    println!(" Found {} crypto listings", listings_data.len());
    
    // Import announcements first
    println!("\n Importing announcements...");
    let announcements_result = import_announcements(&db, announcements_data).await?;
    
    // Import crypto listings
    println!("\n Importing crypto listings...");
    let listings_result = import_crypto_listings(&db, listings_data).await?;
    
    // Print summary
    println!("\n Import complete!");
    println!("\n Announcements:");
    println!("    Imported: {}", announcements_result.imported);
    println!("     Skipped (duplicates): {}", announcements_result.skipped);
    if announcements_result.errors > 0 {
        println!("    Errors: {}", announcements_result.errors);
    }
    
    println!("\n Crypto Listings:");
    println!("    Imported: {}", listings_result.imported);
    println!("     Skipped (duplicates): {}", listings_result.skipped);
    println!("    New tokens created: {}", listings_result.tokens_created);
    if listings_result.errors > 0 {
        println!("    Errors: {}", listings_result.errors);
    }

    Ok(())
}

#[derive(Debug)]
struct AnnouncementEntry {
    title: String,
    source: String,
    announce_date: NaiveDateTime,
    content: String,
    parsed: Option<bool>,
}

#[derive(Debug)]
struct CryptoListingEntry {
    token: String,           // e.g., "AGLDUSD"
    token_name: String,      // e.g., "AGLD"
    listing_announcement_date: Option<String>,  // JSON string
    listing_date: Option<String>,               // JSON string
    delisting_announcement_date: Option<String>, // JSON string
    delisting_date: Option<String>,              // JSON string
}

struct ImportResult {
    imported: usize,
    skipped: usize,
    errors: usize,
    tokens_created: usize,
}

fn parse_dump_file(
    path: &str,
) -> Result<(Vec<AnnouncementEntry>, Vec<CryptoListingEntry>), Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    
    let mut announcements = Vec::new();
    let mut listings = Vec::new();
    
    let mut in_announcements = false;
    let mut in_listings = false;
    
    let copy_announcements_regex = Regex::new(r"COPY\s+(?:public\.)?announcements.*FROM stdin;")?;
    let copy_listings_regex = Regex::new(r"COPY\s+(?:public\.)?crypto_listings.*FROM stdin;")?;
    
    for line in reader.lines() {
        let line = line?;
        
        // Detect start of announcements section
        if copy_announcements_regex.is_match(&line) {
            in_announcements = true;
            in_listings = false;
            println!(" Found announcements section");
            continue;
        }
        
        // Detect start of crypto_listings section
        if copy_listings_regex.is_match(&line) {
            in_listings = true;
            in_announcements = false;
            println!(" Found crypto_listings section");
            continue;
        }
        
        // Detect end of COPY block
        if line.trim() == "\\." {
            if in_announcements {
                println!(" End of announcements section");
                in_announcements = false;
            }
            if in_listings {
                println!(" End of crypto_listings section");
                in_listings = false;
            }
            continue;
        }
        
        // Parse announcements data
        if in_announcements {
            if let Some(entry) = parse_announcement_row(&line) {
                announcements.push(entry);
            }
        }
        
        // Parse crypto_listings data
        if in_listings {
            if let Some(entry) = parse_listing_row(&line) {
                listings.push(entry);
            }
        }
    }
    
    Ok((announcements, listings))
}

fn parse_announcement_row(line: &str) -> Option<AnnouncementEntry> {
    // Format: id \t title \t source \t announce_date \t content \t created_at \t parsed
    let parts: Vec<&str> = line.split('\t').collect();
    
    if parts.len() < 7 {
        return None;
    }
    
    let title = parts[1].to_string();
    let source = parts[2].to_string();
    
    // Parse announce_date
    let announce_date = NaiveDateTime::parse_from_str(parts[3], "%Y-%m-%d %H:%M:%S%.f")
        .ok()?;
    
    let content = parts[4].to_string();
    
    // Parse parsed flag (might be empty or "t"/"f")
    let parsed = match parts[6].trim() {
        "t" => Some(true),
        "f" => Some(false),
        _ => None,
    };
    
    Some(AnnouncementEntry {
        title,
        source,
        announce_date,
        content,
        parsed,
    })
}

fn parse_listing_row(line: &str) -> Option<CryptoListingEntry> {
    // Format: id \t token \t token_name \t listing_announcement_date \t listing_date \t delisting_announcement_date \t delisting_date \t created_at \t updated_at
    let parts: Vec<&str> = line.split('\t').collect();
    
    if parts.len() < 9 {
        return None;
    }
    
    let token = parts[1].to_string();
    let token_name = parts[2].to_string();
    
    // These are JSON strings like: {"binance": "2021-10-05T02:38:02.769Z"}
    let listing_announcement_date = if parts[3] != "{}" { Some(parts[3].to_string()) } else { None };
    let listing_date = if parts[4] != "{}" { Some(parts[4].to_string()) } else { None };
    let delisting_announcement_date = if parts[5] != "{}" { Some(parts[5].to_string()) } else { None };
    let delisting_date = if parts[6] != "{}" { Some(parts[6].to_string()) } else { None };
    
    Some(CryptoListingEntry {
        token,
        token_name,
        listing_announcement_date,
        listing_date,
        delisting_announcement_date,
        delisting_date,
    })
}

async fn import_announcements(
    db: &DatabaseConnection,
    announcements_data: Vec<AnnouncementEntry>,
) -> Result<ImportResult, Box<dyn std::error::Error>> {
    let mut imported = 0;
    let mut skipped = 0;
    let mut errors = 0;
    
    for (idx, entry) in announcements_data.iter().enumerate() {
        // Check if exists by (title, source, announce_date)
        let exists = Announcements::find()
            .filter(announcements::Column::Title.eq(&entry.title))
            .filter(announcements::Column::Source.eq(&entry.source))
            .filter(announcements::Column::AnnounceDate.eq(entry.announce_date))
            .one(db)
            .await;
        
        match exists {
            Ok(Some(_)) => {
                skipped += 1;
            }
            Ok(None) => {
                // Insert new announcement
                let new_announcement = announcements::ActiveModel {
                    title: Set(entry.title.clone()),
                    source: Set(entry.source.clone()),
                    announce_date: Set(entry.announce_date),
                    content: Set(entry.content.clone()),
                    parsed: Set(entry.parsed),
                    ..Default::default()
                };
                
                match new_announcement.insert(db).await {
                    Ok(_) => imported += 1,
                    Err(e) => {
                        eprintln!(" Failed to insert announcement {}: {}", idx + 1, e);
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!(" Database query error at announcement {}: {}", idx + 1, e);
                errors += 1;
            }
        }
        
        // Progress update every 1,000 rows
        if (idx + 1) % 1000 == 0 {
            println!(
                "   Progress: {}/{} (imported: {}, skipped: {}, errors: {})",
                idx + 1,
                announcements_data.len(),
                imported,
                skipped,
                errors
            );
        }
    }
    
    Ok(ImportResult {
        imported,
        skipped,
        errors,
        tokens_created: 0,
    })
}

async fn import_crypto_listings(
    db: &DatabaseConnection,
    listings_data: Vec<CryptoListingEntry>,
) -> Result<ImportResult, Box<dyn std::error::Error>> {
    let mut imported = 0;
    let mut skipped = 0;
    let mut errors = 0;
    let mut tokens_created = 0;
    
    for (idx, entry) in listings_data.iter().enumerate() {
        // Parse trading pair and exchange from token field using token_name
        let (exchange, trading_pair) = match parse_token_field(&entry.token, &entry.token_name) {
            Some((ex, tp)) => (ex, tp),
            None => {
                eprintln!(" Failed to parse token field: {} (token_name: {})", entry.token, entry.token_name);
                errors += 1;
                continue;
            }
        };
        
        // Use lowercase symbol as coin_id (Option A)
        let symbol = entry.token_name.clone();
        let coin_id = symbol.to_lowercase();
        
        // Ensure token exists in token_metadata (create if not exists)
        match ensure_token_exists(db, &symbol).await {
            Ok(true) => tokens_created += 1, // Token was created
            Ok(false) => {}, // Token already existed
            Err(e) => {
                eprintln!(" Failed to ensure token exists for {}: {}", symbol, e);
                errors += 1;
                continue;
            }
        }
        
        // Check if listing exists by (coin_id, exchange, trading_pair)
        let exists = CryptoListings::find()
            .filter(crypto_listings::Column::CoinId.eq(&coin_id))
            .filter(crypto_listings::Column::Exchange.eq(&exchange))
            .filter(crypto_listings::Column::TradingPair.eq(&trading_pair))
            .one(db)
            .await;
        
        match exists {
            Ok(Some(_)) => {
                skipped += 1;
            }
            Ok(None) => {
                // Parse dates from JSON strings
                let listing_announcement_date = entry.listing_announcement_date
                    .as_ref()
                    .and_then(|json| extract_first_date(json));
                
                let listing_date = entry.listing_date
                    .as_ref()
                    .and_then(|json| extract_first_date(json));
                
                let delisting_announcement_date = entry.delisting_announcement_date
                    .as_ref()
                    .and_then(|json| extract_first_date(json));
                
                let delisting_date = entry.delisting_date
                    .as_ref()
                    .and_then(|json| extract_first_date(json));
                
                // Determine status
                let status = if delisting_date.is_some() {
                    "delisted"
                } else {
                    "active"
                };
                
                // Insert new listing
                let new_listing = crypto_listings::ActiveModel {
                    coin_id: Set(coin_id.clone()),
                    symbol: Set(symbol.clone()),
                    token_name: Set(symbol.clone()), // Use symbol as token_name for now
                    exchange: Set(exchange.clone()),
                    trading_pair: Set(trading_pair.clone()),
                    listing_announcement_date: Set(listing_announcement_date),
                    listing_date: Set(listing_date),
                    delisting_announcement_date: Set(delisting_announcement_date),
                    delisting_date: Set(delisting_date),
                    status: Set(status.to_string()),
                    ..Default::default()
                };
                
                match new_listing.insert(db).await {
                    Ok(_) => imported += 1,
                    Err(e) => {
                        eprintln!(" Failed to insert listing {}: {}", idx + 1, e);
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!(" Database query error at listing {}: {}", idx + 1, e);
                errors += 1;
            }
        }
        
        // Progress update every 1,000 rows
        if (idx + 1) % 1000 == 0 {
            println!(
                "   Progress: {}/{} (imported: {}, skipped: {}, tokens_created: {}, errors: {})",
                idx + 1,
                listings_data.len(),
                imported,
                skipped,
                tokens_created,
                errors
            );
        }
    }
    
    Ok(ImportResult {
        imported,
        skipped,
        errors,
        tokens_created,
    })
}

/// Parse token field by subtracting token_name to get trading pair
/// Example: token="AGLDUSD", token_name="AGLD" -> trading_pair="usd"
/// Falls back to known trading pairs if subtraction fails
fn parse_token_field(token: &str, token_name: &str) -> Option<(String, String)> {
    let token_upper = token.to_uppercase();
    let token_name_upper = token_name.to_uppercase();
    
    // Strategy 1: Try subtracting token_name from token (primary)
    if token_upper.starts_with(&token_name_upper) {
        let trading_pair = token_upper.strip_prefix(&token_name_upper)?;
        if !trading_pair.is_empty() {
            return Some(("binance".to_string(), trading_pair.to_lowercase()));
        }
    }
    
    // Strategy 2: Fallback - parse from known trading pairs (backwards extraction)
    let known_pairs = [
        "FDUSD", // Check longer pairs first to avoid false matches
        "USDT",
        "USDC",
        "BUSD",
        "USD",
        "BTC",
        "ETH",
        "BNB",
        "TRY",
        "EUR",
        "GBP",
    ];
    
    for pair in &known_pairs {
        if token_upper.ends_with(pair) {
            let trading_pair = pair.to_lowercase();
            return Some(("binance".to_string(), trading_pair));
        }
    }
    
    // Could not parse with either strategy
    eprintln!("Warning: Could not parse token '{}' with token_name '{}' using either strategy", token, token_name);
    None
}

/// Ensure token exists in token_metadata, create if not exists
async fn ensure_token_exists(
    db: &DatabaseConnection,
    symbol: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Check if exists (case-insensitive)
    let exists = TokenMetadata::find()
        .filter(token_metadata::Column::Symbol.eq(symbol))
        .one(db)
        .await?;
    
    if exists.is_some() {
        return Ok(false); // Already exists
    }
    
    // Create new token
    let new_token = token_metadata::ActiveModel {
        symbol: Set(symbol.to_string()),
        logo_address: Set(None),
        ..Default::default()
    };
    
    new_token.insert(db).await?;
    Ok(true) // Created
}

/// Extract first date from JSON string like {"binance": "2021-10-05T07:00:00.000Z"}
fn extract_first_date(json_str: &str) -> Option<NaiveDateTime> {
    // Simple regex to extract ISO date
    let date_regex = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}").ok()?;
    let caps = date_regex.captures(json_str)?;
    let date_str = caps.get(0)?.as_str();
    
    NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S").ok()
}