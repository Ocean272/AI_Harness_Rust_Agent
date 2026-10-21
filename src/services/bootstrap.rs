use anyhow::Result;
use std::path::PathBuf;


pub async fn start_background_workers(state: crate::api::AppState) -> Result<()> { 

    // Spawn background proactive CRON actor sweep
    //let qdrant_for_cron = qdrant_service.clone();
    let qdrant_for_watcher = state.qdrant_service.clone();

    //let monitor = Arc::new(EmbeddedMonitor::new(50));
    // If model_service is stored in state, grab it here (or initialize it)
    // let model_service = state.model_service.clone();

    tokio::spawn(async move {
    // Ensure your cron path/module is correct
        crate::services::cron::start_cron_actor().await; 
    });

    tokio::spawn(async move {
        let watch_dir = PathBuf::from("./inbox");
        if let Err(e) = crate::listeners::file_watcher::start_directory_watcher(watch_dir, qdrant_for_watcher).await
        {
            eprintln!("[Watcher Fatal] Listener stopped: {:?}", e);
        }
    });

    tokio::spawn(async move {
        let http_client = reqwest::Client::new();

        // Read from .env or fallback to defaults
        let api_host = std::env::var("API_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let api_port = std::env::var("API_PORT").unwrap_or_else(|_| "8080".to_string());
        let api_url = format!("http://{}:{}/api/tasks", api_host, api_port);

        loop {
            let http_client = http_client.clone();
            let api_url = api_url.clone();
            // let model_service_for_imap = model_service.clone();

             // Hand the executor block directly over to the real-time stream handler!
            if let Err(e) = crate::listeners::imap::start_imap_listener(move |_sender: String, raw_body: String| {
                
                let http_client = http_client.clone();
                let api_url = api_url.clone();
                //let model_service = model_service_for_imap.clone();

                async move {
                    // Check for context ingestion prefix
                    if raw_body.trim().starts_with("MEMORY_INGEST:") {
                        let clean_text = raw_body.trim_start_matches("MEMORY_INGEST:").trim();
                        println!("[IMAP -> RAG] Intercepted context memory payload: {}", clean_text);

                        // Post directly to your ingest API endpoint instead of /api/tasks
                        let ingest_url = api_url.replace("/api/tasks", "/api/ingest");
                        let payload = serde_json::json!({
                            "text" : clean_text, 
                        });


                        match http_client.post(&ingest_url)
                            .json(&payload)
                            .send()
                            .await
                        {
                            Ok(res) => println!("[IMAP -> RAG] Context successfully ingested! Status: {}", res.status()),
                            Err(err) => eprintln!("[IMAP -> RAG Error] Ingestion POST failed: {:?}", err),
                        }

                        // Early return bypasses creating an executable task in SQLite!
                        return;

                    }

                    // match model_service.generate_text(&raw_body, 64).await {
                    //     Ok(reply) => {
                    //         println!("[AI Engine] Generated Response:\n{}", reply);
                    //     }
                    //     Err(err) => {
                    //         eprintln!("[AI Engine Error]: {:?}", err);
                    //     }
                    // }

                    println!("[IMAP] Ingested email task prompt, posting payload to API...");

                    let payload = serde_json::json!({
                        "prompt" : raw_body, 
                    });

                    match http_client.post(&api_url)
                        .json(&payload)
                        .send()
                        .await
                    {
                        Ok(res) => println!("[IMAP -> API] Dispatched task! Status: {}", res.status()),
                        Err(err) => eprintln!("[IMAP -> API Error] Failed to reach API: {:?}", err),
                    }
                }
            }).await {
                eprintln!("[IMAP Error] Listener loop failed: {:?}", e);
            }
            // Poll inbox every 5 seconds
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }  
    });

    Ok(())
}