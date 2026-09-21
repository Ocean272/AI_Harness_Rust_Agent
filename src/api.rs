use crate::db::create_task;
use crate::qdrant::QdrantService;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex};

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub task_id: Option<String>,
    pub id: Option<String>,
    pub prompt: String,
    pub proposed_command: Option<String>,
    pub risk_level: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OllamaTaskOutput {
    pub proposed_command: String,
    pub risk_level: String,
}

#[allow(dead_code)]
#[derive(Serialize)]
pub struct TaskResponse {
    pub id: String,
    pub prompt: String,
    pub proposed_command: String,
    pub risk_level: String,
    pub status: String,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

// Data structure representing a task awaiting Human-In-The-Loop review
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PendingTask {
    pub task_id: String,
    pub sender: String,
    pub prompt: String,
    pub proposed_command: String,
    pub risk_level: String,
    pub status: String, // "PENDING_APPROVAL", "EXECUTED", "REJECTED"
}

// Response payload
#[derive(Serialize)]
pub struct ApiResponse {
    pub status: String,
    pub message: String,
    pub task_id: Option<String>,
}

// Shared application state
#[derive(Clone)]
pub struct AppState {
    // In-memory pending queue (backed by SQLite in production loop)
    pub pending_tasks: Arc<Mutex<Vec<PendingTask>>>,
    pub qdrant_service: Arc<QdrantService>,
    pub db_pool: SqlitePool,
}

#[derive(Deserialize)]
pub struct ApprovalPayload {
    pub approved: Option<bool>,
    pub action: Option<String>,
}

impl ApprovalPayload {
    pub fn is_approved(&self) -> bool {
        if let Some(app) = self.approved {
            app
        } else if let Some(act) = &self.action {
            act.to_uppercase() == "APPROVED"
        } else {
            false
        }
    }
}

pub async fn get_task_history(
    State(state): State<AppState>,
) -> (StatusCode, Json<serde_json::Value>) {
    match crate::db::get_all_tasks(&state.db_pool).await {
        Ok(tasks) => {
            println!(
                "[API] Task history requested: fetched {} tasks",
                tasks.len()
            );
            (StatusCode::OK, Json(serde_json::json!(tasks)))
        }
        Err(e) => {
            eprintln!("[DB ERROR] Failed to fetch task history: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

/// Executes a shell command, updates SQLite with the output, and ingests the run details into Qdrant.
pub async fn execute_and_record_task(
    state: &AppState,
    task_id: &str,
    prompt: &str,
    command: &str,
    _risk_level: &str,
) -> (StatusCode, Json<ApiResponse>) {
    // 1. Run the command directly
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output();

    let (final_status, stdout_str, stderr_str) = match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let status = if out.status.success() {
                "EXECUTED"
            } else {
                "FAILED"
            };
            (status, Some(stdout), Some(stderr))
        }
        Err(e) => ("Failed", None, Some(e.to_string())),
    };

    // 2. Persist execution results to SQLite
    if let Err(e) = crate::db::update_task_execution(
        &state.db_pool,
        task_id,
        final_status,
        stdout_str.as_deref(),
        stderr_str.as_deref(),
    )
    .await
    {
        eprintln!(
            "[DB ERROR] Failed to record task execution for {}: {}",
            task_id, e
        );
    }

    // 3. Ingest into Qdrant vector memory
    let _ = state
        .qdrant_service
        .ingest_task_execution(
            task_id,
            prompt,
            command,
            stdout_str.as_deref().unwrap_or(""),
            stderr_str.as_deref().unwrap_or(""),
            final_status,
        )
        .await;

    // 4. Return unified JSON response
    (
        StatusCode::OK,
        Json(ApiResponse {
            status: final_status.to_string(),
            message: format!("Command '{}' executed.", command),
            task_id: Some(task_id.to_string()),
        }),
    )
}

// Axum Handler that accepts JSON and calls your db helper
pub async fn create_task_handler(
    State(state): State<AppState>,
    Json(payload): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    // Fall back to task_id or default if id isn't passed directly
    let task_id = payload
        .task_id
        .or(payload.id)
        .unwrap_or_else(|| format!("task-{}", uuid::Uuid::new_v4()));

    let (final_command, final_risk) = match (payload.proposed_command, payload.risk_level) {
        // Direct Task Submission Mode (explicit command provided)
        (Some(cmd), Some(risk)) => (cmd, risk),

        // AI Planning Mode (Only prompt/goal provided -> Use RAG + Ollama)
        _ => {
            // 1. Fetch RAG Context from Qdrant if available
            let qdrant = &state.qdrant_service;
            let rag_context = match qdrant.get_embedding(&payload.prompt).await {
                Ok(vector) => match qdrant.search(vector, 3).await {
                    Ok(snippets) if !snippets.is_empty() => snippets.join("\n---\n"),
                    _ => "No prior execution history found.".to_string(),
                },
                Err(_) => "Failed to generate query embedding.".to_string(),
            };

            // 2. Prompt Ollama for command generation
            let system_prompt = format!(
                r#"
You are an autonomous Linux system administrator agent.
Convert the following goal into a single safe bash command.

RAG Execution Context from Past Tasks:
{}

User Goal:
{}

Respond STRICTLY with a valid JSON object matching this schema:
{{
    "proposed_command":"exact single bash command",
    "risk_level": "LOW | MEDIUM | HIGH"
}}"#,
                rag_context, payload.prompt
            );

            let ollama_host = std::env::var("OLLAMA_API_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
            let ollama_model =
                std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2:3b".to_string());

            let endpoint = format!("{}/api/generate", ollama_host.trim_end_matches('/'));

            let http_client = reqwest::Client::new();
            let ollama_res = http_client
                .post(&endpoint)
                .json(&serde_json::json!({
                    "model": ollama_model,
                    "prompt": system_prompt,
                    "stream": false,
                    "format": "json"
                }))
                .send()
                .await;

            match ollama_res {
                Ok(res) => {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        if let Some(raw_str) = body["response"].as_str() {
                            if let Ok(task_out) = serde_json::from_str::<OllamaTaskOutput>(raw_str)
                            {
                                (task_out.proposed_command, task_out.risk_level)
                            } else {
                                return (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    Json(
                                        serde_json::json!({"error": "Failed to parse Ollama JSON response."}),
                                    ),
                                );
                            }
                        } else {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({"error": "Empty response from Ollama."})),
                            );
                        }
                    } else {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": "Invalid JSON response."})),
                        );
                    }
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(
                            serde_json::json!({"error": format!("Ollama connection error: {}", e) }),
                        ),
                    );
                }
            }
        }
    };

    //Auto-approve LOW risk tasks, otherwise start as PENDING
    if final_risk.to_uppercase() == "LOW" {
        // Persist initial record first
        let _ = crate::db::create_task(
            &state.db_pool,
            &task_id,
            &payload.prompt,
            &final_command,
            &final_risk,
            "APPROVED",
        )
        .await;

        // Execute immediately using the shared runner
        let (status, Json(res)) = execute_and_record_task(
            &state,
            &task_id,
            &payload.prompt,
            &final_command,
            &final_risk,
        )
        .await;

        (status, Json(serde_json::to_value(res).unwrap_or_default()))
    } else {
        // Save HIGH/MEDIUM tasks as PENDING for HITL review
        match create_task(
            &state.db_pool,
            &task_id,
            &payload.prompt,
            &final_command,
            &final_risk,
            "PENDING",
        )
        .await
        {
            Ok(_) => (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "status": "PENDING",
                    "id": task_id,
                    "proposed_command": final_command,
                    "risk_level": final_risk,
                })),
            ),
            Err(e) => {
                eprintln!("Failed to insert task: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
            }
        }
    }
}

