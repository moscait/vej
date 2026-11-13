//! Text generation via greedy token decoding.

use anyhow::{Result, anyhow};
use llama_cpp_2::TokenToStringError;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::token::LlamaToken;
use std::num::NonZeroU32;

use super::limits;
use super::prompt::add_bos_for;
use super::{DecisionEngine, EngineCore};

impl DecisionEngine {
    pub fn generate_text(
        &self,
        user_prompt: &str,
        system: Option<&str>,
        max_tokens: usize,
    ) -> Result<String> {
        self.core.generate_text(user_prompt, system, max_tokens)
    }
}

impl EngineCore {
    /// Raw bytes for a generated token, retrying with the exact buffer size llama.cpp
    /// asks for (non-deprecated equivalent of `token_to_bytes(token, Special::Plaintext)`).
    fn token_piece_bytes(&self, token: LlamaToken) -> Option<Vec<u8>> {
        const INITIAL_PIECE_BUFFER: usize = 8;
        match self
            .model
            .token_to_piece_bytes(token, INITIAL_PIECE_BUFFER, false, None)
        {
            Err(TokenToStringError::InsufficientBufferSpace(needed)) => {
                let size = usize::try_from(needed.unsigned_abs()).ok()?;
                self.model
                    .token_to_piece_bytes(token, size, false, None)
                    .ok()
            }
            other => other.ok(),
        }
    }

    pub fn generate_text(
        &self,
        user_prompt: &str,
        system: Option<&str>,
        max_tokens: usize,
    ) -> Result<String> {
        let clean_prompt = self.neutralize_user_text(user_prompt);
        let clean_system = system.map(|s| self.neutralize_user_text(s));
        let formatted = self.format_generation_prompt(clean_system.as_deref(), &clean_prompt);
        let add_bos = add_bos_for(self.template);
        let prompt_tokens = self
            .model
            .str_to_token(&formatted, add_bos)
            .map_err(|e| anyhow!("Failed to tokenize generation prompt: {:?}", e))?;
        if prompt_tokens.is_empty() {
            return Err(anyhow!(super::types::InvalidRequest(
                "Generation prompt tokenized to zero tokens".to_string()
            )));
        }

        let ctx_cap = limits::generation_context_cap(self.n_ctx, self.model.n_ctx_train()) as usize;
        limits::check_prompt_tokens(prompt_tokens.len(), ctx_cap.saturating_sub(1))?;

        let max_gen = max_tokens.clamp(64, 2048);
        let ctx_size = (prompt_tokens.len() + max_gen + 64)
            .max(self.n_ctx as usize)
            .min(ctx_cap);
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(ctx_size as u32))
            .with_n_batch(ctx_size as u32)
            .with_n_threads(self.threads as i32);

        let mut ctx = self
            .model
            .new_context(self.backend, ctx_params)
            .map_err(|e| anyhow!("Failed to create generation context: {:?}", e))?;

        let mut batch = LlamaBatch::new(prompt_tokens.len() + 16, 1);
        batch
            .add_sequence(&prompt_tokens, 0, false)
            .map_err(|e| anyhow!("Failed to add prompt sequence: {:?}", e))?;
        ctx.decode(&mut batch)
            .map_err(|e| anyhow!("Generation prefill decode failed: {:?}", e))?;

        let mut cur_pos = prompt_tokens.len() as i32;
        let mut output_bytes: Vec<u8> = Vec::new();

        for _ in 0..max_gen {
            let logits = ctx.get_logits();
            let mut best_idx = 0usize;
            let mut best_val = f32::NEG_INFINITY;
            for (idx, &val) in logits.iter().enumerate() {
                if val > best_val {
                    best_val = val;
                    best_idx = idx;
                }
            }
            let next_token = LlamaToken::new(best_idx as i32);
            if self.model.is_eog_token(next_token) {
                break;
            }
            if let Some(bytes) = self.token_piece_bytes(next_token) {
                output_bytes.extend_from_slice(&bytes);
            }
            if cur_pos >= ctx_size as i32 {
                break;
            }
            batch.clear();
            batch
                .add(next_token, cur_pos, &[0], true)
                .map_err(|e| anyhow!("Failed to add generated token to batch: {:?}", e))?;
            cur_pos += 1;
            ctx.decode(&mut batch)
                .map_err(|e| anyhow!("Generation step decode failed: {:?}", e))?;
        }

        Ok(String::from_utf8_lossy(&output_bytes).trim().to_string())
    }
}
