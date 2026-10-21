
use std::sync::Arc;
use anyhow::Result;
use rig::providers::ollama;
use rig::client::{ProviderClient, CompletionClient};

use crate::tools::email::EmailReceiverTool;
use crate::tools::security::SecurityValidatorTool;
use crate::tools::mask_ip_or_url;

pub fn init_agent_runner() -> Result<Arc<rig::agent::Agent<ollama::CompletionModel>>> {
    let ollama_host = std::env::var("OLLAMA_API_URL").expect("OLLAMA API_URL must be set in .env");
    std::env::set_var("OLLAMA_API_BASE_URL", &ollama_host);
    println!(
        "[Initialization] Connecting to Ollama Brain at: {}",
        mask_ip_or_url(&ollama_host)
    );

    let client = ollama::Client::from_env().expect("Failed to initialize Ollama client");

    let agent_runner = client
        .agent("llama3.2:3b")
        .preamble(r#"
            You are an automated backend infrastructure gateway executor and assistant 
            Your absolute core rule: Do NOT converse, do NOT summarize, and do NOT simulate console outputs or command execution results yourself.
            
            1. If the context contains relevant information to answer the user's inquiry, provide a direct, concise answer.
            2. If the instruction body contains an explicit shell command (e.g., Prefixed with RUN_COMMAND:, EXEC:, or systemctl/service actions), you MUST invoke the `validate_command_security` tool with the shell command before any execution occurs.
            3. If a tool was executed,output 'TASK COMPLETE' at the end. Otherwise, simply output your correct answer to the user.
            "#)
        .default_max_turns(4)
        .additional_params(serde_json::json!({ "keep_alive": "0m" }))
        .tool(EmailReceiverTool)
        .tool(SecurityValidatorTool)
        .build();

    Ok(Arc::new(agent_runner))
}