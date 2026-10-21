use crate::qdrant::QdrantService;
use axum::{
    extract::{Path, State, Json},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex};
use crate::services::sanitizer;
use crate::monitor::{GeminiCloudClient, AgentMonitor, ExecutionTrace};

#[derive(serde::Deserialize)]
pub struct IngestPayload { 
    pub text: String,
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
    pub monitor: Arc<dyn AgentMonitor>,
    pub cloud_client: Option<Arc<GeminiCloudClient>>,
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

// pub async fn query_vector_memory_handler(
//     State(state): State<AppState>,
//     Json(payload): Json<VectorSearchQueryRequest>,
// ) -> Result<Json<VectorSearchQueryResponse>, StatusCode> {
//     // Generate embedding vector for the prompt (using your embedding service / Ollama pipeline)
//     let query_vector = match state.qdrant_service.get_embedding(&payload.prompt).await {
//         Ok(vector) => vector,
//         Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
//     };

//     // Determine limit (default to 20 or user-specific)
//     let limit = payload.limit.unwrap_or(20);

//     // Call Qdrant Service search method (matching your qdrant.rs signature)
//     let search_results = state
//         .qdrant_service
//         .search(query_vector, limit)
//         .await
//         .unwrap_or_default();

//     Ok(Json(VectorSearchQueryResponse {
//         status: "success".to_string(),
//         results: search_results,
//     }))
// }

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
    println!("[DEBUG EXEC] Running command string: '{}'", command);

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
            message: sanitizer::sanitize(&format!("Command '{}' executed.", command)),
            task_id: Some(task_id.to_string()),
        }),
    )
}


async fn serve_dashboard() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn serve_submit() -> Html<&'static str> {
    Html(include_str!("../static/submit.html"))
}

async fn serve_search_page() -> Html<&'static str> {
    Html(include_str!("../static/search.html"))
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

pub async fn get_monitor_traces(
    State(state): State<AppState>,
) -> (StatusCode, Json<Vec<ExecutionTrace>>) {
    let traces = state.monitor.get_recent_traces(50).await;
    (StatusCode::OK, Json(traces))
}

pub async fn ingest_memory_handler(
    State(state): State<AppState>,
    Json(payload): Json<IngestPayload>,
) -> (StatusCode, Json<ApiResponse>) {
    let memory_id = format!("mem-{}", uuid::Uuid::new_v4());

    let _ = crate::db::create_task(
        &state.db_pool,
        &memory_id,
        &payload.text,
        "MEMORY_INGEST",
        "LOW",
        "PROCESSING"
    ).await;

    // Ingest directly into Qdrant vector memory without running shell commands
    let result = state
        .qdrant_service
        .ingest_task_execution(
            &memory_id,
            &payload.text,
            "MEMORY_INGEST",
            &payload.text,
            "",
            "STORED",
        )
        .await;

    match result {
        Ok(_) => {
            if let Err(e)  = crate::db::update_task_execution(
                &state.db_pool,
                &memory_id,
                "STORED",
                Some(&payload.text),
                Some(""),
            ).await {
                eprintln!("[DB ERROR] Failed to record memory ingestion for {}: {}", memory_id, e);
            }

            (
                StatusCode::OK,
                Json(ApiResponse {
                    status: "SUCCESS".to_string(),
                    message: "Context memory successfully ingested into Qdrant".to_string(),
                    task_id: Some(memory_id),
                }),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                status: "ERROR".to_string(),
                message: format!("Failed to ingest vector memory: {:?}", e),
                task_id: None,
            }),
        ),
    }
}

// Initialize Axum Router
pub fn create_router(app_state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_dashboard))
        .route("/submit", get(serve_submit))
        .route("/api/v1/monitor/traces", get(get_monitor_traces))
        .route("/api/v1/agent/task", post(crate::handlers::command::handle_task_submission))
        .route("/api/v1/tasks/pending", get(get_pending_tasks))
        .route("/api/v1/tasks/:task_id/approve", post(approve_task))

        // New: Memory Ingest Route
        .route("/api/ingest", post(ingest_memory_handler))

        // New: Task Audit Log & Respond Routes
        .route("/api/tasks", post(crate::handlers::command::handle_task_submission))
        .route("/api/tasks/history", get(get_task_history))
        .route("/api/tasks/:id/respond", post(approve_task))
        .route("/api/memory/search", post(crate::handlers::rag::query_vector_memory_handler))
        .route("/search", get(serve_search_page))
        .with_state(app_state)
}

