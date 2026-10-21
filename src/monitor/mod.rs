use async_trait::async_trait;
use serde::{Deserialize, Serialize};
pub mod embedded;
pub mod cloud;

pub use cloud::GeminiCloudClient;

// Data Struction capturing a single execution step in memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub trace_id: String,
    pub timestamp_ms: u64,
    pub model_name: String,
    pub prompt_tokens: usize,
    pub max_context_tokens: usize,
    pub duration_ms: u64,
    pub status: ExecutionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExecutionStatus {
    Success,
    ContextPruned,
    EscalatedToCloud,
    LoopDetected,
    JsonParseFailed,
}

/// Pre-flight decision returned by the Context Auditor
#[derive(Debug, PartialEq)]
pub enum SafetyAction {
    Proceed,
    PruneContext,
    EscalateToCloud,
}

/// The core embedded monitor trait interface.
#[async_trait]
pub trait AgentMonitor: Send + Sync {
    /// Inspects token count before sending payload to the local model engine.
    fn audit_context(&self, prompt_tokens: usize, max_context: usize) -> SafetyAction;
    
    /// Validates model tool calls to detect infinite loops or repeated arguments.
    fn validate_tool_execution(&self, tool_name: &str, args: &str) -> Result<(), &'static str>;

    /// Records an execution trace into the internal zero-overhead ring buffer.
    async fn record_trace(&self, trace: ExecutionTrace);

    /// Retrieves the last N execution traces for local inspection / HTTP endpoint.
    async fn get_recent_traces(&self, count: usize) -> Vec<ExecutionTrace>;
}
