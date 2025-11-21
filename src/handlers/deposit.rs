use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rust_decimal::Decimal;
use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect};
use std::collections::HashMap;

use crate::entities::{blockchain_events, daily_prices, index_metadata, prelude::*};
use crate::models::deposit::{
    DepositTransactionAll, DepositTransactionResponse, DepositTransactionSingle,
};
use crate::models::token::ErrorResponse;
use crate::AppState;

const USDC_DECIMALS: u32 = 6;
const INDEX_DECIMALS: u32 = 30;
const NETWORK: &str = "base";

pub async fn get_deposit_transaction_data(
    State(state): State<AppState>,
    Path((index_id, address)): Path<(i32, String)>,
) -> Result<Json<DepositTransactionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let address_filter = if address.is_empty() || address == "0x0000" {
        None
    } else {
        Some(address.to_lowercase())
    };

    // ------- SINGLE-INDEX MODE -------
    if index_id != -1 {
        let result = get_single_index_deposits(&state, index_id, address_filter).await?;
        return Ok(Json(DepositTransactionResponse::Single(result)));
    }

    // ------- ALL-INDEXES MODE -------
    let result = get_all_indexes_deposits(&state, address_filter).await?;
    Ok(Json(DepositTransactionResponse::All(result)))
}

async fn get_single_index_deposits(
    state: &AppState,
    index_id: i32,
    address_filter: Option<String>,
) -> Result<Vec<DepositTransactionSingle>, (StatusCode, Json<ErrorResponse>)> {
    // Get index metadata from database
    let index_data = IndexMetadata::find_by_id(index_id)
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Index {} not found", index_id),
                }),
            )
        })?;

    let index_address = index_data.address.to_lowercase();

    // Get latest price
    let index_price = get_latest_price(state, index_id).await?;

    // Get all mint events for this index
    let all_rows = BlockchainEvents::find()
        .filter(blockchain_events::Column::ContractAddress.eq(&index_address))
        .filter(blockchain_events::Column::Network.eq(NETWORK))
        .filter(blockchain_events::Column::EventType.eq("mint"))
        .all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    // Calculate totals
    let mut total_amount_base = Decimal::ZERO;
    let mut total_qty_base = Decimal::ZERO;

    for ev in &all_rows {
        total_amount_base += to_bigint_units(ev.amount.unwrap_or(Decimal::ZERO), USDC_DECIMALS);
        total_qty_base += to_bigint_units(ev.quantity.unwrap_or(Decimal::ZERO), INDEX_DECIMALS);
    }

    // Filter by address if provided
    let rows: Vec<_> = if let Some(ref addr) = address_filter {
        all_rows
            .into_iter()
            .filter(|r| {
                r.user_address
                    .as_ref()
                    .map(|ua| ua.to_lowercase() == *addr)
                    .unwrap_or(false)
            })
            .collect()
    } else {
        all_rows
    };

    // Group by user
    let mut grouped: HashMap<String, GroupedDeposit> = HashMap::new();

    for event in rows {
        let user = event
            .user_address
            .as_ref()
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        let key = if address_filter.is_some() {
            index_id.to_string()
        } else {
            user.clone()
        };

        let amount_base = to_bigint_units(event.amount.unwrap_or(Decimal::ZERO), USDC_DECIMALS);
        let qty_base = to_bigint_units(event.quantity.unwrap_or(Decimal::ZERO), INDEX_DECIMALS);

        let supply = format_units(amount_base, USDC_DECIMALS);
        let quantity = format_units(qty_base, INDEX_DECIMALS);

        grouped
            .entry(key)
            .and_modify(|g| {
                g.deposit_count += 1;
                g.supply += supply;
                g.supply_value_usd += supply;
                g.quantity += quantity;
            })
            .or_insert_with(|| GroupedDeposit {
                user: if address_filter.is_some() {
                    None
                } else {
                    Some(user)
                },
                deposit_count: 1,
                supply,
                supply_value_usd: supply,
                quantity,
            });
    }

    let total_supply_number = format_units(total_amount_base, USDC_DECIMALS);
    let total_quantity_number = format_units(total_qty_base, INDEX_DECIMALS);

    let result: Vec<DepositTransactionSingle> = grouped
        .into_values()
        .map(|g| {
            let share = if total_supply_number > 0.0 {
                (g.supply / total_supply_number) * 100.0
            } else {
                0.0
            };
            let raw_share = if total_supply_number > 0.0 {
                g.supply / total_supply_number
            } else {
                0.0
            };

            DepositTransactionSingle {
                index_id,
                index_name: index_data.name.clone(),
                index_symbol: index_data.symbol.clone(),
                user: g.user,
                total_supply: format!("{:.2}", total_supply_number),
                total_quantity: total_quantity_number.to_string(),
                supply_value_usd: g.supply_value_usd,
                deposit_count: g.deposit_count,
                supply: format!("{:.2}", g.supply),
                quantity: g.quantity.to_string(),
                currency: "USDC".to_string(),
                share,
                raw_share,
                index_price,
            }
        })
        .collect();

    Ok(result)
}

