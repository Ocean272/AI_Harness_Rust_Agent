use std::collections::{HashSet, VecDeque};
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex as AsyncMutex;
use async_trait::async_trait;
use tracing::info;

use crate::monitor::{AgentMonitor, ExecutionTrace, SafetyAction};

pub struct EmbeddedMonitor {
    // Stores only the last N traces in memory (e.g., 50 logs = < 50 KB)
    trace_buffer: AsyncMutex<VecDeque<ExecutionTrace>>,
    max_buffer_size: usize,

    // Synchronous mutex for fast lock-free loop detection
    seen_calls: StdMutex<HashSet<String>>,
}

impl EmbeddedMonitor {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            trace_buffer: AsyncMutex::new(VecDeque::with_capacity(max_buffer_size)),
            max_buffer_size,
            seen_calls: StdMutex::new(HashSet::new()),
        }
    }

    #[allow(dead_code)]
    pub fn reset_guardrail(&self) {
        if let Ok(mut set) = self.seen_calls.lock() {
            set.clear();
        }
    }
}

#[async_trait]
impl AgentMonitor for EmbeddedMonitor {
    fn audit_context(&self, prompt_tokens: usize, max_context: usize) -> SafetyAction {
        let saturation = (prompt_tokens as f64 / max_context as f64) * 100.0;

        info!(
            "[AgentMonitor] Context usage: {} / {} tokens ({:.1}% saturation)",
            prompt_tokens,
            max_context,
            saturation
        );

        if saturation >= 88.0 {
            // Context is near limit; route to external model API to prevent truncation)
            SafetyAction::EscalateToCloud
        } else if saturation >= 75.0 {
            // Trim oldest dialogue history lines locally
            SafetyAction::PruneContext
        } else {
            SafetyAction::Proceed
        }
    }

    fn validate_tool_execution(&self, tool_name: &str, args: &str) -> Result<(), &'static str> {
        let signature = format!("{}:{}", tool_name, args);
        let mut set = self.seen_calls.lock().unwrap();

        if set.contains(&signature) {
            // Prevents local 3B model from re-invoking the exact same failed tool signature
            Err("Agent Guardrail Triggered: Infiniteloop detect on identical tool payload.")
        } else {
            set.insert(signature);
            Ok(())
        }
    }

    async fn get_recent_traces(&self, count: usize) -> Vec<ExecutionTrace> {
        let buffer = self.trace_buffer.lock().await;
        buffer.iter().rev().take(count).cloned().collect()
    }

    async fn record_trace(&self, trace:ExecutionTrace) {
        let mut buffer = self.trace_buffer.lock().await;
        if buffer.len() >= self.max_buffer_size {
            buffer.pop_front();
        }
        buffer.push_back(trace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_context_thresholds() {
        let monitor = EmbeddedMonitor::new(100); // Adjust constructor if arguments are needed
        let max_context = 2048;

        // 1. Test Proceed (< 75.0% saturation) -> 1000 / 2048 = ~48.8%
        let action = monitor.audit_context(1000, max_context);
        assert_eq!(action, SafetyAction::Proceed);

        // 2. Test PruneContext (75.0% <= saturation < 88.0%) -> 1600 / 2048 = ~78.1%
        let action = monitor.audit_context(1600, max_context);
        assert_eq!(action, SafetyAction::PruneContext);

        // 3. Test EscalateToCloud (>= 88.0% saturation) -> 1900 / 2048 = ~92.7%
        let action = monitor.audit_context(1900, max_context);
        assert_eq!(action, SafetyAction::EscalateToCloud);
    }
}