//! WebSocket handler for real-time operation status streaming
//!
//! Story 3.2 - AC #6, NFR2: Frontend receives status updates within 3 seconds
//!
//! Provides `/api/operations/ws` endpoint for clients to subscribe to operation status updates.
//! Clients subscribe with their wallet address and receive status events for their operations.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
    Json,
};
use futures_util::{SinkExt, StreamExt};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::entities::operations;
use crate::models::operation::{OperationEvent, OperationResponse, OperationStatus, OperationType};
use crate::AppState;

/// Shared state for operation broadcasting
#[derive(Clone)]
pub struct OperationBroadcaster {
    tx: broadcast::Sender<OperationEvent>,
}

impl OperationBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1000);
        Self { tx }
    }

    /// Broadcast an operation event to all subscribers
    pub fn broadcast(&self, event: OperationEvent) {
        // Ignore errors if no subscribers
        let _ = self.tx.send(event);
    }

    /// Subscribe to operation events
    pub fn subscribe(&self) -> broadcast::Receiver<OperationEvent> {
        self.tx.subscribe()
    }
}

impl Default for OperationBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// WebSocket subscription request from client
#[derive(Debug, Clone, Deserialize)]
pub struct WsSubscribeRequest {
    /// Action type (subscribe, unsubscribe, ping)
    pub action: String,
    /// Wallet address to filter operations (required for subscribe)
    pub address: Option<String>,
}

/// WebSocket message to client
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Subscription confirmed
    #[serde(rename = "subscribed")]
    Subscribed { address: String },
    /// Operation status update
    #[serde(rename = "operation")]
    Operation(OperationEvent),
    /// Error message
    #[serde(rename = "error")]
    Error { message: String },
    /// Pong response
    #[serde(rename = "pong")]
    Pong,
    /// Initial state with pending operations
    #[serde(rename = "initial")]
    Initial { operations: Vec<OperationResponse> },
}

/// Query parameters for operations endpoint
#[derive(Debug, Deserialize)]
pub struct OperationsQuery {
    pub user: String,
}

/// GET /api/operations - Get operations for a user
pub async fn get_operations(
    State(state): State<AppState>,
    Query(query): Query<OperationsQuery>,
) -> impl IntoResponse {
    let user_address = query.user.to_lowercase();

    match operations::Entity::find()
        .filter(operations::Column::UserAddress.eq(&user_address))
        .order_by_desc(operations::Column::CreatedAt)
        .all(&state.db)
        .await
    {
        Ok(ops) => {
            let response: Vec<OperationResponse> = ops.into_iter().map(Into::into).collect();
            Json(serde_json::json!({
                "success": true,
                "operations": response,
            }))
        }
        Err(e) => {
            error!("Failed to fetch operations: {}", e);
            Json(serde_json::json!({
                "success": false,
                "error": format!("Database error: {}", e),
            }))
        }
    }
}

/// GET /api/operations/ws - WebSocket endpoint for operation status streaming
///
/// Client sends subscription request:
/// ```json
/// {
///   "action": "subscribe",
///   "address": "0x1234..."
/// }
/// ```
///
/// Server streams operation events:
/// ```json
/// {
///   "type": "operation",
///   "operation_type": "buy",
///   "nonce": 123,
///   "user": "0x1234...",
///   "status": "bridging",
///   "timestamp": 1234567890
/// }
/// ```
pub async fn operations_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    info!("New operations WebSocket connection");

    // Wait for subscription request
    let address = match wait_for_subscription(&mut receiver).await {
        Ok(addr) => addr,
        Err(e) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&WsMessage::Error {
                        message: e.to_string(),
                    })
                    .unwrap()
                    .into(),
                ))
                .await;
            return;
        }
    };

    let address_lower = address.to_lowercase();
    info!("Operations subscription for address: {}", address_lower);

    // Send subscription confirmation
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&WsMessage::Subscribed {
                address: address_lower.clone(),
            })
            .unwrap()
            .into(),
        ))
        .await;

    // Send initial state - pending operations for this user
    if let Ok(pending_ops) = operations::Entity::find()
        .filter(operations::Column::UserAddress.eq(&address_lower))
        .filter(
            operations::Column::Status
                .ne("complete")
                .and(operations::Column::Status.ne("failed"))
                .and(operations::Column::Status.ne("refunded")),
        )
        .order_by_desc(operations::Column::CreatedAt)
        .all(&state.db)
        .await
    {
        let response: Vec<OperationResponse> = pending_ops.into_iter().map(Into::into).collect();
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&WsMessage::Initial {
                    operations: response,
                })
                .unwrap()
                .into(),
            ))
            .await;
    }

    // Subscribe to broadcast channel
    let mut broadcast_rx = state.operation_broadcaster.subscribe();

    // Heartbeat interval
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            // Handle broadcast events
            result = broadcast_rx.recv() => {
                match result {
                    Ok(event) => {
                        // Only forward events for this user (case-insensitive)
                        if event.user.to_lowercase() == address_lower {
                            let msg = WsMessage::Operation(event);
                            if let Err(e) = sender.send(Message::Text(
                                serde_json::to_string(&msg).unwrap().into()
                            )).await {
                                debug!("WebSocket send error: {}", e);
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Missed {} broadcast events", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Broadcast channel closed");
                        break;
                    }
                }
            }

            // Handle heartbeat
            _ = heartbeat.tick() => {
                if let Err(e) = sender.send(Message::Ping(axum::body::Bytes::new())).await {
                    debug!("Heartbeat failed: {}", e);
                    break;
                }
            }

            // Handle incoming messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(req) = serde_json::from_str::<WsSubscribeRequest>(&text) {
                            match req.action.as_str() {
                                "ping" => {
                                    let _ = sender.send(Message::Text(
                                        serde_json::to_string(&WsMessage::Pong).unwrap().into()
                                    )).await;
                                }
                                "unsubscribe" => {
                                    info!("Client unsubscribed");
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("WebSocket closed by client");
                        break;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket receive error: {}", e);
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("Operations WebSocket connection closed for {}", address_lower);
}

async fn wait_for_subscription(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Wait up to 30 seconds for subscription request
    let timeout = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let req: WsSubscribeRequest = serde_json::from_str(&text)?;

                    if req.action != "subscribe" {
                        return Err("First message must be subscribe action".into());
                    }

                    let address = req
                        .address
                        .ok_or("address is required for subscription")?;

                    // Validate address format (basic check)
                    if !address.starts_with("0x") || address.len() != 42 {
                        return Err("Invalid address format".into());
                    }

                    return Ok(address);
                }
                Ok(Message::Ping(_)) => {
                    continue;
                }
                Ok(Message::Close(_)) => {
                    return Err("Connection closed before subscription".into());
                }
                Err(e) => {
                    return Err(format!("WebSocket error: {}", e).into());
                }
                _ => continue,
            }
        }
        Err("Connection ended before subscription".into())
    });

    timeout.await.map_err(|_| "Subscription timeout")?
}