async fn serve_dashboard() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

//  Fetch all pending tasks awaiting approval
async fn get_pending_tasks(State(state): State<AppState>) -> Json<Vec<PendingTask>> {
    let queue = state.pending_tasks.lock().unwrap();
    let pending: Vec<PendingTask> = queue
        .iter()
        .filter(|t| t.status == "PENDING_APPROVAL")
        .cloned()
        .collect();

    Json(pending)
}

// 3. Approve or Reject execution (HITL Action)
pub async fn approve_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(payload): Json<ApprovalPayload>,
) -> (StatusCode, Json<ApiResponse>) {
    let task_from_memory = {
        let mut queue = state.pending_tasks.lock().unwrap();

        if let Some(index) = queue.iter().position(|t| t.task_id == task_id) {
            Some(queue.remove(index).proposed_command)
        } else {
            None
        }
    };

    let record = sqlx::query("SELECT proposed_command FROM tasks WHERE id = ?")
        .bind(&task_id)
        .fetch_optional(&state.db_pool)
        .await
        .ok()
        .flatten();

    let command_to_run = match task_from_memory {
        Some(cmd) => Some(cmd),
        None => {
            use sqlx::Row;
            record.map(|r| r.get::<String, _>("proposed_command"))
        }
    };

    // Handle NOT_FOUND if missing in both places
    let proposed_command = match command_to_run {
        Some(cmd) => cmd,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    status: "NOT_FOUND".to_string(),
                    message: format!("Task ID {} not found.", task_id),
                    task_id: Some(task_id),
                }),
            );
        }
    };

    if payload.is_approved() {
        let task_prompt = crate::db::get_task_prompt(&state.db_pool, &task_id)
            .await
            .unwrap_or_else(|| "No prompt provided".to_string());

        //Delegate execution, SQLite persistence, and Qdrant RAG ingestion to helper
        execute_and_record_task(&state, &task_id, &task_prompt, &proposed_command, "HIGH").await
    } else {
        println!(
            "[HITL REJECTED] Command execution aborted for task: {}",
            task_id
        );

        //2. Persist rejection in SQLite
        let _ = crate::db::update_task_execution(
            &state.db_pool,
            &task_id,
            "REJECTED",
            None,
            Some("Task rejected by operator."),
        )
        .await;

        (
            StatusCode::OK,
            Json(ApiResponse {
                status: "REJECTED".to_string(),
                message: "Task execution aborted by operator.".to_string(),
                task_id: Some(task_id),
            }),
        )
    }
}

// Initialize Axum Router
pub fn create_router(app_state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_dashboard))
        .route("/api/v1/agent/task", post(create_task_handler))
        .route("/api/v1/tasks/pending", get(get_pending_tasks))
        .route("/api/v1/tasks/:task_id/approve", post(approve_task))
        // New: Task Audit Log & Respond Routes
        .route("/api/tasks", post(create_task_handler))
        .route("/api/tasks/history", get(get_task_history))
        .route("/api/tasks/:id/respond", post(approve_task))
        .with_state(app_state)
}
