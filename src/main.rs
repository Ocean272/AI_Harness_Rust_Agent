mod db;
mod qdrant;
mod tools;
mod listeners {
    pub mod file_watcher;
    pub mod imap;
}
mod api;

use anyhow::Result;
use api::{create_router, AppState};
use qdrant::QdrantService;
use rig::client::{CompletionClient, ProviderClient};
use rig::completion::Prompt;
use rig::providers::ollama;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::tools::email::EmailReceiverTool;
use crate::tools::security::SecurityValidatorTool;

fn mask_ip_or_url(input: &str) -> String {
    // Regex-free approach to mask IP4 addresses (192.168.X.X -> 192.168.*.*)
    let parts: Vec<&str> = input.split('.').collect();

    // If we have a 4-part IPv4 format(or URL containing an IP)
    if parts.len() == 4 {
        // Handle port or path on the last octet if attached (eg., "13:11434/")
        let last_part = parts[3];
        let suffix = if let Some(idx) = last_part.find(|c: char| !c.is_numeric()) {
            &last_part[idx..]
        } else {
            ""
        };

        format!("{}.{}.*.*{}", parts[0], parts[1], suffix)
    } else {
        input.to_string()
    }
}

// IsolatedTerminalTool,
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let pool = db::init_db().await?;
    let pool_clone = pool.clone();

    // Initialize QdrantService
    let qdrant_service = Arc::new(QdrantService::new().await?);

    let state = AppState {
        pending_tasks: Arc::new(Mutex::new(Vec::new())),
        qdrant_service,
        db_pool: pool.clone(),
    };

    let app = create_router(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));

    println!("[HTTP Server] Listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    println!("Initializing Qdrant client...");

    //let qdrant_url = std::env::var("QDRANT_URL")
    //    .expect("QDRANT URL must be set in .env");
    let qdrant_service = Arc::new(QdrantService::new().await?);
    println!("Qdrant service ready!");

    let qdrant_for_watcher = Arc::clone(&qdrant_service);
    tokio::spawn(async move {
        let watch_dir = PathBuf::from("./inbox");
        if let Err(e) =
            listeners::file_watcher::start_directory_watcher(watch_dir, qdrant_for_watcher).await
        {
            eprintln!("[Watcher Fatal] Listener stopped: {}", e);
        }
    });

    let ollama_host = std::env::var("OLLAMA_API_URL").expect("OLLAMA API_URL must be set in .env");

    std::env::set_var("OLLAMA_API_BASE_URL", &ollama_host);
    println!(
        "[Initialization] Connecting to Ollama Brain at: {}",
        mask_ip_or_url(&ollama_host)
    );

    let client = ollama::Client::from_env().expect("Failed to initialize Ollama client");

    let agent_runner = client
        .agent("llama3.2:3b")
        .preamble(r#"
            You are an automated backend infrastructure gateway executor and assistant 
            Your absolute core rule: Do NOT converse, do NOT summarize, and do NOT simulate console outputs or command execution results yourself.
            
            1. If the context contains relevant information to answer the user's inquiry, provide a direct, concise answer using that context.
            2. If the instruction body contains an explicit shell command (e.g., Prefixed with RUN_COMMAND: or HEALTH_CHECK:), you MUST explicitly call your sandbox execution tool to run it. 
            3. if a tool wad executed,output 'TASK COMPLETE' at the end. Otherwise, simply output your correct answer to the user.
            "#)
        .default_max_turns(4)
        .additional_params(serde_json::json!({ "keep_alive": "0m" }))
        .tool(EmailReceiverTool)
        .tool(SecurityValidatorTool)
        .build();

    // Wrap the newly configured agent inside an atomic pointer block
    let shared_agent = std::sync::Arc::new(agent_runner);

    println!("\n[System Ready] Booting live background IMAP listener task...");

    let qdrant_clone = qdrant_service.clone();

    tokio::spawn(async move {
        // Hand the executor block directly over to the real-time stream handler!
        if let Err(e) = crate::listeners::imap::start_mailbox_stream(move |sender: String, body: String| {
            let qdrant = qdrant_clone.clone();
            let pool_clone = pool_clone.clone();
            let agent_runner = shared_agent.clone();

            async move {
                println!("\n[Pipeline Trigger] Intercepted live mail payload from: {}", sender);

                let msg_id = uuid::Uuid::new_v4().to_string();
                let run_id = uuid::Uuid::new_v4().to_string();

                // 1. Save incoming payload
                if let Err(e) = db::insert_payload(&pool_clone, &msg_id, &sender, None, &body).await {
                    eprintln!("[DB Error] Failed to save payload: {}",e);
                }
                if let Err(e) = db::create_pipeline_run(&pool_clone, &run_id, &msg_id, "RUNNING").await{
                    eprintln!("[DB Error] Failed to create pipeline run: {}", e);
                }
                                

                // 1. Generate an embedding for the incoming email body
                let context_docs = match qdrant.search_by_text(&body, 3).await {
                    Ok(docs) => docs.join("\n---\n"),
                    Err(e) => {
                        eprintln!("[Qdrant Error] Search failed: {}", e);
                        "No relevant vector context retrieved.".to_string()
                    }
                };

                // Add this log line to verify vector context:
                println!("[RAG Debug] Retrieved Context:\n{}\n", context_docs);

                            
                // -- SINGLE OLLAMA PROMPT HERE ---
                let prompt = format!(
                    "Context from Knowledge Base:\n{}\n\nInstruction Body: '{}'\nAnswer the instruction using the context above.",
                    context_docs, body
                );

                match agent_runner.prompt(&prompt).await {
                    Ok(response) => {
                        println!("[Pipeline Run Output]:\n{}", response);

                        let llm_id = uuid::Uuid::new_v4().to_string();
                        if let Err(e) = db::save_llm_output(
                            &pool_clone,
                            &llm_id,
                            &run_id,
                            "llama3.2:3b",
                            &prompt,
                            &response
                        ).await {
                            eprintln!("[DB Error] Failed to save LLM response: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Pipeline Processing Failure: {}", e);
                    } 
                }

                if let Err(e) = db::mark_pipeline_complete(&pool_clone, &run_id).await{
                    eprintln!("[DB Error] Failed to update pipeline status: {}", e);
                }
            }
        }).await {
            eprintln!("[IMAP Daemon] Task error: {}", e);
        }
        println!("[Pipeline] Task completed. Returning worker thread to pool.");        
                
    }).await?; // This line keeps the system open and running at all times!

    Ok(())
}
