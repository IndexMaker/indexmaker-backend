use std::env;
use std::fs;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, Database, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::Value;

use indexmaker_backend::entities::{index_constituents, index_metadata, prelude::*};

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
        eprintln!("Usage: {} <symbol> <deployment.json> <tokens.json>", args[0]);
        eprintln!("Example: {} SY100 deployment.json sy100_tokens.json", args[0]);
        std::process::exit(1);
    }
    
    let symbol_arg = &args[1];
    let deployment_file = &args[2];
    let tokens_file = &args[3];
    
    dotenvy::dotenv().ok();
    let db = Database::connect(env::var("DATABASE_URL")?).await?;
    
    // Read deployment file
    let deployment_json = fs::read_to_string(deployment_file)?;
    let deployment: DeploymentFile = serde_json::from_str(&deployment_json)?;
    
    // Find matching index in deployment file by symbol
    println!("Found {} indexes in deployment file:", deployment.indexes.len());
    for idx in deployment.indexes.iter() {
        // Try to extract name and symbol from deployment_data
        let deploy_data = idx.deployment_data.get("index_deploy_data");
        let name = deploy_data
            .and_then(|d| d.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown");
        
        let symbol = deploy_data
            .and_then(|d| d.get("symbol"))
            .and_then(|s| s.as_str())
            .unwrap_or("Unknown");
        
        println!("  Symbol: {}, Address: {}, Name: {}", symbol, idx.index_address, name);
    }
    
    // Find the index that matches our symbol in deployment JSON
    let selected_index = deployment.indexes.iter()
        .find(|idx| {
            idx.deployment_data
                .get("index_deploy_data")
                .and_then(|d| d.get("symbol"))
                .and_then(|s| s.as_str())
                .map(|s| s.eq_ignore_ascii_case(symbol_arg))
                .unwrap_or(false)
        })
        .ok_or(format!("Index with symbol '{}' not found in deployment file", symbol_arg))?;
    
    println!("✓ Found index with symbol '{}' in deployment file", symbol_arg);
    
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
    
    // Check if index with this symbol exists in database (REQUIRED)
    let existing = IndexMetadata::find()
        .filter(index_metadata::Column::Symbol.eq(&symbol))
        .one(&db)
        .await?
        .ok_or(format!(
            "Index with symbol '{}' not found in database. Please create it first using /create-index endpoint.",
            symbol
        ))?;
    
    let existing_index_id = existing.index_id;
    
    println!("✓ Found index with symbol '{}' in database (ID: {})", symbol, existing_index_id);
    println!("\n=== Updating Index ===");
    println!("ID: {}", existing_index_id);
    println!("Name: {} → {}", existing.name, name);
    println!("Symbol: {}", symbol);
    println!("Address: {} → {}", existing.address, address);
    println!("Initial Price: {:?} → {}", existing.initial_price, initial_price);
    
    // Update existing index with deployment data
    use sea_orm::IntoActiveModel;
    let mut active_model = existing.into_active_model();
    
    active_model.name = Set(name.clone());
    active_model.symbol = Set(symbol.clone());
    active_model.address = Set(address.clone());
    active_model.category = Set(Some(name.clone()));
    active_model.asset_class = Set(Some("Cryptocurrencies".to_string()));
    active_model.initial_price = Set(Some(initial_price));
    active_model.exchanges_allowed = Set(Some(serde_json::json!(["binance", "bitget"])));
    active_model.exchange_trading_fees = Set(Some(rust_decimal::Decimal::new(1, 3)));
    active_model.exchange_avg_spread = Set(Some(rust_decimal::Decimal::new(5, 4)));
    active_model.rebalance_period = Set(Some(30));
    active_model.deployment_data = Set(Some(selected_index.deployment_data.clone()));
    
    active_model.update(&db).await?;
    println!("✓ Updated index metadata");
    
    let final_index_id = existing_index_id;
    
    // Read tokens file
    let tokens_json = fs::read_to_string(tokens_file)?;
    let tokens: Vec<TokenEntry> = serde_json::from_str(&tokens_json)?;
    
    println!("\n=== Importing {} Constituents ===", tokens.len());
    
    // Remove old constituents first (always, since we always update)
    let deleted = IndexConstituents::delete_many()
        .filter(index_constituents::Column::IndexId.eq(final_index_id))
        .exec(&db)
        .await?;
    println!("ℹ Removed {} old constituents", deleted.rows_affected);
    
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
        
        // Use symbol as coin_id (will be validated during rebalancing)
        let coin_id = symbol.to_lowercase();
        
        // Insert constituent
        let constituent = index_constituents::ActiveModel {
            index_id: Set(final_index_id),
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