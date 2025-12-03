use std::env;
use std::fs;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, Database, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::Value;

use indexmaker_backend::entities::{crypto_listings, index_constituents, index_metadata, prelude::*};

#[derive(Debug, Deserialize)]
struct DeploymentFile {
    indexes: Vec<IndexDeployment>,
}

#[derive(Debug, Deserialize)]
struct IndexDeployment {
    index_address: String,
    deployment_data: Value, // Store entire deployment_data as JSON
}

#[derive(Debug, Deserialize)]
struct TokenEntry {
    pair: String,
    listing: String,
    assetname: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: {} <index_id> <deployment.json> <tokens.json>", args[0]);
        eprintln!("Example: {} 1 deployment.json sy100_tokens.json", args[0]);
        std::process::exit(1);
    }
    
    let index_id: i32 = args[1].parse()?;
    let deployment_file = &args[2];
    let tokens_file = &args[3];
    
    dotenvy::dotenv().ok();
    let db = Database::connect(env::var("DATABASE_URL")?).await?;
    
    // Read deployment file
    let deployment_json = fs::read_to_string(deployment_file)?;
    let deployment: DeploymentFile = serde_json::from_str(&deployment_json)?;
    
    // Check if index_id already exists
    let existing = IndexMetadata::find_by_id(index_id).one(&db).await?;
    
    if existing.is_some() {
        println!("Index {} already exists, updating deployment_data...", index_id);
        
        // Find matching index by address (you need to specify which address to match)
        // For now, let's assume we update the first one or match by some logic
        println!("Please provide logic to match index from deployment file");
        println!("Available indexes in deployment file:");
        for (i, idx) in deployment.indexes.iter().enumerate() {
            println!("  [{}] address: {}", i, idx.index_address);
        }
        
        return Err("Update logic not yet implemented - please specify which index to use".into());
    }
    
    // Ask user which index from deployment file to use
    println!("Found {} indexes in deployment file:", deployment.indexes.len());
    for (i, idx) in deployment.indexes.iter().enumerate() {
        // Try to extract name from deployment_data
        let name = idx.deployment_data
            .get("index_deploy_data")
            .and_then(|d| d.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown");
        
        println!("  [{}] {} - {}", i, idx.index_address, name);
    }
    
    println!("\nUsing first index from deployment file...");
    let selected_index = &deployment.indexes[0];
    
    // Extract name, symbol, address from deployment_data
    let deploy_data = selected_index.deployment_data
        .get("index_deploy_data")
        .ok_or("Missing index_deploy_data")?;
    
    let name = deploy_data.get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing name")?
        .to_string();
    
    let symbol = deploy_data.get("symbol")
        .and_then(|v| v.as_str())
        .ok_or("Missing symbol")?
        .to_string();
    
    let address = selected_index.index_address.clone();
    
    // Parse initial_price from hex
    let initial_price_hex = deploy_data.get("initial_price")
        .and_then(|v| v.as_str())
        .ok_or("Missing initial_price")?;
    
    let initial_price = parse_hex_to_decimal(initial_price_hex)?;
    
    println!("\n=== Creating Index ===");
    println!("ID: {}", index_id);
    println!("Name: {}", name);
    println!("Symbol: {}", symbol);
    println!("Address: {}", address);
    println!("Initial Price: {}", initial_price);
    
    // Create index_metadata
    let new_index = index_metadata::ActiveModel {
        index_id: Set(index_id),
        name: Set(name.clone()),
        symbol: Set(symbol.clone()),
        address: Set(address.clone()),
        category: Set(Some(name.clone())), // Use name as category
        asset_class: Set(Some("Cryptocurrencies".to_string())),
        token_ids: Set(vec![]),
        initial_date: Set(Some(chrono::Utc::now().date_naive())),
        initial_price: Set(Some(initial_price)),
        coingecko_category: Set(None),
        exchanges_allowed: Set(Some(serde_json::json!(["binance", "bitget"]))),
        exchange_trading_fees: Set(Some(rust_decimal::Decimal::new(1, 3))), // 0.001
        exchange_avg_spread: Set(Some(rust_decimal::Decimal::new(5, 4))), // 0.0005
        rebalance_period: Set(Some(30)),
        deployment_data: Set(Some(selected_index.deployment_data.clone())),
        ..Default::default()
    };
    
    new_index.insert(&db).await?;
    println!("✓ Created index metadata");
    
    // Read tokens file
    let tokens_json = fs::read_to_string(tokens_file)?;
    let tokens: Vec<TokenEntry> = serde_json::from_str(&tokens_json)?;
    
    println!("\n=== Importing {} Constituents ===", tokens.len());
    
    let mut success = 0;
    let mut skipped = 0;
    
    for (pos, token) in tokens.iter().enumerate() {
        let (symbol, trading_pair) = match parse_pair(&token.pair) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("⚠ Skipping {}: {}", token.pair, e);
                skipped += 1;
                continue;
            }
        };
        
        let exchange = token.listing.to_lowercase();
        
        // Lookup coin_id
        let listing = CryptoListings::find()
            .filter(crypto_listings::Column::Symbol.eq(&symbol))
            .filter(crypto_listings::Column::Exchange.eq(&exchange))
            .filter(crypto_listings::Column::TradingPair.eq(&trading_pair))
            .one(&db)
            .await?;
        
        let coin_id = match listing {
            Some(l) => l.coin_id,
            None => {
                eprintln!("⚠ Skipping {}: not found in crypto_listings", token.pair);
                skipped += 1;
                continue;
            }
        };
        
        // Insert constituent
        let constituent = index_constituents::ActiveModel {
            index_id: Set(index_id),
            coin_id: Set(coin_id.clone()),
            token_symbol: Set(symbol),
            token_name: Set(token.assetname.clone()),
            exchange: Set(exchange),
            trading_pair: Set(trading_pair),
            position: Set((pos + 1) as i32),
            added_at: Set(Some(Utc::now().naive_utc())),
            removed_at: Set(None),
            ..Default::default()
        };
        
        constituent.insert(&db).await?;
        println!("✓ [{:3}] {}", pos + 1, coin_id);
        success += 1;
    }
    
    println!("\n=== Summary ===");
    println!("Index: {} ({})", name, symbol);
    println!("Imported: {}", success);
    println!("Skipped: {}", skipped);
    println!("Total: {}", tokens.len());
    println!("\n✓ Done!");
    
    Ok(())
}

fn parse_pair(pair: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let p = pair.to_uppercase();
    for quote in ["USDC", "USDT", "BTC", "ETH", "BNB"] {
        if p.ends_with(quote) {
            let symbol = p.trim_end_matches(quote);
            if !symbol.is_empty() {
                return Ok((symbol.to_string(), quote.to_lowercase()));
            }
        }
    }
    Err(format!("Cannot parse pair: {}", pair).into())
}

fn parse_hex_to_decimal(hex: &str) -> Result<rust_decimal::Decimal, Box<dyn std::error::Error>> {
    use rust_decimal::Decimal;
    
    // Remove 0x prefix
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    
    // Parse as u128
    let value = u128::from_str_radix(hex, 16)?;
    
    // Convert to Decimal (assuming 18 decimals like wei to ether)
    let decimal_value = Decimal::from(value);
    let divisor = Decimal::from(10u128.pow(18));
    
    Ok(decimal_value / divisor)
}
