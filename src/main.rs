mod db;
mod tools;
mod qdrant;
mod monitor;
mod attention;
mod handlers;

pub mod listeners;
pub mod api;
pub mod services;

use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;

use api::{create_router, AppState};
use db::init_db;
use qdrant::QdrantService;
use crate::monitor::GeminiCloudClient;
use crate::monitor::embedded::EmbeddedMonitor;


// IsolatedTerminalTool,
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let pool = init_db().await?;

    // Initialize QdrantService
    let qdrant_service = Arc::new(QdrantService::new().await?);

    let monitor = Arc::new(EmbeddedMonitor::new(50));

    let cloud_client = GeminiCloudClient::new().ok().map(Arc::new);

    let state = AppState {
        pending_tasks: Arc::new(std::sync::Mutex::new(vec![])),
        qdrant_service: qdrant_service.clone(),
        db_pool: pool.clone(),
        monitor,
        cloud_client,
    };
    
    services::start_background_workers(state.clone()).await?;

    let app = create_router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("[HTTP Server] Listening on http://{}", addr);

   
    // Wrap the newly configured agent inside an atomic pointer block
    let _shared_agent = services::agent::init_agent_runner()?;

    println!("[System Ready] Booting live background IMAP listener task...");

  
    // This line keeps the system open and running at all times!
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl_c event");
        })
        .await?;
    println!("Shutting down...");
    Ok(())
}
