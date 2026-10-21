use crate::services::sanitizer;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[allow(dead_code)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub exit_code: Option<i32>,
}

// Executes a system binary with strict argument isolation and timeout bounds
pub async fn run_sandboxed_command(
    program: &str,
    args: &[&str],
    timeout_secs: u64,
) -> Result<ExecutionResult, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);

    // Enforce process timeout limit
    let output = match timeout(Duration::from_secs(timeout_secs), cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("Execution failed to spawn: {}", e)),
        Err(_) => return Err(format!("Execution TIMEOUT: Killed after {}s", timeout_secs)),
    };

    // Sanitize stdout & stderr output before returning
    let raw_stdout = String::from_utf8_lossy(&output.stdout);
    let raw_stderr = String::from_utf8_lossy(&output.stderr);

    Ok(ExecutionResult {
        stdout: sanitizer::sanitize(&raw_stdout),
        stderr: sanitizer::sanitize(&raw_stderr),
        success: output.status.success(),
        exit_code: output.status.code(),
    })
}