use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
// use std::collections::HashSet;
use crate::services::{sanitizer, executor};

// define the input arguments the LLM will provide
#[derive(Deserialize, JsonSchema, Debug)]
pub struct SecurityScanArgs {
    /// The raw shell command string extracted from the email that need validation
    pub command_to_validate: String,
}

// Define the structured output returned by the scanner
#[derive(Serialize, Deserialize, Debug)]
pub struct SecurityReport {
    pub safe_to_execute: bool,
    pub risk_level: String,
    pub detected_flags: Vec<String>,
    pub sanitized_command: String,
    pub execution_output: String,
}

pub struct SecurityValidatorTool;

#[derive(Debug)]
pub struct SecurityToolError(pub String);

impl std::fmt::Display for SecurityToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Security Tool Error: {}", self.0)
    }
}

impl std::error::Error for SecurityToolError {}

impl Tool for SecurityValidatorTool {
    const NAME: &'static str = "validate_command_security";

    type Args = SecurityScanArgs;
    type Output = String; // We'll return a serialize JSON report string to Rig
    type Error = SecurityToolError;

    // Add this definition requirement  for Rig's trait implementation
    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        let schema_generator = schemars::gen::SchemaSettings::default().into_generator();
        let schema = schema_generator.into_root_schema_for::<Self::Args>();

        rig::completion::ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Scans and validates raw shell command vectors for structural or system security compliance breaches before sandbox container evaluation.".to_string(),
            parameters: serde_json::to_value(schema).unwrap(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        println!(
            "[Security Gateway] Analyzing command string: '{}'",
            args.command_to_validate
        );

        let mut detected_flags = Vec::new();
        let mut safe_to_execute = true;
        let mut risk_level = "LOW".to_string();

        let cleaned_cmd = args.command_to_validate.trim();

        // 1. High_Severity Blocklist (destrictive modifiers and sensitive files)
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

        for token in structural_blocklist {
            if cleaned_cmd.contains(token) {
                detected_flags.push(format!("Forbidden system token detected: '{}'", token));
                safe_to_execute = false;
                risk_level = "CRITICAL".to_string();
            }
        }

        // 2. Shell Injection & Nesting Checks (Backticks or nested subshells)
        if cleaned_cmd.contains('`') || cleaned_cmd.contains("$(") {
            detected_flags.push(
                "Suspicious shell nesting or command injection sequence (` or $())".to_string(),
            );
            safe_to_execute = false;
            risk_level = "HIGH".to_string();
        }

        // 3. Brute-Force & Loop Probihitions (Fork bombs, excessive pipeline sequences)
        if cleaned_cmd.contains(":|:")
            || cleaned_cmd.split(";").count() > 3
            || cleaned_cmd.split("&&").count() > 3
        {
            detected_flags.push(
                "Excessive command chaining or denial-of-service/fork bombs logic patterns"
                    .to_string(),
            );
            safe_to_execute = false;
            risk_level = "HIGH".to_string();
        }

        // Scrub sensitive tokens/secrets using the sanitizer service
        let scrubbed_cmd = sanitizer::sanitize(cleaned_cmd);

        // Sandbox Execution Evaluation
        let mut exec_output = String::new();
        if safe_to_execute {
            // Split binary and args safely for direct execution
            let parts: Vec<&str> = scrubbed_cmd.split_whitespace().collect();
            if !parts.is_empty() {
                let program = parts[0];
                let args_slice = &parts[1..];

                match executor::run_sandboxed_command(program, args_slice, 5).await {
                    Ok(res) => {
                        exec_output = format!("Exit Code {:?}: {}", res.exit_code, res.stdout);
                    }
                    Err(e) => {
                        exec_output = format!("Sandbox Error: {}", e);
                    }
                }
            }
        } else {
            exec_output = "Execution blocked due to security flags.".to_string();
        }

        // Compile the report
        let report = SecurityReport {
            safe_to_execute,
            risk_level,
            detected_flags: if detected_flags.is_empty() {
                vec!["No threats detected".to_string()]
            } else {
                detected_flags
            },
            sanitized_command: cleaned_cmd.to_string(),
            execution_output: exec_output,
        };

        // Serialize the report back to Rig
        let report_json =
            serde_json::to_string(&report).map_err(|e| SecurityToolError(e.to_string()))?;

        println!(
            "[Security Gateway] Analysis Complete. Status: Safe={} | Risk={}",
            safe_to_execute, report.risk_level
        );
        Ok(report_json)
    }
}
