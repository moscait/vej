//! Inference engine, model loader, prompt construction, and scoring.

mod choices;
mod evaluate;
mod generate;
mod limits;
mod prompt;
mod scoring;
mod types;

use anyhow::{Context, Result, anyhow};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use crate::model_manager::ModelLoader;
use crate::models::{TemplateStyle, find_model_spec, resolve_model_path};
use types::ChoiceItem;

pub use limits::MAX_REQUEST_BODY_BYTES;
pub use types::{
    BatchDecisionRequest, BatchDecisionResponse, DecisionRequest, InvalidRequest, UnifiedResponse,
};
#[cfg(test)]
pub use types::{BatchItemResult, DecisionResponse};

/// A loaded model that can answer decision and generation requests.
pub trait DecisionModel: Send + Sync + 'static {
    fn evaluate_unified(&self, req: &DecisionRequest) -> Result<UnifiedResponse>;
    fn evaluate_batch(&self, req: &BatchDecisionRequest) -> Result<BatchDecisionResponse>;
    fn generate_text(
        &self,
        prompt: &str,
        system: Option<&str>,
        max_tokens: usize,
    ) -> Result<String>;
}

/// llama.cpp allows exactly one live backend per process, so it is initialized once and
/// shared by every model load (creating one per load breaks model switching).
pub fn llama_backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|e| format!("{e:?}")))
        .as_ref()
        .map_err(|e| anyhow!("Failed to initialize llama backend: {e}"))
}

/// Loads GGUF models from the catalog cache or a local path.
pub struct EngineLoader {
    pub n_gpu_layers: u32,
    pub threads: u32,
    pub n_ctx: u32,
}

impl ModelLoader for EngineLoader {
    type Model = DecisionEngine;

    fn canonical_id(&self, query: &str) -> String {
        find_model_spec(query)
            .map(|spec| spec.id.to_string())
            .unwrap_or_else(|| query.to_string())
    }

    fn load(&self, model_id: &str) -> Result<DecisionEngine> {
        let path = resolve_model_path(model_id)
            .with_context(|| format!("Could not resolve model '{model_id}'"))?;
        let template = find_model_spec(model_id).map(|spec| spec.template);
        DecisionEngine::load(
            &path,
            model_id.to_string(),
            template,
            self.n_gpu_layers,
            self.threads,
            self.n_ctx,
        )
    }
}

impl DecisionModel for DecisionEngine {
    fn evaluate_unified(&self, req: &DecisionRequest) -> Result<UnifiedResponse> {
        DecisionEngine::evaluate_unified(self, req)
    }

    fn evaluate_batch(&self, req: &BatchDecisionRequest) -> Result<BatchDecisionResponse> {
        DecisionEngine::evaluate_batch(self, req)
    }

    fn generate_text(
        &self,
        prompt: &str,
        system: Option<&str>,
        max_tokens: usize,
    ) -> Result<String> {
        DecisionEngine::generate_text(self, prompt, system, max_tokens)
    }
}

pub struct DecisionEngine {
    backend: &'static LlamaBackend,
    model: Arc<LlamaModel>,
    model_id: String,
    template: TemplateStyle,
    threads: u32,
    n_ctx: u32,
    special_guard: prompt::SpecialTokenGuard,
}

impl DecisionEngine {
    pub fn load(
        model_path: &Path,
        model_id: String,
        template: Option<TemplateStyle>,
        n_gpu_layers: u32,
        threads: u32,
        n_ctx: u32,
    ) -> Result<Self> {
        let backend = llama_backend()?;
        let model_params = LlamaModelParams::default()
            .with_n_gpu_layers(n_gpu_layers)
            .with_use_mmap(true);

        tracing::info!(
            model_id = %model_id,
            path = ?model_path,
            n_gpu_layers = n_gpu_layers,
            "Loading model into memory"
        );

        let start = Instant::now();
        let model = LlamaModel::load_from_file(backend, model_path, &model_params)
            .with_context(|| format!("Failed to load GGUF model from {:?}", model_path))?;
        tracing::info!(elapsed = ?start.elapsed(), "Model successfully loaded");

        let resolved_template = prompt::resolve_template(&model_id, template);
        let special_guard = prompt::SpecialTokenGuard::from_model(&model)?;

        Ok(Self {
            backend,
            model: Arc::new(model),
            model_id,
            template: resolved_template,
            threads,
            n_ctx,
            special_guard,
        })
    }

