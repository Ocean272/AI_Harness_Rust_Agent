//use serde::{Deserialize, Serialize};
//use schemars::JsonSchema;
//use rig::tool::Tool;
//use rig::completion::ToolDefinition;
use std::process::Command;

//#[derive(Deserialize, Serialize, JsonSchema, Debug)]
//pub struct TerminalArgs {
//    pub command: String,
//}
//
//#[derive(Debug)]
//pub struct IsolatedTerminalTool;
//
//#[derive(Debug)]
//pub struct EmailToolError(pub String);
//
//
//impl std::fmt::Display for SandboxError {
//    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//        write!(f, "Sandbox tool execution failed: {}", self.0)
//    }
//}
//
//impl std::error::Error for SandboxError {}
//
//impl Tool for IsolatedTerminalTool {
//    const NAME: &'static str = "run_sandbox_command";
//    type Args = TerminalArgs;
//    type Output = String;
//    type Error = SandboxError;
//
//    async fn definition(&self, _prompt: String) -> ToolDefinition {
//        let schema = schemars::schema_for!(TerminalArgs);
//        let parameters = serde_json::to_value(&schema)
//            .ok()
//            .and_then(|v| {
//                if v.get("properties").is_some() {
//                    Some(v)
//                } else {
//                    v.get("object").cloned()
//                }
//            })
//            .unwrap_or_else(|| serde_json::json!({
//                "type": "object",
//                "properties": {
//                    "sender": {
//                        "type":"string",
//                        "description": "The email address of the sender to filter by."
//                    }
//                },
//                "required":["sender"]
//            }));
//
//        ToolDefinition {
//            name: "fetch_incoming_emails".to_string(),
//            description: "Fetches incoming emails matching a specific sender address to extract command payloads.".to_string(),
//            parameters,
//        }
//    }

pub fn execute_sandbox_command(raw_command: &str) -> Result<String, String> {
    let cleaned = raw_command
        .replace("/bin/bash -c", "")
        .replace("/usr/bin/bash -c", "")
        .replace("bash -c", "")
        .replace("`", ""); // Clear markdown backticks if any

    let mut clean_target = cleaned.trim();

    // 💡 Smart Extract: If the chatty 3B model adds "The command is: echo ...",
    // locate the actual script starting position dynamically.
    if let Some(pos) = clean_target.find("is:") {
        clean_target = &clean_target[pos + 3..];
    } else if let Some(pos) = clean_target.find("string:") {
        clean_target = &clean_target[pos + 7..];
    }

    let mut clean_target = clean_target.trim();
    if (clean_target.starts_with('"') && clean_target.ends_with('"'))
        || (clean_target.starts_with('\'') && clean_target.ends_with('\''))
    {
        clean_target = &clean_target[1..clean_target.len() - 1];
    }

    let clean_target = clean_target.trim();

    println!(
        "[Orchestrator] Spawning safe Docker sandbox for command: {}",
        clean_target
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--network",
            "none",
            "alpine",
            "sh",
            "-c",
            clean_target,
        ])
        .output()
        .map_err(|e| e.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(format!("Execution Success:\n{}", stdout))
    } else {
        Ok(format!("Execution Failed:\n{}", stderr))
    }
}
