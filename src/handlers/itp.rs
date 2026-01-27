//! ITP (Index Token Product) creation handler
//!
//! POST /api/itp/create endpoint for creating live ITPs via BridgeProxy on Arbitrum.
//! Admin-only endpoint protected by API key authentication.

use axum::{
    extract::State,
    http::{header::HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, Set};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::entities::itps;
use crate::models::itp::{CreateItpRequest, CreateItpResponse, CreateItpSyncResponse, ItpErrorResponse};
use crate::services::itp_creation::{ItpCreationError, ItpCreationService};
use crate::AppState;

/// Default estimated completion time in seconds
const DEFAULT_COMPLETION_TIME: u32 = 30;

/// Max name length
const MAX_NAME_LENGTH: usize = 64;

/// Max symbol length
const MAX_SYMBOL_LENGTH: usize = 8;

/// Max initial price (1 trillion USDC)
const MAX_INITIAL_PRICE: u64 = 1_000_000_000_000;

/// Rate limit: max creations per minute per API key
const RATE_LIMIT_PER_MINUTE: usize = 10;

/// Sanitize input string by removing control characters, null bytes, and normalizing whitespace
fn sanitize_input(input: &str) -> String {
    input
        .chars()
        // Remove null bytes and control characters (except space)
        .filter(|c| !c.is_control() || *c == ' ')
        // Remove Unicode control characters (categories Cc, Cf, Co, Cs)
        .filter(|c| !matches!(c, '\u{0000}'..='\u{001F}' | '\u{007F}'..='\u{009F}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202F}' | '\u{FEFF}'))
        .collect::<String>()
        // Normalize multiple spaces to single space
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        // Trim leading/trailing whitespace
        .trim()
        .to_string()
}

/// Per-API-key rate limit tracking (AC #5.5)
struct PerKeyRateLimiter {
    /// Map of API key hash -> list of request timestamps
    keys: HashMap<String, Vec<Instant>>,
}

impl PerKeyRateLimiter {
    fn new() -> Self {
        Self { keys: HashMap::new() }
    }

    /// Check if request is allowed for the given API key and record it if so
    fn check_and_record(&mut self, api_key: &str) -> bool {
        let now = Instant::now();
        let one_minute_ago = now - std::time::Duration::from_secs(60);

        // Get or create timestamps for this API key
        let timestamps = self.keys.entry(api_key.to_string()).or_insert_with(Vec::new);

        // Remove timestamps older than 1 minute
        timestamps.retain(|t| *t > one_minute_ago);

        if timestamps.len() >= RATE_LIMIT_PER_MINUTE {
            return false;
        }

        timestamps.push(now);
        true
    }

    /// Periodically clean up old entries to prevent memory growth
    fn cleanup_stale_keys(&mut self) {
        let one_minute_ago = Instant::now() - std::time::Duration::from_secs(60);
        self.keys.retain(|_, timestamps| {
            timestamps.retain(|t| *t > one_minute_ago);
            !timestamps.is_empty()
        });
    }
}

lazy_static::lazy_static! {
    static ref RATE_LIMITER: Arc<Mutex<PerKeyRateLimiter>> = Arc::new(Mutex::new(PerKeyRateLimiter::new()));
}

/// Create ITP endpoint handler
///
/// POST /api/itp/create
///
/// Creates a new ITP via BridgeProxy.requestCreateItp() on Arbitrum.
/// Requires admin API key in X-API-Key header.
///
/// # Request Body
///
/// ```json
/// {
///   "name": "Top 10 DeFi Index",
///   "symbol": "DEFI10",
///   "initial_price": 1000000,
///   "sync": false
/// }
/// ```
///
/// # Response (async mode, sync=false)
///
/// ```json
/// {
///   "tx_hash": "0x...",
///   "nonce": 0,
///   "estimated_completion_time": 30,
///   "status": "pending"
/// }
/// ```
///
/// # Response (sync mode, sync=true)
///
/// ```json
/// {
///   "tx_hash": "0x...",
///   "nonce": 0,
///   "orbit_address": "0x...",
///   "arbitrum_address": "0x...",
///   "status": "completed"
/// }
/// ```
pub async fn create_itp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateItpRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ItpErrorResponse>)> {
    let correlation_id = uuid::Uuid::new_v4().to_string();
    info!(
        correlation_id = %correlation_id,
        name = %payload.name,
        symbol = %payload.symbol,
        sync = payload.sync,
        "ITP creation request received"
    );

    // Check admin authentication (returns API key for rate limiting)
    let api_key = check_admin_auth(&headers)?;

    // Check per-API-key rate limit (AC #5.5)
    {
        let mut limiter = RATE_LIMITER.lock().await;
        if !limiter.check_and_record(&api_key) {
            warn!(correlation_id = %correlation_id, "Rate limit exceeded for API key");
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(ItpErrorResponse {
                    error: "Rate limit exceeded. Max 10 creations per minute per API key.".to_string(),
                    code: Some("RATE_LIMIT_EXCEEDED".to_string()),
                }),
            ));
        }
        // Periodically cleanup stale entries
        limiter.cleanup_stale_keys();
    }

    // Sanitize inputs (AC #5.6) - remove control chars, null bytes, normalize whitespace
    let sanitized_name = sanitize_input(&payload.name);
    let sanitized_symbol = sanitize_input(&payload.symbol);

    // Sanitize description and methodology
    let sanitized_description = payload.description.as_ref().map(|d| sanitize_input(d)).unwrap_or_default();
    let sanitized_methodology = payload.methodology.as_ref().map(|m| sanitize_input(m)).unwrap_or_default();

    // Create sanitized payload for validation
    let sanitized_payload = CreateItpRequest {
        name: sanitized_name.clone(),
        symbol: sanitized_symbol.clone(),
        description: Some(sanitized_description.clone()),
        methodology: Some(sanitized_methodology.clone()),
        initial_price: payload.initial_price,
        max_order_size: payload.max_order_size,
        asset_ids: payload.asset_ids.clone(),
        weights: payload.weights.clone(),
        asset_composition: payload.asset_composition.clone(),
        sync: payload.sync,
        admin_address: payload.admin_address.clone(),
    };

    // Validate sanitized request
    validate_create_itp_request(&sanitized_payload)?;

    // Get configuration from environment
    let rpc_url = std::env::var("ARB_RPC_URL").map_err(|_| {
        error!(correlation_id = %correlation_id, "ARB_RPC_URL not configured");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: "Server configuration error".to_string(),
                code: Some("CONFIG_ERROR".to_string()),
            }),
        )
    })?;

    let private_key = std::env::var("ARBITRUM_PRIVATE_KEY")
        .or_else(|_| std::env::var("DEPLOY_PRIVATE_KEY"))
        .map_err(|_| {
            error!(correlation_id = %correlation_id, "ARBITRUM_PRIVATE_KEY not configured");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ItpErrorResponse {
                    error: "Server configuration error".to_string(),
                    code: Some("CONFIG_ERROR".to_string()),
                }),
            )
        })?;

    let bridge_proxy_address = std::env::var("BRIDGE_PROXY_ADDRESS").map_err(|_| {
        error!(correlation_id = %correlation_id, "BRIDGE_PROXY_ADDRESS not configured");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: "Server configuration error".to_string(),
                code: Some("CONFIG_ERROR".to_string()),
            }),
        )
    })?;

    // Initialize service
    let service = ItpCreationService::new(&rpc_url, &private_key, &bridge_proxy_address)
        .await
        .map_err(|e| {
            error!(
                correlation_id = %correlation_id,
                error = %e,
                "Failed to initialize ITP creation service"
            );
            map_creation_error(e)
        })?;

    // Extract asset IDs and weights (default to empty vectors if not provided)
    let asset_ids = payload.asset_ids.clone().unwrap_or_default();
    let weights = payload.weights.clone().unwrap_or_default();

    // Execute creation based on sync mode (using sanitized inputs)
    if payload.sync {
        // Sync mode: wait for completion
        let result = service
            .request_create_itp_sync(
                &sanitized_name,
                &sanitized_symbol,
                &sanitized_description,
                &sanitized_methodology,
                payload.initial_price,
                payload.max_order_size,
                asset_ids.clone(),
                weights.clone(),
            )
            .await
            .map_err(|e| {
                error!(
                    correlation_id = %correlation_id,
                    error = %e,
                    "ITP creation failed (sync mode)"
                );
                map_creation_error(e)
            })?;

        info!(
            correlation_id = %correlation_id,
            tx_hash = %result.tx_hash,
            nonce = result.nonce,
            orbit_address = %result.orbit_address,
            arbitrum_address = %result.arbitrum_address,
            "ITP creation completed (sync mode)"
        );

        // Save ITP to database
        // Convert initial_price from 6 decimals (USDC) to 18 decimals for storage
        let initial_price_18dec = Decimal::from(payload.initial_price) * Decimal::from(1_000_000_000_000u64);
        let asset_json = payload.asset_composition.as_ref()
            .map(|a| serde_json::json!(a));
        // Convert weights from basis points (10000 = 100%) to decimal (1.0 = 100%)
        let weights_json = payload.weights.as_ref()
            .map(|w| serde_json::json!(w.iter().map(|bp| *bp as f64 / 10000.0).collect::<Vec<f64>>()));

        let itp = itps::ActiveModel {
            orbit_address: Set(result.orbit_address.clone()),
            arbitrum_address: Set(Some(result.arbitrum_address.clone())),
            name: Set(sanitized_name.clone()),
            symbol: Set(sanitized_symbol.clone()),
            description: Set(Some(sanitized_description.clone())),
            methodology: Set(Some(sanitized_methodology.clone())),
            initial_price: Set(Some(initial_price_18dec)),
            current_price: Set(Some(initial_price_18dec)),
            total_supply: Set(Some(Decimal::ZERO)),
            state: Set(1), // Active
            deploy_tx_hash: Set(Some(result.tx_hash.clone())),
            admin_address: Set(payload.admin_address.clone()), // Story 2-3 AC#6
            assets: Set(asset_json),
            weights: Set(weights_json),
            created_at: Set(Some(Utc::now().into())),
            updated_at: Set(Some(Utc::now().into())),
            ..Default::default()
        };

        if let Err(e) = itp.insert(&state.db).await {
            warn!(
                correlation_id = %correlation_id,
                error = %e,
                "Failed to save ITP to database (creation still succeeded on-chain)"
            );
        } else {
            info!(
                correlation_id = %correlation_id,
                orbit_address = %result.orbit_address,
                "ITP saved to database"
            );
        }

        let response = CreateItpSyncResponse {
            tx_hash: result.tx_hash,
            nonce: result.nonce,
            orbit_address: result.orbit_address,
            arbitrum_address: result.arbitrum_address,
            status: "completed".to_string(),
        };

        Ok(Json(serde_json::to_value(response).unwrap()))
    } else {
        // Async mode: return immediately after transaction sent (using sanitized inputs)
        let result = service
            .request_create_itp(
                &sanitized_name,
                &sanitized_symbol,
                &sanitized_description,
                &sanitized_methodology,
                payload.initial_price,
                payload.max_order_size,
                asset_ids,
                weights,
            )
            .await
            .map_err(|e| {
                error!(
                    correlation_id = %correlation_id,
                    error = %e,
                    "ITP creation failed (async mode)"
                );
                map_creation_error(e)
            })?;

        info!(
            correlation_id = %correlation_id,
            tx_hash = %result.tx_hash,
            nonce = result.nonce,
            confirmed_at_block = result.confirmed_at_block,
            "ITP creation requested (async mode)"
        );

        let response = CreateItpResponse {
            tx_hash: result.tx_hash,
            nonce: result.nonce,
            confirmed_at_block: result.confirmed_at_block,
            estimated_completion_time: DEFAULT_COMPLETION_TIME,
            status: "pending".to_string(),
        };

        Ok(Json(serde_json::to_value(response).unwrap()))
    }
}

