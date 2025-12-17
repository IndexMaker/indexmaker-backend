// src/bin/import_coins_historical_prices.rs

use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use sea_orm::{Database, EntityTrait, Set, ColumnTrait, QueryFilter, ActiveModelTrait};
use regex::Regex;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::str::FromStr;

use indexmaker_backend::entities::{coins_historical_prices, prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Usage: cargo run --bin import_coins_historical_prices -- dump.sql
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <dump.sql>", args[0]);
        std::process::exit(1);
    }

    let file_path = &args[1];
    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    
    let db = Database::connect(&database_url).await?;

    println!("Parsing {}...", file_path);
    let prices = parse_dump_file(file_path)?;
    
    println!("Found {} price entries", prices.len());
    println!("Importing to database...");
    
    let mut imported = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for (idx, price) in prices.iter().enumerate() {
        // Check if exists by coin_id AND date (unique combination)
        let exists = CoinsHistoricalPrices::find()
            .filter(coins_historical_prices::Column::CoinId.eq(&price.coin_id))
            .filter(coins_historical_prices::Column::Date.eq(price.date))
            .one(&db)
            .await;

        match exists {
            Ok(Some(_)) => {
                // Already exists, skip
                skipped += 1;
            }
            Ok(None) => {
                // Insert new entry
                let new_price = coins_historical_prices::ActiveModel {
                    coin_id: Set(price.coin_id.clone()),
                    symbol: Set(price.symbol.clone()),
                    date: Set(price.date),
                    price: Set(price.price),
                    market_cap: Set(price.market_cap),
                    volume: Set(price.volume),
                    ..Default::default()
                };

                match new_price.insert(&db).await {
                    Ok(_) => imported += 1,
                    Err(e) => {
                        eprintln!("Failed to insert row {}: {}", idx + 1, e);
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("Database query error at row {}: {}", idx + 1, e);
                errors += 1;
            }
        }

        // Progress update every 10,000 rows
        if (idx + 1) % 10000 == 0 {
            println!(
                "   Progress: {}/{} (imported: {}, skipped: {}, errors: {})",
                idx + 1,
                prices.len(),
                imported,
                skipped,
                errors
            );
        }
    }

    println!("\nImport complete!");
    println!("   Imported: {}", imported);
    println!("   Skipped (duplicates): {}", skipped);
    if errors > 0 {
        println!("   Errors: {}", errors);
    }

    Ok(())
}

struct PriceEntry {
    coin_id: String,
    symbol: String,
    date: NaiveDate,
    price: Decimal,
    market_cap: Option<Decimal>,
    volume: Option<Decimal>,
}

fn parse_dump_file(path: &str) -> Result<Vec<PriceEntry>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    let mut in_coins_historical_prices = false;

    // Regex to match: COPY public.coins_historical_prices (id, coin_id, symbol, date, price, market_cap, volume, created_at) FROM stdin;
    let copy_regex = Regex::new(r"COPY\s+(?:public\.)?coins_historical_prices.*FROM stdin;")?;
    
    // Regex to match data rows: id \t coin_id \t symbol \t date \t price \t market_cap \t volume \t created_at
    // Example: 11287363	bitcoin	BTC	2025-12-16	86413.9190195637056604027748	1724624039059.302978515625	50630498834.055145263671875	2025-12-16 15:11:03.065892
    let row_regex = Regex::new(
        r"^(\d+)\t([^\t]+)\t([^\t]+)\t(\d{4}-\d{2}-\d{2})\t([\d.]+)\t([^\t]*)\t([^\t]*)\t"
    )?;

    for line in reader.lines() {
        let line = line?;

        // Detect start of coins_historical_prices section
        if copy_regex.is_match(&line) {
            in_coins_historical_prices = true;
            println!("Found coins_historical_prices section");
            continue;
        }

        // Detect end of COPY block
        if line.trim() == "\\." {
            if in_coins_historical_prices {
                println!("End of coins_historical_prices section");
                in_coins_historical_prices = false;
            }
            continue;
        }

        // Parse data rows
        if in_coins_historical_prices {
            if let Some(caps) = row_regex.captures(&line) {
                // Parse date
                let date = NaiveDate::parse_from_str(&caps[4], "%Y-%m-%d")?;
                
                // Parse price
                let price = Decimal::from_str(&caps[5])?;
                
                // Parse optional market_cap
                let market_cap = if caps[6].is_empty() || &caps[6] == "\\N" {
                    None
                } else {
                    Some(Decimal::from_str(&caps[6])?)
                };
                
                // Parse optional volume
                let volume = if caps[7].is_empty() || &caps[7] == "\\N" {
                    None
                } else {
                    Some(Decimal::from_str(&caps[7])?)
                };

                entries.push(PriceEntry {
                    coin_id: caps[2].to_string(),
                    symbol: caps[3].to_string(),
                    date,
                    price,
                    market_cap,
                    volume,
                });
            }
        }
    }

    Ok(entries)
}