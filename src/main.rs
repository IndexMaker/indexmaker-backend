use axum::{routing::get, Router};
use tracing_subscriber;

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Build router
    let app = Router::new().route("/", get(hello_indexmaker));

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3002")
        .await
        .unwrap();

    tracing::info!("Server listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.unwrap();
}

async fn hello_indexmaker() -> &'static str {
    "Hello from IndexMaker Backend! 🚀"
}
