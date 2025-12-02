// src/bin/import_tokens_historical_prices.rs

use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use sea_orm::{Database, EntityTrait, Set, ColumnTrait, QueryFilter, ActiveModelTrait};
use regex::Regex;

use indexmaker_backend::entities::{historical_prices, prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Usage: cargo run --bin import_tokens_historical_prices -- dump.sql
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
        // Check if exists by coin_id AND timestamp (unique combination)
        let exists = HistoricalPrices::find()
            .filter(historical_prices::Column::CoinId.eq(&price.coin_id))
            .filter(historical_prices::Column::Timestamp.eq(price.timestamp))
            .one(&db)
            .await;

        match exists {
            Ok(Some(_)) => {
                // Already exists, skip
                skipped += 1;
            }
            Ok(None) => {
                // Insert new entry
                let new_price = historical_prices::ActiveModel {
                    coin_id: Set(price.coin_id.clone()),
                    symbol: Set(price.symbol.clone()),
                    timestamp: Set(price.timestamp),
                    price: Set(price.price),
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
    timestamp: i32,
    price: f64,
}

fn parse_dump_file(path: &str) -> Result<Vec<PriceEntry>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    let mut in_historical_prices = false;

    // Regex to match: COPY public.historical_prices (id, coin_id, symbol, "timestamp", price) FROM stdin;
    let copy_regex = Regex::new(r"COPY\s+(?:public\.)?historical_prices.*FROM stdin;")?;
    
    // Regex to match data rows: id \t coin_id \t symbol \t timestamp \t price
    // Example: 13082	stellar	xlm	1487376000	0.002018999999999999
    let row_regex = Regex::new(r"^(\d+)\t([^\t]+)\t([^\t]+)\t(\d+)\t([\d.]+)")?;

    for line in reader.lines() {
        let line = line?;

        // Detect start of historical_prices section
        if copy_regex.is_match(&line) {
            in_historical_prices = true;
            println!("Found historical_prices section");
            continue;
        }

        // Detect end of COPY block
        if line.trim() == "\\." {
            if in_historical_prices {
                println!("End of historical_prices section");
                in_historical_prices = false;
            }
            continue;
        }

        // Parse data rows
        if in_historical_prices {
            if let Some(caps) = row_regex.captures(&line) {
                entries.push(PriceEntry {
                    coin_id: caps[2].to_string(),
                    symbol: caps[3].to_string(),
                    timestamp: caps[4].parse()?,
                    price: caps[5].parse()?,
                });
            }
        }
    }

    Ok(entries)
}