/// POST /api/operations/update - Receive operation updates from bridge-node
///
/// This endpoint is called by the bridge-node to report operation status changes.
/// It persists the status and broadcasts to connected WebSocket clients.
#[derive(Debug, Deserialize)]
pub struct UpdateOperationRequest {
    pub user_address: String,
    pub operation_type: String,
    pub nonce: u64,
    pub status: String,
    pub arb_tx_hash: Option<String>,
    pub orbit_tx_hash: Option<String>,
    pub completion_tx_hash: Option<String>,
    pub amount: Option<String>,
    pub itp_amount: Option<String>,
    pub itp_address: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub retryable: Option<bool>,
}

pub async fn update_operation(
    State(state): State<AppState>,
    Json(req): Json<UpdateOperationRequest>,
) -> impl IntoResponse {
    use sea_orm::{ActiveModelTrait, Set};

    let user_address = req.user_address.to_lowercase();

    // Check if operation exists
    let existing = operations::Entity::find()
        .filter(operations::Column::UserAddress.eq(&user_address))
        .filter(operations::Column::Nonce.eq(req.nonce as i64))
        .filter(operations::Column::OperationType.eq(&req.operation_type))
        .one(&state.db)
        .await;

    let result = match existing {
        Ok(Some(op)) => {
            // Update existing operation
            let mut active: operations::ActiveModel = op.into();
            active.status = Set(req.status.clone());
            if let Some(ref tx) = req.arb_tx_hash {
                active.arb_tx_hash = Set(Some(tx.clone()));
            }
            if let Some(ref tx) = req.orbit_tx_hash {
                active.orbit_tx_hash = Set(Some(tx.clone()));
            }
            if let Some(ref tx) = req.completion_tx_hash {
                active.completion_tx_hash = Set(Some(tx.clone()));
            }
            if let Some(ref amt) = req.itp_amount {
                active.itp_amount = Set(Some(amt.clone()));
            }
            if let Some(ref code) = req.error_code {
                active.error_code = Set(Some(code.clone()));
            }
            if let Some(ref msg) = req.error_message {
                active.error_message = Set(Some(msg.clone()));
            }
            if let Some(retryable) = req.retryable {
                active.retryable = Set(retryable);
            }
            active.updated_at = Set(chrono::Utc::now().into());
            active.update(&state.db).await
        }
        Ok(None) => {
            // Create new operation
            let active = operations::ActiveModel {
                user_address: Set(user_address.clone()),
                operation_type: Set(req.operation_type.clone()),
                nonce: Set(req.nonce as i64),
                status: Set(req.status.clone()),
                arb_tx_hash: Set(req.arb_tx_hash.clone()),
                orbit_tx_hash: Set(req.orbit_tx_hash.clone()),
                completion_tx_hash: Set(req.completion_tx_hash.clone()),
                amount: Set(req.amount.clone()),
                itp_amount: Set(req.itp_amount.clone()),
                itp_address: Set(req.itp_address.clone()),
                error_code: Set(req.error_code.clone()),
                error_message: Set(req.error_message.clone()),
                retryable: Set(req.retryable.unwrap_or(false)),
                ..Default::default()
            };
            active.insert(&state.db).await
        }
        Err(e) => {
            error!("Database error checking operation: {}", e);
            return Json(serde_json::json!({
                "success": false,
                "error": format!("Database error: {}", e),
            }));
        }
    };

    match result {
        Ok(_) => {
            // Broadcast event to WebSocket clients
            let op_type = req
                .operation_type
                .parse::<OperationType>()
                .unwrap_or(OperationType::Buy);
            let op_status = req
                .status
                .parse::<OperationStatus>()
                .unwrap_or(OperationStatus::Initiated);

            let event = OperationEvent {
                operation_type: op_type,
                nonce: req.nonce,
                user: user_address,
                status: op_status,
                phase: None,
                tx_hash: req.arb_tx_hash.clone(),
                arb_tx_hash: req.arb_tx_hash,
                orbit_tx_hash: req.orbit_tx_hash,
                error: req.error_code.as_ref().map(|code| {
                    crate::models::operation::OperationError {
                        code: code.clone(),
                        message: req.error_message.clone().unwrap_or_default(),
                        retryable: req.retryable.unwrap_or(false),
                    }
                }),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            };

            state.operation_broadcaster.broadcast(event);

            Json(serde_json::json!({
                "success": true,
            }))
        }
        Err(e) => {
            error!("Failed to save operation: {}", e);
            Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to save: {}", e),
            }))
        }
    }
}
