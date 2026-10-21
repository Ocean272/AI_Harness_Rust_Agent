pub mod email;
pub mod security;
pub mod terminal;

// Re-export smoothly for main.rs
//pub use terminal::IsolatedTerminalTool;
//pub use email::EmailReceiverTool;

pub fn mask_ip_or_url(input: &str) -> String {
    // Regex-free approach to mask IP4 addresses (192.168.X.X -> 192.168.*.*)
    let parts: Vec<&str> = input.split('.').collect();

    // If we have a 4-part IPv4 format(or URL containing an IP)
    if parts.len() == 4 {
        // Handle port or path on the last octet if attached (eg., "13:11434/")
        let last_part = parts[3];
        let suffix = if let Some(idx) = last_part.find(|c: char| !c.is_numeric()) {
            &last_part[idx..]
        } else {
            ""
        };

        format!("{}.{}.*.*{}", parts[0], parts[1], suffix)
    } else {
        input.to_string()
    }
}