async fn get_all_indexes_deposits(
    state: &AppState,
    address_filter: Option<String>,
) -> Result<Vec<DepositTransactionAll>, (StatusCode, Json<ErrorResponse>)> {
    // Load all indexes from database
    let all_indexes = IndexMetadata::find().all(&state.db).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Database error: {}", e),
            }),
        )
    })?;

    let by_addr: HashMap<String, _> = all_indexes
        .into_iter()
        .map(|idx| (idx.address.to_lowercase(), idx))
        .collect();

    // Get all mint events
    let all_rows = BlockchainEvents::find()
        .filter(blockchain_events::Column::Network.eq(NETWORK))
        .filter(blockchain_events::Column::EventType.eq("mint"))
        .all(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    // Filter by user if provided
    let filtered_rows: Vec<_> = if let Some(ref addr) = address_filter {
        all_rows
            .iter()
            .filter(|r| {
                r.user_address
                    .as_ref()
                    .map(|ua| ua.to_lowercase() == *addr)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    } else {
        all_rows.clone()
    };

    // Calculate overall totals per contract
    let mut totals_by_contract: HashMap<String, ContractTotals> = HashMap::new();
    for ev in &all_rows {
        let contract = ev.contract_address.to_lowercase();
        let entry = totals_by_contract.entry(contract).or_default();
        entry.amount_base += to_bigint_units(ev.amount.unwrap_or(Decimal::ZERO), USDC_DECIMALS);
        entry.qty_base += to_bigint_units(ev.quantity.unwrap_or(Decimal::ZERO), INDEX_DECIMALS);
    }

    // Group filtered rows by contract
    let mut by_contract_filtered: HashMap<String, FilteredContractGroup> = HashMap::new();
    for ev in filtered_rows {
        let contract = ev.contract_address.to_lowercase();
        let entry = by_contract_filtered.entry(contract).or_default();
        entry.amount_base += to_bigint_units(ev.amount.unwrap_or(Decimal::ZERO), USDC_DECIMALS);
        entry.qty_base += to_bigint_units(ev.quantity.unwrap_or(Decimal::ZERO), INDEX_DECIMALS);
        entry.count += 1;
    }

    // Pre-fetch latest prices
    let mut price_map: HashMap<i32, Option<f64>> = HashMap::new();
    for contract_addr in by_contract_filtered.keys() {
        if let Some(meta) = by_addr.get(contract_addr) {
            price_map.insert(meta.index_id, get_latest_price(state, meta.index_id).await?);
        }
    }

    // Build result
    let mut result = Vec::new();
    for (contract_addr, group) in by_contract_filtered {
        if let Some(meta) = by_addr.get(&contract_addr) {
            let overall_totals = totals_by_contract.get(&contract_addr).cloned().unwrap_or_default();

            let total_supply = format_units(overall_totals.amount_base, USDC_DECIMALS);
            let total_quantity = format_units(overall_totals.qty_base, INDEX_DECIMALS);

            let group_supply = format_units(group.amount_base, USDC_DECIMALS);
            let group_quantity = format_units(group.qty_base, INDEX_DECIMALS);

            let share = if total_supply > 0.0 {
                (group_supply / total_supply) * 100.0
            } else {
                0.0
            };
            let share_pct = if total_supply > 0.0 {
                group_supply / total_supply
            } else {
                0.0
            };

            let index_price = price_map.get(&meta.index_id).and_then(|p| *p);

            result.push(DepositTransactionAll {
                index_id: meta.index_id,
                name: meta.name.clone(),
                symbol: meta.symbol.clone(),
                address: contract_addr.clone(),
                user: address_filter.clone(),
                total_supply: format!("{:.2}", total_supply),
                balance_raw: total_quantity.to_string(),
                deposit_count: group.count,
                supply: format!("{:.2}", group_supply),
                quantity: group_quantity.to_string(),
                currency: "USDC".to_string(),
                share,
                decimals: INDEX_DECIMALS,
                share_pct,
                usd_price: index_price,
            });
        }
    }

    Ok(result)
}

async fn get_latest_price(
    state: &AppState,
    index_id: i32,
) -> Result<Option<f64>, (StatusCode, Json<ErrorResponse>)> {
    let price_row = DailyPrices::find()
        .filter(daily_prices::Column::IndexId.eq(index_id.to_string()))
        .order_by(daily_prices::Column::Date, Order::Desc)
        .limit(1)
        .one(&state.db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Database error: {}", e),
                }),
            )
        })?;

    Ok(price_row.and_then(|row| row.price.to_string().parse::<f64>().ok()))
}

fn to_bigint_units(value: Decimal, decimals: u32) -> Decimal {
    let multiplier = Decimal::from(10_u64.pow(decimals));
    value * multiplier
}

fn format_units(value: Decimal, decimals: u32) -> f64 {
    let divisor = Decimal::from(10_u64.pow(decimals));
    let result = value / divisor;
    result.to_string().parse::<f64>().unwrap_or(0.0)
}

#[derive(Default, Clone)]
struct ContractTotals {
    amount_base: Decimal,
    qty_base: Decimal,
}

#[derive(Default)]
struct FilteredContractGroup {
    amount_base: Decimal,
    qty_base: Decimal,
    count: i32,
}

struct GroupedDeposit {
    user: Option<String>,
    deposit_count: i32,
    supply: f64,
    supply_value_usd: f64,
    quantity: f64,
}
