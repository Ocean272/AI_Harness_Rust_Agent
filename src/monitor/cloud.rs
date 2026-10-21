use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::env;

// #[derive(Serialize)]
// pub struct Message {
//     pub role: String,
//     pub content: String,
// }

// #[derive(Serialize)]
// struct ChatCompletionRequest {
//     model: String,
//     messages: Vec<Message>,
// }

// #[derive(Deserialize)]
// struct ChoiceMessage {
//     content: String,
// }

// #[derive(Deserialize)]
// struct Choice {
//     message: ChoiceMessage,
// }

// #[derive(Deserialize)]
// struct ChatCompletionResponse {
//     choices: Vec<Choice>,
// }

// pub struct GeminiCloudClient {
//     client: Client,
//     api_key: String,
// }

// impl GeminiCloudClient {
//     /// Instantiates the client using GEMINI_API_KEY from .env
//     pub fn new() -> Result<Self, String> {
//         let api_key = env::var("GEMINI_API_KEY")
//             .map_err(|_| "GEMINI_API_KEY not found in .env file".to_string())?;
        
//         Ok(Self {
//             client: Client::new(),
//             api_key,
//             // base_url: "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
//         })
//     }

//     pub async fn completion(&self, prompt: &str) -> Result<String, String> {
//         let request_payload = ChatCompletionRequest {
//             model: "gemini-1.5-flash".to_string(),
//             messages: vec![Message {
//                 role: "user".to_string(),
//                 content: prompt.to_string(),
//             }],
//         };

//         let url = "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";

//         let response = self.client
//             .post(url)
//             .bearer_auth(&self.api_key)
//             .header("Content-Type", "application/json")
//             .json(&request_payload)
//             .send()
//             .await
//             .map_err(|e| format!("HTTP request failed: {}", e))?;

//         if !response.status().is_success() {
//             let status = response.status();
//             let err_text = response.text().await.unwrap_or_else(|_| "Empty response body".to_string());
//             return Err(format!("HTTP {} - Details: {}", status, err_text));
//         }

//         let parsed = response
//             .json::<ChatCompletionResponse>()
//             .await
//             .map_err(|e| format!("Failed to parse JSON response: {}", e))?;

//         parsed.choices
//             .first()
//             .map(|c| c.message.content.clone())
//             .ok_or_else(|| "No choices returned from Gemini API.".to_string())
//     }
// }

#[derive(Serialize)]
struct Part {
    text: String,
}

#[derive(Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<Content>,
}

#[derive(Deserialize)]
struct CandidatePart {
    text: String,
}

#[derive(Deserialize)]
struct CandidateContent {
    parts: Vec<CandidatePart>,
}

#[derive(Deserialize)]
struct Candidate {
    content: CandidateContent,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<Candidate>>,
}

pub struct GeminiCloudClient {
    client: Client,
    api_key: String,
}

impl GeminiCloudClient {
    pub fn new() -> Result<Self, String> {
        let api_key = env::var("GEMINI_API_KEY")
            .map_err(|_| "GEMINI_API_KEY not found in environment or .env file".to_string())?;

        Ok(Self {
            client: Client::new(),
            api_key,
        })
    }

    pub async fn completion(&self, prompt: &str) -> Result<String, String> {
        let models = vec!["gemini-3.8-flash", "gemini-3.5-flash-lite", "gemini-2.5-flash"];

        let request_payload = GeminiRequest {
            contents: vec![Content {
                parts: vec![Part {
                    text: prompt.to_string(),
                }],
            }],
        };

        for model in models {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                model, self.api_key
            );

            let response = self.client
                .post(url)
                .header("Content-Type", "application/json")
                .json(&request_payload)
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {}", e))?;
                            
            if response.status().is_success() {
                let parsed = response
                    .json::<GeminiResponse>()
                    .await
                    .map_err(|e| format!("Failed to parse JSON response: {}", e))?;
                
                return parsed.candidates
                    .as_ref()
                    .and_then(|c| c.first())
                    .and_then(|c| c.content.parts.first())
                    .map(|p| p.text.clone())
                    .ok_or_else(|| "No completion text returned from Gemini API".to_string())
            }
            
            let status = response.status();
            let err_text = response.text().await.unwrap_or_else(|_| "Empty response body".to_string());

            // if 503 (high demand), log and retry with the next fallback model
            if status.as_u16() == 503 {
                eprintln!("[Cloud Warning]: Model {} overloaded (503). Retrying with secondary model ... ", model);
                continue;
            }
            return Err(format!("HTTP {} - Details: {}", status, err_text));
        }
        Err("All Gemini models are currently unavailable due to high demand.".to_string())
    }
}