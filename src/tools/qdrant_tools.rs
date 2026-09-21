use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use crate::qdrant::QdrantService;

#[derive(Deserialize)]
pub struct KnowledgeQuery {
    pub query_text: String,
}

#[derive(Deserialize)]
pub struct KnowledgeResult {
    pub matched_docs: Vec<String>,
}

pub struct QdrantKnowledgeTool {
    pub qdrant: QdrantService,
}

imple Tool for QdrantKnowledgeTool {
    const NAME: &'static str = "search_knowledge_base";
    type Error = anyhow::Error;
    type Args = KnowledgeQuery;
    type Output = KnowledgeResult;

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. Vectorize input query
        let query_vector = generate_embedding(&args.query_text).await?;
        
        // 2. Query Qdrant via gRPC
        let docs = self.qdrant.search(query_vector, 3).await?;

        Ok(KnowledgeResult { matched_docs: docs })
    }
}
