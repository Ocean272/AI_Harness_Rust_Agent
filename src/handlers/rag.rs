use axum::{
    extract::State,
    response::IntoResponse,
    Json
};
use crate::api::AppState;

#[derive(serde::Deserialize)]
pub struct SearchPayload {
    pub prompt: String,
    pub limit: Option<u64>,
}

pub async fn query_vector_memory_handler(
    State(state): State<AppState>,
    Json(payload): Json<SearchPayload>,
) -> impl IntoResponse {
    println!("[RAG Search] Received query prompt: '{}', limit: {:?}", payload.prompt, payload.limit);
    // Determine limit (default to 5 or user-specific)
    let limit = payload.limit.unwrap_or(20);
    
    // Generate embedding vector for the prompt (using your embedding service / Ollama pipeline)
    match state.qdrant_service.get_embedding(&payload.prompt).await {
        Ok(vector) => {
            println!("[RAG Search] Embedding generated successfully. Querying Qdrant...");
            match state.qdrant_service.search(vector, limit).await {
                Ok(snippets) => {
                    println!("[RAG Search] Qdrant search returned {} snippets.", snippets.len());
                    Json(serde_json::json!({
                        "status": "success",
                        "results": snippets
                    }))
                },
                Err(e) => {
                    eprintln!("[RAG Search Error] Qdrant search failed: {}", e);
                    Json(serde_json::json!({
                        "status": "error",
                        "message": format!("Qdrant search failed: {}", e)
                    }))
                },    
            }
        },
        Err(e) => {
            eprintln!("[RAG Search Error] Embedding generation failed: {}", e);
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Embedding generation failed: {}", e)
                
            }))
        }
    }
    

    // // Call Qdrant Service search method (matching your qdrant.rs signature)
    // let search_results = state
    //     .qdrant_service
    //     .search(query_vector, limit)
    //     .await
    //     .unwrap_or_default();

    // Ok(Json(VectorSearchQueryResponse {
    //     status: "success".to_string(),
    //     results: search_results,
    // }))
}