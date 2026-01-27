//! ITP Chain Discovery Sync Job
//!
//! Periodically scans on-chain events to discover ITPs that exist on
//! Arbitrum+Orbit but are not yet in the backend database.
//! This handles ITPs created via the bridge node (which doesn't write to the DB).

use alloy::{
    network::EthereumWallet,
    primitives::Address,
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    sol,
};
use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{interval, Duration as TokioDuration};
use tracing::{debug, error, info, warn};

use asset_registry::AssetRegistry;
use crate::entities::{itps, prelude::Itps};
use crate::services::itp_chain_discovery::{DiscoveredItp, ItpChainDiscoveryService};

// Castle interface for voting and quote updates
sol! {
    #[sol(rpc)]
    interface ICastleVote {
        function submitVote(uint128 indexId, bytes calldata data) external;
        function updateIndexQuote(uint128 vendorId, uint128 indexId) external;
    }
}

/// Orbit chain voter - submits votes and updates index quotes for discovered ITPs
struct OrbitVoter {
    castle_address: Address,
    wallet: EthereumWallet,
    orbit_rpc: String,
    vendor_id: u128,
}

impl OrbitVoter {
    /// Try to create from environment variables. Returns None if not configured.
    fn from_env(castle_address: &str) -> Option<Self> {
        let orbit_rpc = env::var("ORBIT_RPC_URL")
            .or_else(|_| env::var("TESTNET_RPC"))
            .ok()?;

        let private_key_str = env::var("ORBIT_PRIVATE_KEY")
            .or_else(|_| env::var("TESTNET_PRIVATE_KEY"))
            .or_else(|_| env::var("DEPLOY_PRIVATE_KEY"))
            .ok()?;

        let signer: PrivateKeySigner = private_key_str.parse().ok()?;
        let wallet = EthereumWallet::from(signer);

        let castle = Address::from_str(castle_address).ok()?;

        let vendor_id: u128 = env::var("VENDOR_ID")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        Some(Self {
            castle_address: castle,
            wallet,
            orbit_rpc,
            vendor_id,
        })
    }

    /// Vote and update quote for an ITP
    async fn vote_and_update_quote(&self, index_id: u128) {
        let provider = match ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(self.wallet.clone())
            .on_http(match self.orbit_rpc.parse() {
                Ok(url) => url,
                Err(e) => {
                    warn!(error = %e, "Failed to parse Orbit RPC URL for voting");
                    return;
                }
            }) {
            provider => provider,
        };

        let castle = ICastleVote::new(self.castle_address, &provider);

        // Submit vote
        match castle.submitVote(index_id, alloy::primitives::Bytes::new()).send().await {
            Ok(pending) => {
                match pending.get_receipt().await {
                    Ok(receipt) => {
                        if receipt.status() {
                            info!(index_id = index_id, "Backend auto-vote: submitVote confirmed");
                        } else {
                            debug!(index_id = index_id, "Backend auto-vote: submitVote reverted (may be already voted)");
                        }
                    }
                    Err(e) => {
                        debug!(index_id = index_id, error = %e, "Backend auto-vote: submitVote receipt error");
                    }
                }
            }
            Err(e) => {
                debug!(index_id = index_id, error = %e, "Backend auto-vote: submitVote send failed (may be already voted)");
            }
        }

        // Update quote
        match castle.updateIndexQuote(self.vendor_id, index_id).send().await {
            Ok(pending) => {
                match pending.get_receipt().await {
                    Ok(receipt) => {
                        if receipt.status() {
                            info!(index_id = index_id, "Backend auto-vote: updateIndexQuote confirmed");
                        } else {
                            debug!(index_id = index_id, "Backend auto-vote: updateIndexQuote reverted");
                        }
                    }
                    Err(e) => {
                        debug!(index_id = index_id, error = %e, "Backend auto-vote: updateIndexQuote receipt error");
                    }
                }
            }
            Err(e) => {
                debug!(index_id = index_id, error = %e, "Backend auto-vote: updateIndexQuote send failed");
            }
        }
    }
}

/// Default sync interval in seconds (60 seconds)
const DEFAULT_SYNC_INTERVAL_SECS: u64 = 60;

