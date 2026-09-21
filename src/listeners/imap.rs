use std::env;
use std::net::TcpStream;
use std::sync::Arc;

//use native_tls::TlsConnector;

pub struct ImapListenerConfig {
    pub server: String,
    pub port: u16,
    pub email: String,
    pub token: String,
}

impl ImapListenerConfig {
    pub fn from_env() -> Self {
        let _ = dotenvy::dotenv();

        Self {
            server: env::var("IMAP_SERVER").unwrap_or_else(|_| "192.168.0.18".to_string()),
            port: env::var("IMAP_PORT")
                .unwrap_or_else(|_| "143".to_string())
                .parse::<u16>()
                .unwrap_or(143),
            email: env::var("IMAP_EMAIL").unwrap_or_else(|_| "test@sandbox.local".to_string()),
            token: env::var("IMAP_TOKEN").unwrap_or_else(|_| "password123".to_string()),
        }
    }
}

fn mask_ip(ip: &str) -> String {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() == 4 {
        format!("{}.{}.*.*", parts[0], parts[1])
    } else {
        "***.***.*.*".to_string()
    }
}

pub async fn start_mailbox_stream<F, Fut>(
    //  config: ImapListenerConfig,
    on_email_received: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: Fn(String, String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let config = ImapListenerConfig::from_env();
    let on_email = Arc::new(on_email_received);

    // Run the blocking IMAP listener on a dedicated OS thread pool managed by Tokio
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        println!(
            "[IMAP Daemon] Connecting securely to {}:{}...",
            mask_ip(&config.server),
            config.port
        );

        let stream =
            TcpStream::connect((config.server.as_str(), config.port)).map_err(|e| e.to_string())?;

        let client = imap::Client::new(stream);

        let mut imap_session = client
            .login(&config.email, &config.token)
            .map_err(|(err, _returned_client)| err.to_string())?;

        imap_session.select("INBOX").map_err(|e| e.to_string())?;
        println!("[IMAP Daemon] Authenticated! Real-time mailbox streaming active.");

        if let Ok(search_results) = imap_session.search("ALL") {
            println!(
                "[IMAP Daemon] Boot check found {} messages in INBOX",
                search_results.len()
            );
        }

        loop {
            println!("[IMAP Daemon] Entering IDLE sleep mode, waiting for server push...");
            {
                // Blocks the thread efficiently until the mail server notifies us of a change
                let mut idle_handle = imap_session.idle();
                idle_handle.keepalive(true);

                // The Handle acts as an iterator. Reading from it blocks the thread efficiently
                // unti a network line pushes from the mail server.
                idle_handle
                    .wait_while(imap::extensions::idle::stop_on_any)
                    .map_err(|e| e.to_string())?;
                println!("[IMAP Daemon] 🔔 Server notification received!");
            }
            println!("[IMAP Daemon] Mailbox woke up! Checking for new messages...");

            std::thread::sleep(std::time::Duration::from_millis(500));

            // Search for unread emails
            let search_results = imap_session.search("UNSEEN").map_err(|e| e.to_string())?;

            if let Some(&latest_mail_id) = search_results.iter().last() {
                let messages = imap_session
                    .fetch(latest_mail_id.to_string(), "(ENVELOPE BODY[TEXT])")
                    .map_err(|e| e.to_string())?;

                if let Some(message) = messages.iter().next() {
                    let sender = message
                        .envelope()
                        .and_then(|e| e.from.as_ref().and_then(|from_list| from_list.first()))
                        .map(|f| {
                            let mailbox_bytes = f
                                .mailbox
                                .as_ref()
                                .map(|cow| cow.as_ref())
                                .unwrap_or(b"unknown");
                            let host_bytes = f
                                .host
                                .as_ref()
                                .map(|cow| cow.as_ref())
                                .unwrap_or(b"sender.com");

                            format!(
                                "{}@{}",
                                String::from_utf8_lossy(mailbox_bytes),
                                String::from_utf8_lossy(host_bytes)
                            )
                        })
                        .unwrap_or_else(|| "unknown@sender.com".to_string());

                    let body = message
                        .text() // Extracts segment 1 text directly
                        .map(|b| String::from_utf8_lossy(b).trim().to_string())
                        .unwrap_or_default();

                    println!("\n=================================================================");
                    println!("{}", body);
                    println!(
                        "===================================================================\n"
                    );

                    // Mark as read
                    imap_session
                        .store(latest_mail_id.to_string(), "+FLAGS (\\Seen)")
                        .map_err(|e| e.to_string())?;

                    println!("[IMAP Daemon] 🔔 New email intercepted from: {}", sender);

                    // Bridge the email details back into our main async Rig runtime context!
                    let on_email_clone = on_email.clone();
                    let sender_clone = sender.clone();
                    let body_clone = body.clone();

                    tokio::runtime::Handle::current().spawn(async move {
                        on_email_clone(sender_clone, body_clone).await;
                    });
                }
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e)?;

    Ok(())
}