/// Check ITP creation status endpoint
///
/// GET /api/itp/status/:nonce?from_block=N
///
/// Checks the on-chain status of an ITP creation request.
/// This allows the frontend to poll for real progress instead of showing fake progress.
///
/// # Response
///
/// ```json
/// {
///   "nonce": 0,
///   "status": "pending" | "completed",
///   "orbit_address": "0x..." (only when completed),
///   "arbitrum_address": "0x..." (only when completed)
/// }
/// ```
pub async fn get_itp_status(
    headers: HeaderMap,
    axum::extract::Path(nonce): axum::extract::Path<u64>,
    axum::extract::Query(query): axum::extract::Query<crate::models::itp::ItpStatusQuery>,
) -> Result<Json<crate::models::itp::ItpStatusResponse>, (StatusCode, Json<ItpErrorResponse>)> {
    info!(nonce = nonce, from_block = query.from_block, "ITP status check request");

    // Check admin authentication
    let _api_key = check_admin_auth(&headers)?;

    // Get configuration from environment
    let rpc_url = std::env::var("ARB_RPC_URL").map_err(|_| {
        error!("ARB_RPC_URL not configured");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: "Server configuration error".to_string(),
                code: Some("CONFIG_ERROR".to_string()),
            }),
        )
    })?;

    let private_key = std::env::var("ARBITRUM_PRIVATE_KEY")
        .or_else(|_| std::env::var("DEPLOY_PRIVATE_KEY"))
        .map_err(|_| {
            error!("ARBITRUM_PRIVATE_KEY not configured");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ItpErrorResponse {
                    error: "Server configuration error".to_string(),
                    code: Some("CONFIG_ERROR".to_string()),
                }),
            )
        })?;

    let bridge_proxy_address = std::env::var("BRIDGE_PROXY_ADDRESS").map_err(|_| {
        error!("BRIDGE_PROXY_ADDRESS not configured");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: "Server configuration error".to_string(),
                code: Some("CONFIG_ERROR".to_string()),
            }),
        )
    })?;

    // Initialize service
    let service = ItpCreationService::new(&rpc_url, &private_key, &bridge_proxy_address)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to initialize ITP creation service");
            map_creation_error(e)
        })?;

    // Check status
    let (status, orbit_address, arbitrum_address) = service
        .check_itp_status(nonce, query.from_block)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to check ITP status");
            map_creation_error(e)
        })?;

    info!(
        nonce = nonce,
        status = %status,
        orbit_address = ?orbit_address,
        arbitrum_address = ?arbitrum_address,
        "ITP status check complete"
    );

    Ok(Json(crate::models::itp::ItpStatusResponse {
        nonce,
        status,
        orbit_address,
        arbitrum_address,
    }))
}

