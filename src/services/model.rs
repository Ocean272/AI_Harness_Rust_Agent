use anyhow::Result;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::attention::{generate, MicroGPT};

pub struct Tokenizer {
    chars: Vec<char>,
}

impl Tokenizer {
    pub fn new(vocab: &str) -> Self {
        Self {
            chars: vocab.chars().collect(),
        }
    }

    pub fn encode(&self, text:  &str) -> Vec<u32> {
        text.chars()
            .map(|c| self.chars.iter().position(|&x| x == c).unwrap_or(0) as u32)
            .collect()
    }

    pub fn decode(&self, token_ids: &[u32]) -> String {
        token_ids
            .iter()
            .map(|&id| self.chars.get(id as usize).cloned().unwrap_or('?'))
            .collect()
        }
}

pub struct ModelService {
    model: Arc<Mutex<MicroGPT>>,
    tokenizer: Tokenizer,
    device: Device,
    block_size: usize,
}

impl ModelService {
    pub fn new(weights_path: &str, vocab_str: &str) -> Result<Self> {
        let device = Device::Cpu;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)?
        };

        let vocab_size = vocab_str.chars().count();
        let n_embd = 64;
        let n_head = 4;
        let n_layer  = 4;
        let block_size = 64;

        let model = MicroGPT::new(vb, vocab_size, n_embd, n_head, n_layer)?;

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            tokenizer: Tokenizer::new(vocab_str),
            device,
            block_size,
        })
    }

    pub async fn generate_text(&self, prompt: &str, max_tokens: usize) -> Result<String> {
        let prompt_ids = self.tokenizer.encode(prompt);
        let input_ids = if prompt_ids.is_empty() { vec![0u32]} else {prompt_ids};

        let seq_len = input_ids.len();
        let context = Tensor::from_slice(&input_ids, (1, seq_len), &self.device)?;

        let model = self.model.lock().await;
        let generated = generate(&model, context, max_tokens, self.block_size)?;
        let token_ids: Vec<u32> = generated.squeeze(0)?.to_vec1()?;
        Ok(self.tokenizer.decode(&token_ids))
    } 
}