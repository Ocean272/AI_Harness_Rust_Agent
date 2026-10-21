use regex::Regex;
use std::sync::OnceLock;

static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

fn get_patterns() -> &'static Vec<Regex> {
    PATTERNS.get_or_init(|| {
        vec![
            // Matches passwords, secret keys, bearer tokens, API keys
            Regex::new(r#"(?i)(api[_-]?key|secret|password|bearer|token)\s*[:=]\s*['"]?([a-zA-Z0-9_\-\.]{8,})['"]?"#).unwrap(),
            // Matches RSA / OpenSSH Private Keys
            Regex::new(r"-----BEGIN [A-Z ]+ PRIVATE KEY-----[\s\S]*?_____END [A-Z ]+ PRIVATE KEY-----").unwrap(),
            // Matches JWT Tokens
            Regex::new(r"eyJ[a-zA-Z0-9_-]+\.eyJ[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+").unwrap(),
            // Matches IPv4 addresses if you want to mask internal node IPs
            // Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap(),
        ]
    })
}

pub fn sanitize(raw_text: &str) -> String {
    let mut cleaned = raw_text.to_string();
    for pattern in get_patterns() {
        cleaned = pattern.replace_all(&cleaned, "$1=[REDACTED_SECRET]").to_string();
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_redaction(){
        let input = "Server responded with api_key='sk_live_1234567890abcdef' and token: 987654321xyz";
        let output = sanitize(input);
        assert!(!output.contains("sk_live_1234567890abcdef"));
        assert!(output.contains("['REDACTED_SECRET]"));
    }
}