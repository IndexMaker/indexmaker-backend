//! IndexMaker MCP Server
//!
//! Provides MCP tools for deploying and managing ITPs via the indexmaker backend API.
//! Includes tools for browsing assets, categories, and generating index compositions.

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Result;
use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::{ErrorData as McpError, *},
    schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt,
    transport::stdio,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::info;

// ============================================================================
// Configuration
// ============================================================================

#[derive(Clone)]
struct Config {
    backend_url: String,
    api_key: String,
}

impl Config {
    fn from_env() -> Result<Self> {
        dotenvy::from_filename("global.env").ok();
        dotenvy::dotenv().ok();

        let backend_url = std::env::var("INDEXMAKER_BACKEND_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3002".to_string());
        let api_key = std::env::var("ADMIN_API_KEY")
            .or_else(|_| std::env::var("INDEXMAKER_BACKEND_API_KEY"))
            .unwrap_or_else(|_| "indexmaker-admin-secret-key-2026".to_string());

        Ok(Self {
            backend_url,
            api_key,
        })
    }
}

// ============================================================================
// Backend API Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CreateItpApiRequest {
    name: String,
    symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    methodology: Option<String>,
    initial_price: u64,
    #[serde(default = "default_max_order_size")]
    max_order_size: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_ids: Option<Vec<u128>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    weights: Option<Vec<u128>>,
    #[serde(default)]
    sync: bool,
}

fn default_max_order_size() -> u128 {
    1_000_000_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ItpErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
}

// Category types
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CategoryResponse {
    category_id: String,
    name: String,
}

// Asset types
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Asset {
    id: String,
    symbol: String,
    name: String,
    #[serde(default)]
    total_supply: f64,
    #[serde(default)]
    circulating_supply: f64,
    #[serde(default)]
    price_usd: f64,
    #[serde(default)]
    market_cap: f64,
    #[serde(default)]
    expected_inventory: f64,
    #[serde(default)]
    thumb: String,
}

// Top category response
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TopCategoryResponse {
    category_id: String,
    category_name: String,
    date: String,
    top: i32,
    coins: Vec<TopCategoryCoin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TopCategoryCoin {
    rank: i32,
    coin_id: String,
    symbol: String,
    name: String,
    market_cap: f64,
    price: f64,
    #[serde(default)]
    volume_24h: Option<f64>,
}

// Market cap history
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MarketCapHistoryResponse {
    coin_id: String,
    symbol: String,
    data: Vec<MarketCapDataPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MarketCapDataPoint {
    date: String,
    market_cap: f64,
    price: f64,
    #[serde(default)]
    volume_24h: Option<f64>,
}

// Composition suggestion
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompositionSuggestion {
    assets: Vec<CompositionAsset>,
    total_market_cap: f64,
    methodology: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompositionAsset {
    coin_id: String,
    symbol: String,
    name: String,
    market_cap: f64,
    price: f64,
    weight_bps: u128,  // Weight in basis points (100 = 1%)
    weight_percent: f64,
}

// ============================================================================
// Backend API Client
// ============================================================================

struct BackendClient {
    client: reqwest::Client,
    config: Config,
}

impl BackendClient {
    fn new(config: Config) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    async fn create_itp(&self, request: CreateItpApiRequest) -> Result<serde_json::Value> {
        let url = format!("{}/api/itp/create", self.config.backend_url);

        let response = self
            .client
            .post(&url)
            .header("X-API-Key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            let error: ItpErrorResponse =
                serde_json::from_str(&body).unwrap_or_else(|_| ItpErrorResponse {
                    error: body,
                    code: None,
                });
            anyhow::bail!("Backend error: {} (code: {:?})", error.error, error.code)
        }
    }

    async fn list_itps(&self) -> Result<serde_json::Value> {
        let url = format!("{}/api/itp/list", self.config.backend_url);

        let response = self
            .client
            .get(&url)
            .header("X-API-Key", &self.config.api_key)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            anyhow::bail!("Failed to list ITPs: {}", body)
        }
    }

    async fn get_itp_status(&self, nonce: u64) -> Result<serde_json::Value> {
        let url = format!("{}/api/itp/status/{}", self.config.backend_url, nonce);

        let response = self
            .client
            .get(&url)
            .header("X-API-Key", &self.config.api_key)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            anyhow::bail!("Failed to get ITP status: {}", body)
        }
    }

    async fn get_categories(&self) -> Result<Vec<CategoryResponse>> {
        let url = format!("{}/coingecko-categories", self.config.backend_url);

        let response = self
            .client
            .get(&url)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            anyhow::bail!("Failed to get categories: {}", body)
        }
    }

    async fn get_all_assets(&self) -> Result<Vec<Asset>> {
        let url = format!("{}/fetch-all-assets", self.config.backend_url);

        let response = self
            .client
            .get(&url)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            anyhow::bail!("Failed to get assets: {}", body)
        }
    }

    async fn get_top_by_category(&self, category_id: &str, top: i32, date: Option<&str>) -> Result<TopCategoryResponse> {
        let mut url = format!(
            "{}/api/market-cap/top-category?category_id={}&top={}",
            self.config.backend_url, category_id, top
        );
        if let Some(d) = date {
            url.push_str(&format!("&date={}", d));
        }

        let response = self
            .client
            .get(&url)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            anyhow::bail!("Failed to get top assets by category: {}", body)
        }
    }

    async fn get_market_cap_history(&self, coin_id: &str, start_date: Option<&str>, end_date: Option<&str>) -> Result<MarketCapHistoryResponse> {
        let mut url = format!(
            "{}/api/market-cap/history?coin_id={}",
            self.config.backend_url, coin_id
        );
        if let Some(s) = start_date {
            url.push_str(&format!("&start_date={}", s));
        }
        if let Some(e) = end_date {
            url.push_str(&format!("&end_date={}", e));
        }

        let response = self
            .client
            .get(&url)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            anyhow::bail!("Failed to get market cap history: {}", body)
        }
    }
}