    pub fn neutralize_user_text(&self, text: &str) -> String {
        self.special_guard.neutralize(text)
    }

    pub(crate) fn neutralize_choices(&self, choices: &[ChoiceItem]) -> Vec<ChoiceItem> {
        choices
            .iter()
            .map(|c| ChoiceItem {
                key: c.key.clone(),
                display: self.neutralize_user_text(&c.display),
            })
            .collect()
    }

    pub(crate) fn plan_scoring(
        &self,
        choices: &[ChoiceItem],
    ) -> std::result::Result<scoring::ScoringPlan, InvalidRequest> {
        scoring::resolve_scoring_plan(self.model.as_ref(), choices)
    }

    pub(crate) fn format_prompt(
        &self,
        instruction: Option<&str>,
        state_str: &str,
        choices: &[ChoiceItem],
        scheme: scoring::LabelScheme,
    ) -> String {
        prompt::format_decision_prompt(self.template, instruction, state_str, choices, scheme)
    }

    pub fn format_generation_prompt(&self, system: Option<&str>, prompt_str: &str) -> String {
        prompt::format_generation_prompt(self.template, system, prompt_str)
    }

    pub fn create_context_with_size(
        &self,
        min_size: u32,
        n_sequences: u32,
    ) -> Result<LlamaContext<'_>> {
        let train_ctx = self.model.n_ctx_train();
        let ctx_size = self.n_ctx.max(min_size).max(2048).min(train_ctx).max(1);
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(ctx_size))
            .with_n_batch(ctx_size)
            .with_n_seq_max(n_sequences.max(64))
            .with_kv_unified(true)
            .with_n_threads(self.threads as i32);

        self.model
            .new_context(self.backend, ctx_params)
            .map_err(|e| anyhow!("Failed to create inference context: {:?}", e))
    }

    pub(crate) fn create_batch_context(
        &self,
        max_chunk_tokens: usize,
        budget: usize,
        n_sequences: u32,
    ) -> Result<LlamaContext<'_>> {
        let desired = (max_chunk_tokens.saturating_add(256)).min(budget).max(1) as u32;
        let ctx_size = self.n_ctx.max(desired).min(budget.max(1) as u32).max(1);
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(ctx_size))
            .with_n_batch(ctx_size)
            .with_n_seq_max(n_sequences.max(64))
            .with_kv_unified(true)
            .with_n_threads(self.threads as i32);

        self.model
            .new_context(self.backend, ctx_params)
            .map_err(|e| anyhow!("Failed to create batch inference context: {:?}", e))
    }

    pub fn create_context(&self) -> Result<LlamaContext<'_>> {
        self.create_context_with_size(0, 1)
    }

    pub(crate) fn score_choices_from_logits(
        &self,
        logits: &[f32],
        choices: &[ChoiceItem],
        plan: &scoring::ScoringPlan,
        temperature: f32,
    ) -> (String, f32, HashMap<String, f32>) {
        scoring::score_choices_from_logits(logits, choices, plan, temperature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llama_backend_is_initialized_once_and_shared() {
        // Regression: per-load `LlamaBackend::init()` fails with BackendAlreadyInitialized
        // whenever a second model loads while the first is still alive.
        let first = llama_backend().expect("backend initializes");
        let second = llama_backend().expect("backend is reusable");
        assert!(std::ptr::eq(first, second));
    }

    #[test]
    fn engine_loader_canonicalizes_catalog_aliases_and_passes_paths_through() {
        let loader = EngineLoader {
            n_gpu_layers: 0,
            threads: 1,
            n_ctx: 512,
        };
        assert_eq!(loader.canonical_id("LLAMA-3.2-1B"), "llama-3.2-1b");
        assert_eq!(
            loader.canonical_id("/models/custom.gguf"),
            "/models/custom.gguf"
        );
    }

    #[test]
    fn engine_loader_reports_missing_models_without_loading() {
        let loader = EngineLoader {
            n_gpu_layers: 0,
            threads: 1,
            n_ctx: 512,
        };
        let err = loader
            .load("/nonexistent/model.gguf")
            .err()
            .expect("missing file fails");
        assert!(format!("{err:#}").contains("not found"), "{err:#}");
    }
}
