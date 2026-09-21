use crate::qdrant::QdrantService;
use anyhow::Result;
use mail_parser::MessageParser;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

// Process a single .eml file and ingest its contents into Qdrant
pub async fn process_eml_file(path: &Path, qdrant: &QdrantService) -> Result<()> {
    println!("[Watcher] Processing file: {:?}", path);

    let bytes = tokio::fs::read(path).await?;
    let message = MessageParser::default()
        .parse(&bytes)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse EML format"))?;

    let subject = message.subject().unwrap_or("No Subject");
    let body = message
        .body_text(0)
        .unwrap_or_else(|| "No text body found".into());

    let doc_id = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown_email");

    let formatted_content = format!("Subject: {}\n\n{}", subject, body);

    qdrant.ingest_document(doc_id, &formatted_content).await?;
    println!("[Watcher] Successfully ingested: {}", doc_id);

    Ok(())
}

// Watches a directory and sends newly created .eml file paths over an async channel
pub async fn start_directory_watcher(
    watch_path: PathBuf,
    qdrant: Arc<QdrantService>,
) -> Result<()> {
    if !watch_path.exists() {
        tokio::fs::create_dir_all(&watch_path).await?;
    }

    let (tx, mut rx) = mpsc::channel::<PathBuf>(100);

    // Setup file system watcher
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                if let EventKind::Create(_) = event.kind {
                    for path in event.paths {
                        if path.extension().and_then(|s| s.to_str()) == Some("eml") {
                            let _ = tx.blocking_send(path);
                        }
                    }
                }
            }
        },
        Config::default(),
    )?;

    if let Ok(mut entries) = tokio::fs::read_dir(&watch_path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("eml") {
                if let Err(e) = process_eml_file(&path, &qdrant).await {
                    eprintln!(
                        "[Watcher Error] Startup ingest failed for {:?}: {}",
                        path, e
                    );
                }
            }
        }
    }

    watcher.watch(&watch_path, RecursiveMode::NonRecursive)?;
    println!(
        "[Watcher] Monitoring directory '{:?}' for incoming .eml files...",
        watch_path
    );

    // Event processing loop
    while let Some(file_path) = rx.recv().await {
        // Small delay to allow file writing to finish
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        if let Err(e) = process_eml_file(&file_path, &qdrant).await {
            eprintln!("[Watcher Error] Failed to process {:?}: {}", file_path, e);
        }
    }

    Ok(())
}