// ============================================================================
// MCP Tool Parameters
// ============================================================================

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeployItpRequest {
    /// Name of the ITP (e.g., "Top 10 DeFi Index"). 1-64 characters.
    #[schemars(description = "Name of the ITP (e.g., 'Top 10 DeFi Index'). 1-64 characters.")]
    pub name: String,

    /// Symbol of the ITP (e.g., "DEFI10"). 1-8 uppercase alphanumeric characters.
    #[schemars(description = "Symbol of the ITP (e.g., 'DEFI10'). 1-8 uppercase alphanumeric characters.")]
    pub symbol: String,

    /// Initial price in USDC with 6 decimals (e.g., 1000000 = $1.00)
    #[schemars(description = "Initial price in USDC with 6 decimals (e.g., 1000000 = $1.00)")]
    pub initial_price: u64,

    /// Optional description of the ITP
    #[schemars(description = "Optional description of the ITP")]
    pub description: Option<String>,

    /// Optional methodology description
    #[schemars(description = "Optional methodology description")]
    pub methodology: Option<String>,

    /// Maximum order size in USDC (6 decimals). Default: 1000000000 (1000 USDC)
    #[schemars(description = "Maximum order size in USDC (6 decimals). Default: 1000000000 (1000 USDC)")]
    pub max_order_size: Option<u128>,

    /// Asset IDs for the ITP composition
    #[schemars(description = "Asset IDs for the ITP composition")]
    pub asset_ids: Option<Vec<u128>>,

    /// Asset weights in basis points (must sum to 10000 = 100%)
    #[schemars(description = "Asset weights in basis points (must sum to 10000 = 100%)")]
    pub weights: Option<Vec<u128>>,

    /// Wait for deployment completion (default: true). If false, returns immediately with nonce.
    #[schemars(description = "Wait for deployment completion (default: true). If false, returns immediately with nonce.")]
    pub sync: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetItpStatusRequest {
    /// Nonce from the ITP creation response
    #[schemars(description = "Nonce from the ITP creation response")]
    pub nonce: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchCategoriesRequest {
    /// Optional search term to filter categories by name
    #[schemars(description = "Optional search term to filter categories by name (e.g., 'defi', 'layer')")]
    pub search: Option<String>,

    /// Maximum number of results to return (default: 50)
    #[schemars(description = "Maximum number of results to return (default: 50)")]
    pub limit: Option<i32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTopAssetsByCategoryRequest {
    /// CoinGecko category ID (e.g., 'decentralized-finance-defi', 'layer-1')
    #[schemars(description = "CoinGecko category ID (e.g., 'decentralized-finance-defi', 'layer-1')")]
    pub category_id: String,

    /// Number of top assets to return (default: 10)
    #[schemars(description = "Number of top assets to return (default: 10)")]
    pub top: Option<i32>,

    /// Optional date for historical data (YYYY-MM-DD format)
    #[schemars(description = "Optional date for historical data (YYYY-MM-DD format)")]
    pub date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchAssetsRequest {
    /// Optional search term to filter by name or symbol
    #[schemars(description = "Optional search term to filter by name or symbol")]
    pub search: Option<String>,

    /// Minimum market cap filter in USD
    #[schemars(description = "Minimum market cap filter in USD")]
    pub min_market_cap: Option<f64>,

    /// Maximum number of results (default: 50)
    #[schemars(description = "Maximum number of results (default: 50)")]
    pub limit: Option<i32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetMarketCapHistoryRequest {
    /// CoinGecko coin ID (e.g., 'bitcoin', 'ethereum')
    #[schemars(description = "CoinGecko coin ID (e.g., 'bitcoin', 'ethereum')")]
    pub coin_id: String,

    /// Start date (YYYY-MM-DD format)
    #[schemars(description = "Start date (YYYY-MM-DD format)")]
    pub start_date: Option<String>,

    /// End date (YYYY-MM-DD format)
    #[schemars(description = "End date (YYYY-MM-DD format)")]
    pub end_date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GenerateCompositionRequest {
    /// Category ID for the composition (e.g., 'decentralized-finance-defi')
    #[schemars(description = "Category ID for the composition (e.g., 'decentralized-finance-defi')")]
    pub category_id: Option<String>,

    /// Number of assets to include (default: 10)
    #[schemars(description = "Number of assets to include (default: 10)")]
    pub num_assets: Option<i32>,

    /// Weighting method: 'market_cap' (default), 'equal', or 'sqrt_market_cap'
    #[schemars(description = "Weighting method: 'market_cap' (default), 'equal', or 'sqrt_market_cap'")]
    pub weighting: Option<String>,

    /// Maximum weight for any single asset in basis points (default: 3000 = 30%)
    #[schemars(description = "Maximum weight for any single asset in basis points (default: 3000 = 30%)")]
    pub max_weight_bps: Option<u128>,

    /// Minimum market cap filter in USD
    #[schemars(description = "Minimum market cap filter in USD")]
    pub min_market_cap: Option<f64>,
}

// ============================================================================
// MCP Server Implementation
// ============================================================================

#[derive(Clone)]
pub struct IndexMakerMcpServer {
    client: Arc<Mutex<BackendClient>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl IndexMakerMcpServer {
    pub fn new(config: Config) -> Self {
        Self {
            client: Arc::new(Mutex::new(BackendClient::new(config))),
            tool_router: Self::tool_router(),
        }
    }

    // ========================================================================
    // Category & Asset Discovery Tools
    // ========================================================================

    /// List available asset categories from CoinGecko
    #[tool(description = "List available asset categories (e.g., DeFi, Layer 1, Gaming). Use to discover category IDs for building index compositions.")]
    async fn list_categories(
        &self,
        Parameters(params): Parameters<SearchCategoriesRequest>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client.lock().await;

        match client.get_categories().await {
            Ok(mut categories) => {
                // Filter by search term if provided
                if let Some(search) = &params.search {
                    let search_lower = search.to_lowercase();
                    categories.retain(|c| {
                        c.name.to_lowercase().contains(&search_lower) ||
                        c.category_id.to_lowercase().contains(&search_lower)
                    });
                }

                // Limit results
                let limit = params.limit.unwrap_or(50) as usize;
                categories.truncate(limit);

                if categories.is_empty() {
                    Ok(CallToolResult::success(vec![Content::text(
                        "No categories found matching your criteria.",
                    )]))
                } else {
                    let formatted = serde_json::to_string_pretty(&categories).unwrap_or_default();
                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "Found {} categories:\n{}",
                        categories.len(),
                        formatted
                    ))]))
                }
            }
            Err(e) => Err(McpError {
                code: ErrorCode(-32603),
                message: Cow::from(format!("Failed to list categories: {}", e)),
                data: None,
            }),
        }
    }

    /// Get top assets by market cap in a specific category
    #[tool(description = "Get the top N assets by market cap in a specific category. Returns coin IDs, symbols, market caps, and prices. Use this to select assets for an index composition.")]
    async fn get_top_assets_by_category(
        &self,
        Parameters(params): Parameters<GetTopAssetsByCategoryRequest>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client.lock().await;

        let top = params.top.unwrap_or(10);
        match client.get_top_by_category(&params.category_id, top, params.date.as_deref()).await {
            Ok(response) => {
                let formatted = serde_json::to_string_pretty(&response).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Top {} assets in '{}' ({}):\n{}",
                    response.coins.len(),
                    response.category_name,
                    response.category_id,
                    formatted
                ))]))
            }
            Err(e) => Err(McpError {
                code: ErrorCode(-32603),
                message: Cow::from(format!("Failed to get top assets: {}", e)),
                data: None,
            }),
        }
    }

    /// Search and list all available assets with market data
    #[tool(description = "Search all available assets with current market data (price, market cap, supply). Filter by name/symbol or minimum market cap.")]
    async fn search_assets(
        &self,
        Parameters(params): Parameters<SearchAssetsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client.lock().await;

        match client.get_all_assets().await {
            Ok(mut assets) => {
                // Filter by search term
                if let Some(search) = &params.search {
                    let search_lower = search.to_lowercase();
                    assets.retain(|a| {
                        a.name.to_lowercase().contains(&search_lower) ||
                        a.symbol.to_lowercase().contains(&search_lower) ||
                        a.id.to_lowercase().contains(&search_lower)
                    });
                }

                // Filter by minimum market cap
                if let Some(min_cap) = params.min_market_cap {
                    assets.retain(|a| a.market_cap >= min_cap);
                }

                // Sort by market cap descending
                assets.sort_by(|a, b| b.market_cap.partial_cmp(&a.market_cap).unwrap_or(std::cmp::Ordering::Equal));

                // Limit results
                let limit = params.limit.unwrap_or(50) as usize;
                assets.truncate(limit);

                if assets.is_empty() {
                    Ok(CallToolResult::success(vec![Content::text(
                        "No assets found matching your criteria.",
                    )]))
                } else {
                    let formatted = serde_json::to_string_pretty(&assets).unwrap_or_default();
                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "Found {} assets:\n{}",
                        assets.len(),
                        formatted
                    ))]))
                }
            }
            Err(e) => Err(McpError {
                code: ErrorCode(-32603),
                message: Cow::from(format!("Failed to search assets: {}", e)),
                data: None,
            }),
        }
    }

    /// Get historical market cap and price data for an asset
    #[tool(description = "Get historical market cap, price, and volume data for a specific asset. Useful for analyzing asset performance before including in a composition.")]
    async fn get_market_cap_history(
        &self,
        Parameters(params): Parameters<GetMarketCapHistoryRequest>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client.lock().await;

        match client.get_market_cap_history(
            &params.coin_id,
            params.start_date.as_deref(),
            params.end_date.as_deref()
        ).await {
            Ok(response) => {
                let formatted = serde_json::to_string_pretty(&response).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Market cap history for {} ({} data points):\n{}",
                    params.coin_id,
                    response.data.len(),
                    formatted
                ))]))
            }
            Err(e) => Err(McpError {
                code: ErrorCode(-32603),
                message: Cow::from(format!("Failed to get market cap history: {}", e)),
                data: None,
            }),
        }
    }

    // ========================================================================
    // Composition Generation Tool
    // ========================================================================

    /// Generate an index composition with suggested weights
    #[tool(description = "Generate a suggested index composition with weights based on category, number of assets, and weighting method. Returns asset IDs, symbols, and weights ready for ITP deployment.")]
    async fn generate_composition(
        &self,
        Parameters(params): Parameters<GenerateCompositionRequest>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client.lock().await;

        let num_assets = params.num_assets.unwrap_or(10);
        let weighting = params.weighting.as_deref().unwrap_or("market_cap");
        let max_weight_bps = params.max_weight_bps.unwrap_or(3000); // 30% default cap

        // Get assets either from category or all assets
        let assets: Vec<TopCategoryCoin> = if let Some(category_id) = &params.category_id {
            match client.get_top_by_category(category_id, num_assets, None).await {
                Ok(response) => response.coins,
                Err(e) => {
                    return Err(McpError {
                        code: ErrorCode(-32603),
                        message: Cow::from(format!("Failed to get assets from category: {}", e)),
                        data: None,
                    });
                }
            }
        } else {
            // Get all assets and convert to TopCategoryCoin format
            match client.get_all_assets().await {
                Ok(mut assets) => {
                    // Filter by min market cap if specified
                    if let Some(min_cap) = params.min_market_cap {
                        assets.retain(|a| a.market_cap >= min_cap);
                    }

                    // Sort by market cap and take top N
                    assets.sort_by(|a, b| b.market_cap.partial_cmp(&a.market_cap).unwrap_or(std::cmp::Ordering::Equal));
                    assets.truncate(num_assets as usize);

                    assets.iter().enumerate().map(|(i, a)| TopCategoryCoin {
                        rank: (i + 1) as i32,
                        coin_id: a.id.clone(),
                        symbol: a.symbol.clone(),
                        name: a.name.clone(),
                        market_cap: a.market_cap,
                        price: a.price_usd,
                        volume_24h: None,
                    }).collect()
                }
                Err(e) => {
                    return Err(McpError {
                        code: ErrorCode(-32603),
                        message: Cow::from(format!("Failed to get assets: {}", e)),
                        data: None,
                    });
                }
            }
        };

        if assets.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No assets found for composition generation.",
            )]));
        }

        // Calculate weights based on method
        let total_market_cap: f64 = assets.iter().map(|a| a.market_cap).sum();

        let mut composition_assets: Vec<CompositionAsset> = match weighting {
            "equal" => {
                let equal_weight = 10000 / assets.len() as u128;
                assets.iter().map(|a| CompositionAsset {
                    coin_id: a.coin_id.clone(),
                    symbol: a.symbol.clone(),
                    name: a.name.clone(),
                    market_cap: a.market_cap,
                    price: a.price,
                    weight_bps: equal_weight,
                    weight_percent: equal_weight as f64 / 100.0,
                }).collect()
            }
            "sqrt_market_cap" => {
                let total_sqrt: f64 = assets.iter().map(|a| a.market_cap.sqrt()).sum();
                assets.iter().map(|a| {
                    let raw_weight = (a.market_cap.sqrt() / total_sqrt * 10000.0) as u128;
                    CompositionAsset {
                        coin_id: a.coin_id.clone(),
                        symbol: a.symbol.clone(),
                        name: a.name.clone(),
                        market_cap: a.market_cap,
                        price: a.price,
                        weight_bps: raw_weight,
                        weight_percent: raw_weight as f64 / 100.0,
                    }
                }).collect()
            }
            _ => { // market_cap weighted (default)
                assets.iter().map(|a| {
                    let raw_weight = (a.market_cap / total_market_cap * 10000.0) as u128;
                    CompositionAsset {
                        coin_id: a.coin_id.clone(),
                        symbol: a.symbol.clone(),
                        name: a.name.clone(),
                        market_cap: a.market_cap,
                        price: a.price,
                        weight_bps: raw_weight,
                        weight_percent: raw_weight as f64 / 100.0,
                    }
                }).collect()
            }
        };

        // Apply max weight cap and redistribute excess
        let mut excess: u128 = 0;
        let mut capped_count = 0;
        for asset in composition_assets.iter_mut() {
            if asset.weight_bps > max_weight_bps {
                excess += asset.weight_bps - max_weight_bps;
                asset.weight_bps = max_weight_bps;
                asset.weight_percent = max_weight_bps as f64 / 100.0;
                capped_count += 1;
            }
        }

        // Redistribute excess to uncapped assets
        if excess > 0 && capped_count < composition_assets.len() {
            let uncapped_count = composition_assets.len() - capped_count;
            let extra_per_asset = excess / uncapped_count as u128;
            for asset in composition_assets.iter_mut() {
                if asset.weight_bps < max_weight_bps {
                    asset.weight_bps += extra_per_asset;
                    asset.weight_percent = asset.weight_bps as f64 / 100.0;
                }
            }
        }

        // Ensure weights sum to 10000
        let total_weight: u128 = composition_assets.iter().map(|a| a.weight_bps).sum();
        if total_weight != 10000 && !composition_assets.is_empty() {
            let diff = 10000i128 - total_weight as i128;
            composition_assets[0].weight_bps = (composition_assets[0].weight_bps as i128 + diff) as u128;
            composition_assets[0].weight_percent = composition_assets[0].weight_bps as f64 / 100.0;
        }

        let methodology = format!(
            "{} weighted, {} assets, max {}% per asset",
            weighting,
            composition_assets.len(),
            max_weight_bps as f64 / 100.0
        );

        let suggestion = CompositionSuggestion {
            assets: composition_assets,
            total_market_cap,
            methodology,
        };

        let formatted = serde_json::to_string_pretty(&suggestion).unwrap_or_default();

        // Also output the weights array ready for deploy_itp
        let weights: Vec<u128> = suggestion.assets.iter().map(|a| a.weight_bps).collect();
        let symbols: Vec<&str> = suggestion.assets.iter().map(|a| a.symbol.as_str()).collect();

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Generated composition suggestion:\n{}\n\n--- Ready for deploy_itp ---\nSymbols: {:?}\nWeights (basis points): {:?}\nTotal: {} bps (should be 10000)",
            formatted,
            symbols,
            weights,
            weights.iter().sum::<u128>()
        ))]))
    }

    // ========================================================================
    // ITP Deployment Tools
    // ========================================================================

    /// Deploy a new ITP (Index Token Product) to Arbitrum and Orbit chains.
    #[tool(description = "Deploy a new ITP (Index Token Product) to Arbitrum and Orbit chains. Creates a new tradeable index token with the specified name, symbol, and initial price. Returns transaction hash and deployment addresses.")]
    async fn deploy_itp(
        &self,
        Parameters(params): Parameters<DeployItpRequest>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client.lock().await;

        let request = CreateItpApiRequest {
            name: params.name,
            symbol: params.symbol,
            description: params.description,
            methodology: params.methodology,
            initial_price: params.initial_price,
            max_order_size: params.max_order_size.unwrap_or(default_max_order_size()),
            asset_ids: params.asset_ids,
            weights: params.weights,
            sync: params.sync.unwrap_or(true),
        };

        match client.create_itp(request).await {
            Ok(response) => {
                let formatted = serde_json::to_string_pretty(&response).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(formatted)]))
            }
            Err(e) => Err(McpError {
                code: ErrorCode(-32603),
                message: Cow::from(format!("Failed to deploy ITP: {}", e)),
                data: None,
            }),
        }
    }

    /// List all ITPs that have been created.
    #[tool(description = "List all ITPs (Index Token Products) that have been created. Returns a list with names, symbols, and addresses.")]
    async fn list_itps(&self) -> Result<CallToolResult, McpError> {
        let client = self.client.lock().await;

        match client.list_itps().await {
            Ok(itps) => {
                let formatted = serde_json::to_string_pretty(&itps).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(formatted)]))
            }
            Err(e) => Err(McpError {
                code: ErrorCode(-32603),
                message: Cow::from(format!("Failed to list ITPs: {}", e)),
                data: None,
            }),
        }
    }

    /// Get the deployment status of an ITP by its nonce.
    #[tool(description = "Get the deployment status of an ITP by its nonce. Use this to check if an async deployment has completed and retrieve the final addresses.")]
    async fn get_itp_status(
        &self,
        Parameters(params): Parameters<GetItpStatusRequest>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client.lock().await;

        match client.get_itp_status(params.nonce).await {
            Ok(status) => {
                let formatted = serde_json::to_string_pretty(&status).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(formatted)]))
            }
            Err(e) => Err(McpError {
                code: ErrorCode(-32603),
                message: Cow::from(format!("Failed to get ITP status: {}", e)),
                data: None,
            }),
        }
    }
}

