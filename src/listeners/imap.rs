use std::env;
use std::net::TcpStream;
use mail_parser::MessageParser;
use std::future::Future;

use crate::tools::mask_ip_or_url;

pub async fn start_imap_listener<F, Fut>(handler: F) -> Result<(), Box<dyn std::error::Error>> 
where
    F: Fn(String, String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send,
{
    let host = env::var("IMAP_SERVER").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = env::var("IMAP_PORT")
        .unwrap_or_else(|_| "143".to_string())
        .parse()?;
    let username = env::var("IMAP_USERNAME").unwrap_or_else(|_| "agent@sandbox.local".to_string());
    let password = env::var("IMAP_PASSWORD").unwrap_or_default();

    println!("[IMAP] Connecting to {}:{}...", mask_ip_or_url(&host), port);

    // Establish TCP Connection
    let stream = TcpStream::connect((host.as_str(), port))?;
    let client = imap::Client::new(stream);

    // Authenticate with Dovecot
    let mut session = client
        .login(&username, &password)
        .map_err(|(err,_client)| err)?;
    
    println!("[IMAP] Authenticate successfully as {}", username);

    // Select INBOX
    session.select("INBOX")?;

    // Search for unread/unseen messages
    let unseen_sequence = session.search("UNSEEN")?;

    if unseen_sequence.is_empty() {
        println!("[IMAP] No unseen emails found.");
        session.logout()?;
        return Ok(());
    }

    println!("[IMAP] Found {} unseen email(s)", unseen_sequence.len());

    let parser = MessageParser::default();

    // Fetch unseen messages
    for seq in unseen_sequence.iter() {
        let fetches = session.fetch(seq.to_string(), "BODY[]")?;

        // Iterate over fetched messages
        for message in fetches.iter() {
            if let Some(body) = message.body() {
                if let Some(parsed) = parser.parse(body) {
                    let sender = parsed
                        .from()
                        .and_then(|f| f.first())
                        .map(|a| a.address().unwrap_or("unknown"))
                        .unwrap_or("unknown");
                    let _subject = parsed.subject().unwrap_or("No subject");
                    let raw_body = parsed
                        .body_text(0)
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "Empty body".to_string());

                    handler(sender.to_string(), raw_body).await;                    
                }
            }
        }
    }

    session.logout()?;
    Ok(())

}