/// Check admin authentication via X-API-Key header
/// Returns the API key on success for use in per-key rate limiting
fn check_admin_auth(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ItpErrorResponse>)> {
    let admin_key = std::env::var("ADMIN_API_KEY").map_err(|_| {
        error!("ADMIN_API_KEY not configured");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: "Server configuration error".to_string(),
                code: Some("CONFIG_ERROR".to_string()),
            }),
        )
    })?;

    let provided_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided_key != admin_key {
        warn!("Invalid or missing API key");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ItpErrorResponse {
                error: "Invalid or missing API key".to_string(),
                code: Some("UNAUTHORIZED".to_string()),
            }),
        ));
    }

    Ok(provided_key.to_string())
}

/// Validate CreateItpRequest
fn validate_create_itp_request(
    req: &CreateItpRequest,
) -> Result<(), (StatusCode, Json<ItpErrorResponse>)> {
    // Validate name
    if req.name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpErrorResponse {
                error: "Token name cannot be empty".to_string(),
                code: Some("INVALID_NAME".to_string()),
            }),
        ));
    }

    if req.name.len() > MAX_NAME_LENGTH {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpErrorResponse {
                error: format!("Token name cannot exceed {} characters", MAX_NAME_LENGTH),
                code: Some("INVALID_NAME".to_string()),
            }),
        ));
    }

    // Name should be alphanumeric + spaces
    if !req.name.chars().all(|c| c.is_alphanumeric() || c == ' ') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpErrorResponse {
                error: "Token name must be alphanumeric (spaces allowed)".to_string(),
                code: Some("INVALID_NAME".to_string()),
            }),
        ));
    }

    // Validate symbol
    if req.symbol.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpErrorResponse {
                error: "Token symbol cannot be empty".to_string(),
                code: Some("INVALID_SYMBOL".to_string()),
            }),
        ));
    }

    if req.symbol.len() > MAX_SYMBOL_LENGTH {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpErrorResponse {
                error: format!("Token symbol cannot exceed {} characters", MAX_SYMBOL_LENGTH),
                code: Some("INVALID_SYMBOL".to_string()),
            }),
        ));
    }

    // Symbol should be uppercase alphanumeric
    if !req.symbol.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpErrorResponse {
                error: "Token symbol must be uppercase alphanumeric".to_string(),
                code: Some("INVALID_SYMBOL".to_string()),
            }),
        ));
    }

    // Validate initial price
    if req.initial_price == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpErrorResponse {
                error: "Initial price must be greater than 0".to_string(),
                code: Some("INVALID_PRICE".to_string()),
            }),
        ));
    }

    if req.initial_price > MAX_INITIAL_PRICE {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ItpErrorResponse {
                error: format!(
                    "Initial price cannot exceed {} (1 trillion USDC)",
                    MAX_INITIAL_PRICE
                ),
                code: Some("INVALID_PRICE".to_string()),
            }),
        ));
    }

    // Validate asset_ids and weights if provided
    if let Some(ref asset_ids) = req.asset_ids {
        if let Some(ref weights) = req.weights {
            if weights.len() != asset_ids.len() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ItpErrorResponse {
                        error: "Weights count must match asset_ids count".to_string(),
                        code: Some("INVALID_WEIGHTS".to_string()),
                    }),
                ));
            }

            // Weights should sum to 10000 (100% in basis points)
            let sum: u128 = weights.iter().sum();
            if sum != 10000 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ItpErrorResponse {
                        error: format!("Weights must sum to 10000 (100%), got {}", sum),
                        code: Some("INVALID_WEIGHTS".to_string()),
                    }),
                ));
            }
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ItpErrorResponse {
                    error: "Weights required when asset_ids provided".to_string(),
                    code: Some("INVALID_WEIGHTS".to_string()),
                }),
            ));
        }
    }

    Ok(())
}

