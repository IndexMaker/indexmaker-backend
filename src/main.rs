use axum::{extract::State, routing::get, Json, Router};
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;
use serde::Serialize;
use std::env;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod entities;

#[derive(Clone)]
struct AppState {
    db: DatabaseConnection,
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,indexmaker_backend=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Connect to database
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    tracing::info!("Connecting to database...");
    let db = Database::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    tracing::info!("Running migrations...");
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    let state = AppState { db };

    // Build router
    let app = Router::new()
        .route("/", get(hello_indexmaker))
        .route("/indexes", get(get_index_list))
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

    tracing::info!("Server listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.unwrap();
}

async fn hello_indexmaker() -> &'static str {
    "Hello from IndexMaker Backend! 🚀"
}

#[derive(Serialize)]
struct IndexListResponse {
    indexes: Vec<String>,
}

async fn get_index_list(State(_state): State<AppState>) -> Json<IndexListResponse> {
    // TODO: Implement actual query
    Json(IndexListResponse {
        indexes: vec![],
    })
}
