use std::str::FromStr;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskRecord {
    pub id: String,
    pub prompt: String,
    pub proposed_command: String,
    pub risk_level: String,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub created_at: String,
}

pub async fn init_db() -> Result<SqlitePool, Box<dyn std::error::Error>> {
    // Configure connection options with WAL mode and a busy timeout for concurrency 
    let options = SqliteConnectOptions::from_str("sqlite://data/pipeline.db?mode=rwc")?
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));

    // Creates database file automatically if it doesn't exist
    let pool = SqlitePool::connect_with(options).await?;

    // Create tables
    sqlx::query(
        r#"
        PRAGMA foreign_keys = ON;
        
        CREATE TABLE IF NOT EXISTS incoming_payloads (
            id TEXT PRIMARY KEY,
            sender TEXT NOT NULL,
            subject TEXT,
            raw_body TEXT NOT NULL,
            received_at DATETME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS pipeline_runs(
            id TEXT PRIMARY KEY,
            payload_id TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('PENDING', 'RUNNING', 'SUCCESS', 'FAILED')),
            error_message TEXT,
            started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            completed_at DATETIME,
            FOREIGN KEY (payload_id) REFERENCES incoming_payloads (id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS llm_outputs (
            id TEXT PRIMARY KEY,
            pipeline_run_id TEXT NOT NULL,
            model_used TEXT NOT NULL,
            prompt TEXT NOT NULL,
            response_text TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (pipeline_run_id) REFERENCES pipeline_runs(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            prompt TEXT NOT NULL,
            proposed_command TEXT NOT NULL,
            risk_level TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'PENDING',
            stdout TEXT,
            stderr TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )
    .execute(&pool)
    .await?;

    println!("Database initialized successfully at data/pipeline.db!");
    Ok(pool)
}

// 1, Save incoming email payload
pub async fn insert_payload(
    pool: &SqlitePool,
    id: &str,
    sender: &str,
    subject: Option<&str>,
    raw_body: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO incoming_payloads (id, sender, subject, raw_body) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(sender)
    .bind(subject)
    .bind(raw_body)
    .execute(pool)
    .await?;

    Ok(())
}

// 2. Track pipeline run status
pub async fn create_pipeline_run(
    pool: &SqlitePool,
    run_id: &str,
    payload_id: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO pipeline_runs (id, payload_id, status) VALUES (?, ?, ?)")
        .bind(run_id)
        .bind(payload_id)
        .bind(status)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn mark_pipeline_complete(pool: &SqlitePool, run_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE pipeline_runs
        SET status = 'SUCCESS', completed_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(run_id)
    .execute(pool)
    .await?;

    Ok(())
}

// 3. Save Ollama output
pub async fn save_llm_output(
    pool: &SqlitePool,
    id: &str,
    pipeline_run_id: &str,
    model_used: &str,
    prompt: &str,
    response_text: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO llm_outputs (id, pipeline_run_id, model_used, prompt, response_text) VALUES (?, ?, ?, ?, ?)
        "#
    )
    .bind(id)
    .bind(pipeline_run_id)
    .bind(model_used)
    .bind(prompt)
    .bind(response_text)
    .execute(pool)
    .await?;

    Ok(())
}

// 4. Create a new task record upon submission
#[allow(dead_code)]
pub async fn create_task(
    pool: &SqlitePool,
    id: &str,
    prompt: &str,
    proposed_command: &str,
    risk_level: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO tasks (id, prompt, proposed_command, risk_level, status)
        VALUES(?, ?, ?, ?, ?)
        "#,
    )
    .bind(id)
    .bind(prompt)
    .bind(proposed_command)
    .bind(risk_level)
    .bind(status)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_task_execution(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    stdout: Option<&str>,
    stderr: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE tasks
        SET status = ?, stdout = ?, stderr = ?
        WHERE id = ?
        "#,
    )
    .bind(status)
    .bind(stdout)
    .bind(stderr)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_task_prompt(pool: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT prompt FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

pub async fn get_all_tasks(pool: &SqlitePool) -> Result<Vec<TaskRecord>, sqlx::Error> {
    sqlx::query_as::<_, TaskRecord>("SELECT * FROM tasks ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}