/// Map ItpCreationError to HTTP response
fn map_creation_error(err: ItpCreationError) -> (StatusCode, Json<ItpErrorResponse>) {
    match err {
        ItpCreationError::ProviderError(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: format!("Bridge connection error: {}", msg),
                code: Some("PROVIDER_ERROR".to_string()),
            }),
        ),
        ItpCreationError::TransactionError(msg) => {
            // Check for common revert reasons
            let (error_msg, code) = if msg.contains("revert") {
                parse_revert_reason(&msg)
            } else {
                (format!("Transaction failed: {}", msg), "TX_ERROR".to_string())
            };

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ItpErrorResponse {
                    error: error_msg,
                    code: Some(code),
                }),
            )
        }
        ItpCreationError::GasEstimationError(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: format!("Gas estimation failed: {}", msg),
                code: Some("GAS_ERROR".to_string()),
            }),
        ),
        ItpCreationError::EventParsingError(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: format!("Event parsing failed: {}", msg),
                code: Some("EVENT_ERROR".to_string()),
            }),
        ),
        ItpCreationError::Timeout(msg) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ItpErrorResponse {
                error: format!("Operation timed out: {}", msg),
                code: Some("TIMEOUT".to_string()),
            }),
        ),
        ItpCreationError::InvalidConfig(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ItpErrorResponse {
                error: format!("Configuration error: {}", msg),
                code: Some("CONFIG_ERROR".to_string()),
            }),
        ),
    }
}

