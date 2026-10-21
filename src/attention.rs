use candle_core::{D, Device, Result, Tensor};
use candle_nn::{embedding, linear, LayerNorm, Embedding, Linear, Module, VarBuilder};

pub struct Head {
    key: Linear,
    query: Linear,
    value: Linear,
    head_size: usize
}

impl Head {
    pub fn new(vb: VarBuilder, n_embd: usize, head_size: usize) -> Result<Self> {
        
        // Conbined QKV projection layer
        let key = linear_no_bias(n_embd, head_size, vb.pp("key"))?;
        let query = linear_no_bias(n_embd, head_size, vb.pp("query"))?;
        let value = linear_no_bias(n_embd, head_size, vb.pp("value"))?;

        Ok(Self { key, query, value, head_size })

    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let (_b, t, _c) = x.dims3()?;
        
        let q = self.query.forward(x)?;
        let k = self.key.forward(x)?;
        let v = self.value.forward(x)?;

        // (q @ k.T) -> (B, T, head_size) @ (B, head_size, T) = (B, T, T)
        // Raw scaled dot-product attention scores: (q @ k. T) / sqrt(head_size)
        let scale = 1.0 / (self.head_size as f64).sqrt();
        let att = (q.matmul(&k.transpose(1, 2)?)? * scale)?;

        // Apply causal mask & softmax
        let mask = self.get_causal_mask(t, x.device())?;
        let att = att.broadcast_add(&mask)?;
        let att = candle_nn::ops::softmax(&att, D::Minus1)?;

        // (att @ v) -> (B, T, T) @ (B, T, head_size) = (B, T, head_size)
        att.matmul(&v)
    }

    fn get_causal_mask(&self, seq_len: usize, device: &Device) -> Result<Tensor> {
        let mask: Vec<f32> = (0..seq_len)
            .flat_map(|i| (0..seq_len).map(move |j| if j > i { f32::NEG_INFINITY } else { 0.0 }))
            .collect();
        
        Tensor::from_slice(&mask, (seq_len, seq_len), device)
    }
}

fn linear_no_bias(in_dim: usize, out_dim: usize, vb: VarBuilder) -> Result<Linear> {
    let weight = vb.get((out_dim, in_dim), "weight")?;
    Ok(Linear::new(weight, None))
}

pub struct MultiHeadAttention {
    heads: Vec<Head>,
    proj: Linear,
}

impl MultiHeadAttention {
    pub fn new(vb: VarBuilder, n_embd: usize, n_head: usize) -> Result<Self> {
        let head_size = n_embd / n_head;
        let mut heads = Vec::new();

        // Binds each head to blocks.{i}.sa.heads.{j}
        for j in 0..n_head {
            heads.push(Head::new(vb.pp(format!("heads.{j}")), n_embd, head_size)?);
        }

        let proj = linear(n_embd, n_embd, vb.pp("proj"))?;

        Ok(Self { heads, proj })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let mut head_outputs = Vec::new();
        for head in &self.heads {
            head_outputs.push(head.forward(x)?);
        }

        // Concatenate heads outputs along channel dimension
        let out = Tensor::cat(&head_outputs, D::Minus1)?;
        self.proj.forward(&out)
    }
}


// Feed-Forward Network matching blocks.{i}.ffwd
pub struct FeedForward {
    net_0: Linear, // Linear projection (n_embd -> 4 * n_embd)
    net_2: Linear, // Linear projection (4 * n_embd -> n_embd)
}

impl FeedForward {
    pub fn new(vb: VarBuilder, n_embd: usize) -> Result<Self> {
        let net_0 = linear(n_embd, 4 * n_embd, vb.pp("net.0"))?;
        let net_2 = linear(4 * n_embd, n_embd, vb.pp("net.2"))?;
        Ok(Self { net_0, net_2 })

    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let x = self.net_0.forward(x)?;
        let x = x.relu()?;
        self.net_2.forward(&x)
    }
}