/// Environment variable names
const ENV_ARB_RPC_URL: &str = "ARB_RPC_URL";
const ENV_ORBIT_RPC_URL: &str = "ORBIT_RPC_URL";
const ENV_BRIDGE_PROXY_ADDRESS: &str = "BRIDGE_PROXY_ADDRESS";
const ENV_CASTLE_ADDRESS: &str = "CASTLE_ADDRESS";
const ENV_CONTRACT_DEPLOY_BLOCK: &str = "CONTRACT_DEPLOY_BLOCK";
const ENV_SYNC_INTERVAL: &str = "ITP_DISCOVERY_INTERVAL_SECS";

/// Start the ITP chain discovery sync job
///
/// Scans `ItpCreated` events on Arbitrum BridgeProxy and populates
/// the `itps` database table with discovered ITPs.
pub async fn start_itp_chain_discovery_job(
    db: DatabaseConnection,
    asset_registry: Arc<AssetRegistry>,
) {
    tokio::spawn(async move {
        // Load configuration
        let arb_rpc = match env::var(ENV_ARB_RPC_URL) {
            Ok(url) => url,
            Err(_) => {
                warn!("ARB_RPC_URL not set - ITP chain discovery disabled");
                return;
            }
        };

        let orbit_rpc = match env::var(ENV_ORBIT_RPC_URL) {
            Ok(url) => url,
            Err(_) => {
                warn!("ORBIT_RPC_URL not set - ITP chain discovery disabled");
                return;
            }
        };

        let bridge_proxy = match env::var(ENV_BRIDGE_PROXY_ADDRESS) {
            Ok(addr) => addr,
            Err(_) => {
                warn!("BRIDGE_PROXY_ADDRESS not set - ITP chain discovery disabled");
                return;
            }
        };

        let castle_address = match env::var(ENV_CASTLE_ADDRESS) {
            Ok(addr) => addr,
            Err(_) => {
                warn!("CASTLE_ADDRESS not set - ITP chain discovery disabled");
                return;
            }
        };

        let start_block: u64 = env::var(ENV_CONTRACT_DEPLOY_BLOCK)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(425242000); // Default to BridgeProxy deploy block

        let sync_interval_secs: u64 = env::var(ENV_SYNC_INTERVAL)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SYNC_INTERVAL_SECS);

        info!(
            arb_rpc = %arb_rpc,
            bridge_proxy = %bridge_proxy,
            castle_address = %castle_address,
            start_block = start_block,
            sync_interval_secs = sync_interval_secs,
            "Initializing ITP chain discovery job"
        );

        // Initialize discovery service
        let service = match ItpChainDiscoveryService::new(
            &arb_rpc,
            &orbit_rpc,
            &bridge_proxy,
            &castle_address,
            asset_registry,
            start_block,
        ).await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "Failed to initialize ITP chain discovery service");
                return;
            }
        };

        // Initialize auto-voter for Orbit chain (optional - only if private key configured)
        let orbit_voter = OrbitVoter::from_env(&castle_address);
        if orbit_voter.is_some() {
            info!("Backend auto-vote enabled for Orbit chain");
        } else {
            info!("Backend auto-vote disabled (no ORBIT_PRIVATE_KEY / TESTNET_PRIVATE_KEY)");
        }

        info!("ITP chain discovery job started");

        let mut interval = interval(TokioDuration::from_secs(sync_interval_secs));

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("Shutdown signal received, stopping ITP chain discovery job");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = run_discovery_cycle(&db, &service, orbit_voter.as_ref()).await {
                        error!(error = %e, "ITP chain discovery cycle failed");
                    }
                }
            }
        }

        info!("ITP chain discovery job stopped");
    });
}