/// Parse contract revert reason to human-readable message
fn parse_revert_reason(msg: &str) -> (String, String) {
    if msg.contains("InvalidTokenName") {
        ("Token name cannot be empty".to_string(), "INVALID_NAME".to_string())
    } else if msg.contains("InvalidTokenSymbol") {
        ("Token symbol cannot be empty".to_string(), "INVALID_SYMBOL".to_string())
    } else if msg.contains("Ownable") || msg.contains("caller is not the owner") {
        (
            "Unauthorized: wallet is not BridgeProxy owner".to_string(),
            "UNAUTHORIZED_WALLET".to_string(),
        )
    } else if msg.contains("insufficient funds") {
        (
            "Insufficient gas funds in wallet".to_string(),
            "INSUFFICIENT_GAS".to_string(),
        )
    } else {
        (format!("Contract reverted: {}", msg), "CONTRACT_REVERT".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(name: &str, symbol: &str, initial_price: u64) -> CreateItpRequest {
        CreateItpRequest {
            name: name.to_string(),
            symbol: symbol.to_string(),
            description: None,
            methodology: None,
            initial_price,
            max_order_size: 1_000_000_000,
            asset_ids: None,
            weights: None,
            asset_composition: None,
            sync: false,
            admin_address: None, // Story 2-3 AC#6: Optional issuer address
        }
    }

    #[test]
    fn test_validate_name_empty() {
        let req = make_request("", "TEST", 1000000);
        let result = validate_create_itp_request(&req);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_validate_name_too_long() {
        let req = make_request(&"A".repeat(65), "TEST", 1000000);
        let result = validate_create_itp_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_symbol_empty() {
        let req = make_request("Test Index", "", 1000000);
        let result = validate_create_itp_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_symbol_lowercase() {
        let req = make_request("Test Index", "test", 1000000);
        let result = validate_create_itp_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_price_zero() {
        let req = make_request("Test Index", "TEST", 0);
        let result = validate_create_itp_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_price_too_high() {
        let req = make_request("Test Index", "TEST", MAX_INITIAL_PRICE + 1);
        let result = validate_create_itp_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_valid_request() {
        let req = make_request("Test Index", "TEST", 1000000);
        let result = validate_create_itp_request(&req);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_with_weights() {
        let mut req = make_request("Test Index", "TEST", 1000000);
        req.asset_ids = Some(vec![1, 2]);
        req.weights = Some(vec![5000, 5000]); // 50% each in basis points
        let result = validate_create_itp_request(&req);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_weights_mismatch() {
        let mut req = make_request("Test Index", "TEST", 1000000);
        req.asset_ids = Some(vec![1, 2]);
        req.weights = Some(vec![5000, 3000, 2000]); // 3 weights, 2 assets
        let result = validate_create_itp_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_weights_not_sum_to_10000() {
        let mut req = make_request("Test Index", "TEST", 1000000);
        req.asset_ids = Some(vec![1, 2]);
        req.weights = Some(vec![3000, 3000]); // 60%, not 100%
        let result = validate_create_itp_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_revert_reason_invalid_name() {
        let (msg, code) = parse_revert_reason("execution reverted: InvalidTokenName");
        assert_eq!(msg, "Token name cannot be empty");
        assert_eq!(code, "INVALID_NAME");
    }

    #[test]
    fn test_parse_revert_reason_ownable() {
        let (msg, code) = parse_revert_reason("Ownable: caller is not the owner");
        assert!(msg.contains("Unauthorized"));
        assert_eq!(code, "UNAUTHORIZED_WALLET");
    }

    #[test]
    fn test_map_creation_error_timeout() {
        let err = ItpCreationError::Timeout("test".to_string());
        let (status, _) = map_creation_error(err);
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    }

    #[test]
    fn test_sanitize_input_removes_control_chars() {
        let input = "Test\x00Name\x1FHere";
        let sanitized = sanitize_input(input);
        assert_eq!(sanitized, "TestNameHere");
    }

    #[test]
    fn test_sanitize_input_normalizes_whitespace() {
        let input = "  Test   Multiple   Spaces  ";
        let sanitized = sanitize_input(input);
        assert_eq!(sanitized, "Test Multiple Spaces");
    }

    #[test]
    fn test_sanitize_input_removes_zero_width_chars() {
        let input = "Test\u{200B}Name\u{FEFF}Here";
        let sanitized = sanitize_input(input);
        assert_eq!(sanitized, "TestNameHere");
    }

    #[test]
    fn test_sanitize_input_preserves_valid_input() {
        let input = "Top 10 DeFi Index";
        let sanitized = sanitize_input(input);
        assert_eq!(sanitized, "Top 10 DeFi Index");
    }
}
