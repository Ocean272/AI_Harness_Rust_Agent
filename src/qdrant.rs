use anyhow::Result;
use qdrant_client::qdrant::{
    vectors_config::Config, CreateCollection, Distance, PointStruct, SearchPointsBuilder,
    UpsertPointsBuilder, VectorParams, VectorsConfig, Condition, Filter
};
use qdrant_client::Qdrant;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
pub struct DocumentPayload {
    pub title: String,
    pub content: String,
}

pub const VECTOR_DIM: u64 = 768;

pub fn chunk_text(text: &str, _max_chars: usize) -> Vec<String> {
    text.split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect()
}

pub async fn generate_ollama_embedding(client: &reqwest::Client, text: &str) -> Result<Vec<f32>> {
    let _ = dotenvy::dotenv();
    let ollama_url =
        std::env::var("OLLAMA_API_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());

    let payload = OllamaEmbedRequest {
        model: "nomic-embed-text".to_string(),
        input: text.to_string(),
    };

    let res: OllamaEmbedResponse = client
        .post(format!("{}/api/embed", ollama_url))
        .json(&payload)
        .send()
        .await?
        .json()
        .await?;

    res.embeddings
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Empty embedding array returned from Ollama."))
}

pub struct QdrantService {
    client: qdrant_client::Qdrant,
    pub collection_name: String,
    http_client: reqwest::Client,
}

impl QdrantService {
    pub async fn new() -> Result<Self> {
        let _ = dotenvy::dotenv();

        let qdrant_url =
            std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6333".to_string());

        let collection_name =
            std::env::var("QDRANT_COLLECTION").unwrap_or_else(|_| "email_embeddings".to_string());

        let client = Qdrant::from_url(&qdrant_url).build()?;
        let http_client = reqwest::Client::new();

        let service = Self {
            client,
            http_client,
            collection_name,
        };

        service.ensure_collection().await?;

        Ok(service)
    }

    async fn ensure_collection(&self) -> Result<()> {
        if !self.client.collection_exists(&self.collection_name).await? {
            self.client
                .create_collection(CreateCollection {
                    collection_name: self.collection_name.to_string(),
                    vectors_config: Some(VectorsConfig {
                        config: Some(Config::Params(VectorParams {
                            size: VECTOR_DIM,
                            distance: Distance::Cosine.into(),
                            ..Default::default()
                        })),
                    }),
                    ..Default::default()
                })
                .await?;
            println!("[Qdrant] Created collection '{}'", self.collection_name);
        }
        Ok(())
    }

    pub async fn insert_doc(&self, content: &str, vector: Vec<f32>) -> Result<()> {
        let point_id = uuid::Uuid::new_v4().to_string();

        let payload = serde_json::json!({
            "content": content
        });

        let payload_map: qdrant_client::Payload = payload.try_into()?;
        let point = PointStruct::new(point_id, vector, payload_map);

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]))
            .await?;
        Ok(())
    }

    pub async fn search(&self, vector: Vec<f32>, _limit: u64) -> Result<Vec<String>> {
        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.collection_name, vector, 20)
                    .with_payload(true)
                    .filter(Filter {
                        must_not: vec![Condition::matches("command", "DIRECT_ANSWER".to_string())],
                        ..Default::default()
                    }),
            )
            .await?;

        // for hit in &search_result.result {
        //     println!("[DEBUG SEARCH] Hit payload: {:?}", hit.payload)
        // }

        let mut hits = search_result.result;
        hits.sort_by(|a, b| {
            let cmd_a = a.payload.get("command").and_then(|v| v.as_str()).map_or("", |v| v);
            let cmd_b = b.payload.get("command").and_then(|v| v.as_str()).map_or("", |v| v);

            let is_memory_a = if cmd_a == "MEMORY_INGEST" { 1 } else { 0 };
            let is_memory_b = if cmd_b == "MEMORY_INGEST" { 1 } else { 0 };

            if is_memory_a != is_memory_b {
                is_memory_b.cmp(&is_memory_a)
            } else {
                let time_a = a.payload.get("ingested_at").and_then(|v| v.as_str()).map_or("", |v| v);       
                let time_b = b.payload.get("ingested_at").and_then(|v| v.as_str()).map_or("", |v| v);
                time_b.cmp(time_a)  
            }
        });

        let mut docs = Vec::new();
        for hit in &hits {
            if hit.score > 0.4 {
                let payload = &hit.payload;

                // Try extracting structed task execution payload
                let command = payload.get("command").and_then(|v| v.as_str());
                if let Some(cmd) = command {
                    if cmd == "DIRECT_ANSWER" {
                        continue;
                    }

                    if cmd == "MEMORY_INGEST" {
                        // Format clean memory ingest payloads without empty command/stdout blocks
                        if let Some(content) = payload.get("content").and_then(|v| v.as_str()) {
                            docs.push(format!("Memory Ingest: {}",content));
                        } else if let Some(prompt) = payload.get("prompt").and_then(|v| v.as_str()) {
                            docs.push(format!("Memory Ingest: {}", prompt));
                        }
                        continue;
                    }
                }

                let stdout = payload.get("stdout").and_then(|v| v.as_str());
                let stderr = payload.get("stderr").and_then(|v| v.as_str());
                let prompt = payload.get("prompt").and_then(|v| v.as_str());

                if command.is_some() || stdout.is_some() || stderr.is_some() {
                    let snippet = format!(
                        "Prompt: {}\nCommand: {}\nStdout: {}\nStderr: {}",
                        prompt.map_or("N/A", |v| v),
                        command.map_or("N/A", |v| v),
                        stdout.map_or("", |v| v),
                        stderr.map_or("", |v| v),
                    );
                    docs.push(snippet);
                } else if let Some(payload_val) = payload.get("content") {
                    let str_val = payload_val.as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| payload_val.to_string());
                    docs.push(str_val);
                }
            }
        }

        Ok(docs)
    }

    pub async fn ingest_document(&self, _doc_id: &str, content: &str) -> Result<()> {
        let chunks = chunk_text(content, 500);

        for chunk in chunks.iter() {
            // 1. Generate vector via Ollama
            let vector = generate_ollama_embedding(&self.http_client, chunk).await?;

            // 2. Upsert point into Qdrant collection
            self.insert_doc(chunk, vector).await?;
        }
        Ok(())
    }

    pub async fn ingest_task_execution(
        &self,
        task_id: &str,
        prompt: &str,
        command: &str,
        stdout: &str,
        stderr: &str,
        status: &str,
    ) -> Result<()> {

        let timestamp = chrono::Utc::now().to_rfc3339();
        // Build a rich textual summary representing the execution memory
        let memory_text = format!(
            "Timestamp: {}\nTask ID: {}\nPrompt: {}\nCommand executed: {}\nStatus: {}\n Stdout: {}\n Stderr: {}",
            timestamp, task_id, prompt, command, status, stdout, stderr
        );

        // Generate vector embedding for this execution log using Ollama
        let vector = generate_ollama_embedding(&self.http_client, &memory_text).await?;

        // Prepare payload metadata map
        let mut payload_map = std::collections::HashMap::new();
        payload_map.insert(
            "task_id".to_string(),
            qdrant_client::qdrant::Value::from(task_id),
        );
        payload_map.insert(
            "prompt".to_string(),
            qdrant_client::qdrant::Value::from(prompt),
        );
        payload_map.insert(
            "command".to_string(),
            qdrant_client::qdrant::Value::from(command),
        );
        payload_map.insert(
            "status".to_string(),
            qdrant_client::qdrant::Value::from(status),
        );
        payload_map.insert(
            "stdout".to_string(),
            qdrant_client::qdrant::Value::from(stdout),
        );
        payload_map.insert(
            "stderr".to_string(),
            qdrant_client::qdrant::Value::from(stderr),
        );
        payload_map.insert(
            "ingested_at".to_string(),
            qdrant_client::qdrant::Value::from(chrono::Utc::now().to_rfc3339()),
        );

        // Create point using task_id (or hashed ID)
        let point_id = uuid::Uuid::parse_str(task_id)
            .unwrap_or_else(|_| uuid::Uuid::new_v4())
            .to_string();

        let point = PointStruct::new(point_id, vector, payload_map);

        // Upsert into Qdrant collection
        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]))
            .await?;

        println!("[Qdrant] Ingested task execution memory for {}", task_id);
        Ok(())
    }

    pub async fn get_embedding(&self, text: &str) -> Result<Vec<f32>> {
        generate_ollama_embedding(&self.http_client, text).await
    }
    
    #[allow(dead_code)]
    pub async fn search_by_text(&self, text: &str, limit: u64) -> Result<Vec<String>> {
        let vector = self.get_embedding(text).await?;
        self.search(vector, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_full_rag_pipeline() -> Result<()> {
        // Connect to local Qdrant (using gRPC port 6334)
        dotenvy::dotenv().ok();
        let qdrant = QdrantService::new().await?;

        // Test Ingestion (chunking -> Ollama embeddings -> Qdrant storage)
        let test_doc_id = "doc_email_123";
        let test_content = "The server maintenance is scheduled for Friday at 10 PM.";

        // Test insertion
        qdrant.ingest_document(test_doc_id, test_content).await?;
        println!("[Test] Document succesfully ingested and vectorized!");

        // Test retrieval
        let http_client = reqwest::Client::new();
        let query_text = "When is server maintenance?";
        let query_vector = generate_ollama_embedding(&http_client, query_text).await?;

        let results = qdrant.search(query_vector, 3).await?;
        assert!(!results.is_empty(), "Expected at least one matching chunk");
        println!("[Test] Retrieved contexts: {:?}", results);

        Ok(())
    }

    #[tokio::test]
    async fn seed_system_runbooks() -> Result<()> {
        dotenvy::dotenv().ok();
        let qdrant = QdrantService::new().await?;

        // Sample operational runbooks to eliminate command hallucinations
        let runbooks = [
            "HOST RUNBOOK - SYSTEM SERVICES: Available systemd units are 'api-gateway.service', 'postgres.service', and 'qdrant.service. Use 'systemctl restart <unit>' to manage them.",
            "HOST RUNBOOK - STORAGE & DISK: Root filesystem is on /dev/mapper/pve-vm--103--disk--0. To check disk space, use 'df -h'.",
            "HOST RUNBOOK - CONTAINERS: Host uses local Docker engine for containers. To clean up prune containers, use 'docker container prune -f'. Do NOT use kubectl; Kubernetes is not running on this host.",
            "HOST RUNBOOK - MEMORY DIAGNOSTICS: To check system memory usage, read '/proc/meminfo' using 'cat /proc/meminfo' or execute 'free -h'.",
        ];

        for (i, doc) in runbooks.iter().enumerate() {
            let doc_id = format!("runbook_{}", i);
            qdrant.ingest_document(&doc_id, doc).await?;
            println!("[SEED] Successfully ingested {}", doc_id);
        }

        Ok(())
    }
}