#[tool_handler]
impl ServerHandler for IndexMakerMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "IndexMaker MCP Server provides tools for creating index compositions and deploying ITPs (Index Token Products).\n\n\
                WORKFLOW:\n\
                1. Use list_categories to browse available categories (DeFi, Layer 1, etc.)\n\
                2. Use get_top_assets_by_category to see top assets in a category\n\
                3. Use generate_composition to create a weighted portfolio suggestion\n\
                4. Use deploy_itp with the generated weights to deploy the index\n\n\
                TOOLS:\n\
                - list_categories: Browse CoinGecko categories\n\
                - get_top_assets_by_category: Get top N assets by market cap in category\n\
                - search_assets: Search all assets by name/symbol\n\
                - get_market_cap_history: Historical price/market cap data\n\
                - generate_composition: Create weighted index composition\n\
                - deploy_itp: Deploy an ITP to blockchain\n\
                - list_itps: List deployed ITPs\n\
                - get_itp_status: Check deployment status"
                    .to_string(),
            ),
        }
    }
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging to stderr (stdout is used for MCP protocol)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("indexmaker_mcp_server=info".parse().unwrap()),
        )
        .with_writer(std::io::stderr)
        .init();

    info!("Starting IndexMaker MCP Server");

    // Load configuration
    let config = Config::from_env()?;
    info!("Backend URL: {}", config.backend_url);

    // Create server and serve via stdio
    let server = IndexMakerMcpServer::new(config);
    let service = server.serve(stdio()).await?;
    let quit_reason = service.waiting().await?;

    info!("Server stopped: {:?}", quit_reason);

    Ok(())
}