// Single Transformer Block matching blocks.{i}
pub struct Block {
    sa: MultiHeadAttention,
    ffwd: FeedForward,
    ln1: LayerNorm,
    ln2: LayerNorm,
}

impl Block {
    pub fn new(vb: VarBuilder, n_embd: usize, n_head: usize) -> Result<Self> {
        let sa = MultiHeadAttention::new(vb.pp("sa"), n_embd, n_head)?;
        let ffwd = FeedForward::new(vb.pp("ffwd"), n_embd)?;
        let ln1 = candle_nn::layer_norm(n_embd, 1e-5, vb.pp("ln1"))?;
        let ln2 = candle_nn::layer_norm(n_embd, 1e-5, vb.pp("ln2"))?;
        Ok(Self { sa, ffwd, ln1, ln2 })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let x_norm1 = self.ln1.forward(x)?;
        let x = (x + self.sa.forward(&x_norm1)?)?;
        let x_norm2 = self.ln2.forward(&x)?;
        x + self.ffwd.forward(&x_norm2)?
    }
}

pub struct MicroGPT {
    token_embedding_table: Embedding,
    position_embedding_table: Embedding,
    blocks: Vec<Block>,
    ln_f: LayerNorm,
    lm_head: Linear,
}

impl MicroGPT {
    pub fn new(vb: VarBuilder, vocab_size: usize, n_embd: usize, n_head: usize, n_layer: usize) -> Result <Self> {
        let token_embedding_table = embedding(vocab_size, n_embd, vb.pp("token_embedding_table"))?;
        let position_embedding_table = embedding(64, n_embd, vb.pp("position_embedding_table"))?;

        let mut blocks = Vec::new();
        for i in 0..n_layer {
            blocks.push(Block::new(vb.pp(format!("blocks.{i}")), n_embd, n_head)?);
        }

        let ln_f = candle_nn::layer_norm(n_embd, 1e-5, vb.pp("ln_f"))?;
        let lm_head = linear(n_embd, vocab_size, vb.pp("lm_head"))?;

        Ok(Self { token_embedding_table, position_embedding_table, blocks, ln_f, lm_head})
    }

    pub fn forward(&self, idx: &Tensor) -> Result<Tensor> {
        let (_b, t) = idx.dims2()?;
        let tok_emb = self.token_embedding_table.forward(idx)?;

        let pos = Tensor::arange(0u32, t as u32, idx.device())?;
        let pos_emb = self.position_embedding_table.forward(&pos)?;

        let mut x = tok_emb.broadcast_add(&pos_emb)?;
        for block in &self.blocks {
            x = block.forward(&x)?;
        }

        let x = self.ln_f.forward(&x)?;
        self.lm_head.forward(&x)
    }
}

pub fn generate(model: &MicroGPT, mut idx: Tensor, max_new_tokens: usize, block_size: usize) -> Result<Tensor> {
    for _ in 0..max_new_tokens {
        let (_b, t) = idx.dims2()?;
        // Crop context window to block_size if sequence exceeds maximum context length
        let idx_cond = if t > block_size {
            idx.narrow(1, t - block_size, block_size)?
        } else {
            idx.clone()
        };

        // Forward pass to obtain raw unnormalized logits
        let logits = model.forward(&idx_cond)?;

        // Extract logits for the last time step T: (B, C)
        let last_step_logits = logits.narrow(1, logits.dim(1)? - 1, 1)?.squeeze(1)?;

        // Apply softmax along channel dimension
        let probs = candle_nn::ops::softmax(&last_step_logits, D::Minus1)?;

        // Greedy sampling: Select token index with maximum probability
        let idx_next = probs.argmax(D::Minus1)?.unsqueeze(1)?;

        // Append newly predicted token ID to sequence
        idx = Tensor::cat(&[&idx, &idx_next], 1)?;
    }
    Ok(idx)
}