use axum::{extract::State, Json, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::api::{AppState, execute_and_record_task};
use crate::monitor::SafetyAction;
//use crate::tools::security::SecurityValidatorTool; // Uncomment with your actual path

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskType {
    Query,
    Command,
}

#[derive(Deserialize)]
pub struct TaskSubmission {
    pub task_type: TaskType,
    pub task_id: Option<String>,
    pub id: Option<String>,
    pub prompt: String,
    pub proposed_command: Option<String>,
    pub risk_level: Option<String>,
}

#[derive(Serialize)]
pub struct TaskResponse {
    pub status: String,
    pub message: String,
    pub task_id: Option<String>,
    pub risk_level: Option<String>,
}

pub async fn handle_task_submission(
    // State(pool) : State<SqlitePool>, // uncomment if using shared state database pool
    State(state): State<AppState>,
    Json(payload): Json<TaskSubmission>,
) -> Result<Json<TaskResponse>, StatusCode> {
    let  task_id = payload.task_id
        .or(payload.id)
        .unwrap_or_else(|| format!("task-{}", uuid::Uuid::new_v4()));

    let prompt = payload.prompt.clone();
    let proposed_command = payload.proposed_command
        .clone()
        .unwrap_or_else(|| "".to_string());


    match payload.task_type {
        TaskType::Query => {
            if prompt.starts_with("MEMORY_INGEST:") {
                let clean_memory = prompt.trim_start_matches("MEMORY_INGEST").trim();

                if let Err(e) = state.qdrant_service.ingest_document(&task_id, clean_memory).await {
                    eprintln!("Failed to insert document into Qdrant: {:?}", e);
                }

                let status = "SUCCESS";
                let stdout_msg = format!("Memory successfully indexed: {}", clean_memory);

                // Indexes the raw memory into Qdrant, bypass Ollama if it was tagged with "MEMORY_INGEST" during pre-post.
                let insert_result = sqlx::query(
                    r#"
                    INSERT INTO tasks (id, prompt, proposed_command, risk_level, status, stdout, stderr)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&task_id)
                .bind(&prompt)
                .bind("")
                .bind("SAFE")
                .bind(status)
                .bind(&stdout_msg)
                .bind("")
                .execute(&state.db_pool)
                .await;
                
                if let Err(e) = insert_result {
                    eprintln!("FAILED to insert query task into SQLite: {:?}", e);
                }

                return Ok(Json(TaskResponse {
                    status: status.to_string(),
                    message: stdout_msg,
                    task_id: Some(task_id),
                    risk_level: Some("SAFE".to_string()),
                }));
            }
            // Standard RAG Query Flow (Ollama Inference Pipeline)
            // Fetch RAG context from Qdrant
            let rag_context = match state.qdrant_service.get_embedding(&prompt).await {
                Ok(vector) => match state.qdrant_service.search(vector, 5).await {
                    Ok(snippets) if !snippets.is_empty() => snippets.join("\n\n"),
                    _ => "No prior memory context found.".to_string(),
                },
                Err(_) => "Failed to retrieve vector memory.".to_string(),
            };

            let ollama_host = std::env::var("OLLAMA_API_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
            let ollama_model = std::env::var("OLLAMA_MODEL")
                .unwrap_or_else(|_| "llama3.2:3b".to_string());

            let endpoint = format!("{}/api/generate", ollama_host.trim_end_matches('/'));
            let client = reqwest::Client::new();

            let system_prompt = format!(
                "You are an autonomous Linux system administrator agent.\n\n
                 Relevant Memory Context:\n{}\n\n
                 User Query:{}",

                rag_context, prompt
            );

            // --- AGENT MONITOR CONTEXT AUDIT ---
            let estimated_tokens = system_prompt.len() / 4;
            let action = state.monitor.audit_context(estimated_tokens, 4096);

            let ollama_answer: String = match action {
                SafetyAction::EscalateToCloud => {
                    eprintln!("[Monitor Warning]: Context saturation high (>88%). Escalating request...");

                    let cloud_res = None;
                    if let Some(ref cloud_client) = state.cloud_client {
                        if let Ok(cloud_response) = cloud_client.completion(&prompt).await {
                            // Return cloud response directly or format it into ollama_answer
                            let _cloud_res = Some(cloud_response);
                        }
                    }
                    // Return cloud response if successful, otherwise fall back to empty/error string
                    cloud_res.unwrap_or_else(|| "Failed to escalate to cloud model.".to_string())
                }
                SafetyAction::PruneContext => {
                    eprintln!("[Monitor Warning]: Context saturation moderate (>75%). Pruning context...");
                    // Execute local Ollama call
                    let body = serde_json::json!({
                        "model": ollama_model,
                        "prompt": system_prompt,
                        "stream": false
                    });

                    match client.post(&endpoint).json(&body).send().await {
                        Ok(res) => match res.json::<serde_json::Value>().await {
                            Ok(json) => json["response"].as_str().unwrap_or("No response generated.").to_string(),
                            Err(_) => "Failed to parse Ollama response.".to_string(),
                        },
                        Err(_) => "Failed to connect to Ollama.".to_string(),
                    }                   
                }
                SafetyAction::Proceed => {
                    let body = serde_json::json!({
                        "model": ollama_model,
                        "prompt": system_prompt,
                        "stream": false
                    });

                    match client.post(&endpoint).json(&body).send().await {
                        Ok(res) => match res.json::<serde_json::Value>().await {
                            Ok(json) => json["response"].as_str().unwrap_or("No response generated.").to_string(),
                            Err(_) => "Failed to parse Ollama response.".to_string(),
                        },
                        Err(_) => "Failed to connect to Ollama.".to_string(),
                    }
                }
            };
            let status = "SUCCESS";

            // Persist the query and answer into SQLite database
            let insert_result = sqlx::query(
                r#"
                INSERT INTO tasks (id, prompt, proposed_command, risk_level, status, stdout, stderr)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&task_id)
            .bind(&prompt)
            .bind("")
            .bind("SAFE")
            .bind(status)
            .bind(&ollama_answer)
            .bind("")
            .execute(&state.db_pool)
            .await;
            
            if let Err(e) = insert_result {
                eprintln!("FAILED to insert query task into SQLite: {:?}", e);
            }

            let _ = state.qdrant_service.ingest_task_execution(
                &task_id,
                &prompt,
                "QUERY_EXECUTION",
                &ollama_answer,
                "",
                status,
            ).await;

            return Ok(Json(TaskResponse {
                status: status.to_string(),
                message: ollama_answer,
                task_id: Some(task_id),
                risk_level: Some("SAFE".to_string()),
            }));
        }
        TaskType::Command => {
            let mut risk_level = "LOW".to_string();
            let mut safe_to_execute = true;

            let structural_blocklist = vec![
                "rm -rf",
                "rm -f",
                "mkfs",
                "dd if=",
                "/etc/shadow",
                "/etc/passwd",
                "/etc/secret",
                "systemctl",
                "service",
                "docker",
            ];

            for token in &structural_blocklist {
                if proposed_command.contains(token) {
                    safe_to_execute = false;
                    risk_level = "CRITICAL".to_string();
                    break;
                }
            }    
            // If user explicitly passed a risk level in payload, you can factor it in or let the validator override
            if let Some(user_risk) = payload.risk_level {
                if user_risk.to_uppercase() == "CRITICAL" {
                    risk_level = "CRITICAL".to_string();
                    safe_to_execute = false;
                }
            }

            let status = if safe_to_execute { "EXECUTED" } else { "PENDING" };
            
            let _ = sqlx::query(
                r#"
                INSERT INTO tasks (id, prompt, proposed_command, risk_level, status)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(&task_id)
            .bind(&prompt)
            .bind(&proposed_command)
            .bind(&risk_level)
            .bind(status)
            .execute(&state.db_pool)
            .await;
            
            if safe_to_execute {
                let _ = execute_and_record_task(
                    &state,
                    &task_id,
                    &prompt,
                    &proposed_command,
                    &risk_level,
                ).await;
            }
            
            return Ok(Json(TaskResponse {
                status: status.to_string(),
                message: format!("Command '{}' executed successfully.", proposed_command),
                task_id: Some(task_id),
                risk_level: Some(risk_level),
            }));
        }
    };
}