/// Run a single discovery cycle
async fn run_discovery_cycle(
    db: &DatabaseConnection,
    service: &ItpChainDiscoveryService,
    orbit_voter: Option<&OrbitVoter>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Starting ITP chain discovery cycle");

    let discovered = service.discover_all_itps().await.map_err(|e| {
        format!("Discovery failed: {}", e)
    })?;

    if discovered.is_empty() {
        info!("No ITPs found on-chain");
        return Ok(());
    }

    let mut inserted = 0;
    let mut updated = 0;
    let mut skipped = 0;

    for itp in &discovered {
        // Check if this ITP already exists in the database (by orbit_address)
        let existing = Itps::find()
            .filter(itps::Column::OrbitAddress.eq(&itp.orbit_address))
            .one(db)
            .await?;

        if let Some(existing_model) = existing {
            // Already exists - update total_supply if changed, and backfill admin_address if missing
            let new_supply = Decimal::from_str(&itp.total_supply).unwrap_or(Decimal::ZERO);
            let old_supply = existing_model.total_supply.unwrap_or(Decimal::ZERO);
            let needs_admin = existing_model.admin_address.is_none() && itp.admin_address.is_some();

            if new_supply != old_supply || needs_admin {
                let mut active: itps::ActiveModel = existing_model.into();
                if new_supply != old_supply {
                    active.total_supply = Set(Some(new_supply));
                }
                if needs_admin {
                    active.admin_address = Set(itp.admin_address.clone());
                    info!(
                        orbit = %itp.orbit_address,
                        admin = ?itp.admin_address,
                        "Backfilled admin_address for existing ITP"
                    );
                }
                active.updated_at = Set(Some(Utc::now().into()));
                active.update(db).await?;
                updated += 1;
                if new_supply != old_supply {
                    info!(
                        orbit = %itp.orbit_address,
                        symbol = %itp.symbol,
                        old_supply = %old_supply,
                        new_supply = %new_supply,
                        "Updated ITP total_supply"
                    );
                }
            } else {
                skipped += 1;
            }
        } else {
            // New ITP - insert into database
            insert_discovered_itp(db, itp).await?;
            inserted += 1;
            info!(
                orbit = %itp.orbit_address,
                arbitrum = %itp.arbitrum_address,
                name = %itp.name,
                symbol = %itp.symbol,
                "Inserted new ITP from chain discovery"
            );

            // Auto-vote for newly discovered ITP
            if let Some(voter) = orbit_voter {
                if itp.index_id > 0 {
                    info!(index_id = itp.index_id, "Auto-voting for newly discovered ITP");
                    voter.vote_and_update_quote(itp.index_id).await;
                }
            }
        }
    }

    // Auto-vote for all discovered ITPs (idempotent - safe to vote again)
    if let Some(voter) = orbit_voter {
        let votable: Vec<&DiscoveredItp> = discovered.iter()
            .filter(|itp| itp.index_id > 0)
            .collect();
        if !votable.is_empty() {
            info!("🗳️ Backend auto-vote: updating quotes for {} ITPs", votable.len());
            for itp in &votable {
                voter.vote_and_update_quote(itp.index_id).await;
            }
        }
    }

    info!(
        total = discovered.len(),
        inserted = inserted,
        updated = updated,
        skipped = skipped,
        "ITP chain discovery cycle complete"
    );

    Ok(())
}

/// Insert a newly discovered ITP into the database
async fn insert_discovered_itp(
    db: &DatabaseConnection,
    itp: &DiscoveredItp,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Convert initial_price to Decimal (already in 18 decimals)
    let initial_price = Decimal::from(itp.initial_price_18);
    let total_supply = Decimal::from_str(&itp.total_supply).unwrap_or(Decimal::ZERO);

    // Build JSON for assets and weights
    let assets_json = if !itp.assets.is_empty() {
        Some(serde_json::json!(itp.assets))
    } else {
        None
    };

    let weights_json = if !itp.weights.is_empty() {
        Some(serde_json::json!(itp.weights))
    } else {
        None
    };

    let model = itps::ActiveModel {
        orbit_address: Set(itp.orbit_address.clone()),
        arbitrum_address: Set(Some(itp.arbitrum_address.clone())),
        index_id: Set(Some(itp.index_id as i64)),
        name: Set(itp.name.clone()),
        symbol: Set(itp.symbol.clone()),
        description: Set(Some(itp.description.clone())),
        methodology: Set(Some(itp.methodology.clone())),
        initial_price: Set(Some(initial_price)),
        current_price: Set(Some(initial_price)), // Start with initial price
        total_supply: Set(Some(total_supply)),
        state: Set(1), // Active
        deploy_tx_hash: Set(Some(itp.tx_hash.clone())),
        admin_address: Set(itp.admin_address.clone()),
        assets: Set(assets_json),
        weights: Set(weights_json),
        created_at: Set(Some(Utc::now().into())),
        updated_at: Set(Some(Utc::now().into())),
        ..Default::default()
    };

    model.insert(db).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(DEFAULT_SYNC_INTERVAL_SECS, 60);
    }

    #[test]
    fn test_env_var_names() {
        assert_eq!(ENV_ARB_RPC_URL, "ARB_RPC_URL");
        assert_eq!(ENV_ORBIT_RPC_URL, "ORBIT_RPC_URL");
        assert_eq!(ENV_BRIDGE_PROXY_ADDRESS, "BRIDGE_PROXY_ADDRESS");
        assert_eq!(ENV_CASTLE_ADDRESS, "CASTLE_ADDRESS");
    }
}
