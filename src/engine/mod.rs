//! Inference engine, model loader, prompt construction, and scoring.

mod choices;
mod evaluate;
mod generate;
mod limits;
mod pool;
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
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use crate::model_manager::ModelLoader;
use crate::models::{TemplateStyle, find_model_spec, resolve_model_path};
use types::ChoiceItem;

pub use limits::MAX_REQUEST_BODY_BYTES;
pub use types::{
    BatchDecisionRequest, BatchDecisionResponse, BatchItem, BatchItemResult, DecisionRequest,
    DecisionResponse, InvalidRequest, JevResponse, UnifiedResponse,
};

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
    pub max_concurrent: usize,
}

impl EngineLoader {
    fn resolve_existing_file(model_id: &str) -> Result<PathBuf> {
        let path = resolve_model_path(model_id)
            .with_context(|| format!("Could not resolve model '{model_id}'"))?;
        if !path.is_file() {
            return Err(anyhow!("Model path '{}' is not a file", path.display()));
        }
        Ok(path)
    }
}

impl ModelLoader for EngineLoader {
    type Model = DecisionEngine;

    fn canonical_id(&self, query: &str) -> String {
        find_model_spec(query)
            .map(|spec| spec.id.to_string())
            .unwrap_or_else(|| query.to_string())
    }

    fn check(&self, model_id: &str) -> Result<()> {
        Self::resolve_existing_file(model_id).map(|_| ())
    }

    fn load(&self, model_id: &str) -> Result<DecisionEngine> {
        let path = Self::resolve_existing_file(model_id)?;
        let template = find_model_spec(model_id).map(|spec| spec.template);
        DecisionEngine::load_with_concurrency(
            &path,
            model_id.to_string(),
            template,
            self.n_gpu_layers,
            self.threads,
            self.n_ctx,
            self.max_concurrent,
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

pub(crate) struct EngineCore {
    pub(crate) backend: &'static LlamaBackend,
    pub(crate) model: Arc<LlamaModel>,
    pub(crate) model_id: String,
    pub(crate) template: TemplateStyle,
    pub(crate) threads: u32,
    pub(crate) n_ctx: u32,
    special_guard: prompt::SpecialTokenGuard,
}

/// Loaded decision model paired with persistent inference worker contexts.
pub struct DecisionEngine {
    // Dropped before `core` so workers exit and destroy their `LlamaContext` instances first.
    pool: pool::ContextPool,
    core: Arc<EngineCore>,
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
        Self::load_with_concurrency(
            model_path,
            model_id,
            template,
            n_gpu_layers,
            threads,
            n_ctx,
            1,
        )
    }

    pub fn load_with_concurrency(
        model_path: &Path,
        model_id: String,
        template: Option<TemplateStyle>,
        n_gpu_layers: u32,
        threads: u32,
        n_ctx: u32,
        max_concurrent: usize,
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

        let core = Arc::new(EngineCore {
            backend,
            model: Arc::new(model),
            model_id,
            template: resolved_template,
            threads,
            n_ctx,
            special_guard,
        });
        let pool = pool::ContextPool::start(Arc::clone(&core), max_concurrent)?;

        Ok(Self { pool, core })
    }

    pub fn neutralize_user_text(&self, text: &str) -> String {
        self.core.neutralize_user_text(text)
    }

    /// Returns true if `text` tokenizes (with `AddBos::Never`) into any control or
    /// user-defined special token.
    pub fn contains_special_tokens(&self, text: &str) -> Result<bool> {
        self.core.contains_special_tokens(text)
    }

    /// Returns a control or user-defined special token from the loaded model's vocabulary, if any.
    pub fn sample_special_token(&self) -> Option<String> {
        self.core.sample_special_token()
    }

    pub fn format_generation_prompt(&self, system: Option<&str>, prompt_str: &str) -> String {
        self.core.format_generation_prompt(system, prompt_str)
    }

    pub fn create_context_with_size(
        &self,
        min_size: u32,
        n_sequences: u32,
    ) -> Result<LlamaContext<'_>> {
        self.core.create_context_with_size(min_size, n_sequences)
    }

    pub fn create_context(&self) -> Result<LlamaContext<'_>> {
        self.core.create_context()
    }
}

impl EngineCore {
    pub(crate) fn neutralize_user_text(&self, text: &str) -> String {
        self.special_guard.neutralize(text)
    }

    pub(crate) fn contains_special_tokens(&self, text: &str) -> Result<bool> {
        let tokens = self
            .model
            .str_to_token(text, llama_cpp_2::model::AddBos::Never)
            .map_err(|e| anyhow!("Failed to tokenize probe text: {e:?}"))?;
        Ok(tokens.into_iter().any(|tok| {
            self.model.token_attr(tok).intersects(
                llama_cpp_2::token_type::LlamaTokenAttr::Control
                    | llama_cpp_2::token_type::LlamaTokenAttr::UserDefined,
            )
        }))
    }

    pub(crate) fn sample_special_token(&self) -> Option<String> {
        prompt::collect_special_tokens(&self.model)
            .into_iter()
            .find(|tok| self.contains_special_tokens(tok).unwrap_or(false))
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

    pub(crate) fn format_generation_prompt(
        &self,
        system: Option<&str>,
        prompt_str: &str,
    ) -> String {
        prompt::format_generation_prompt(self.template, system, prompt_str)
    }

    pub(crate) fn create_context_with_size(
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

    pub(crate) fn create_context(&self) -> Result<LlamaContext<'_>> {
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
            max_concurrent: 1,
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
            max_concurrent: 1,
        };
        let err = loader
            .load("/nonexistent/model.gguf")
            .err()
            .expect("missing file fails");
        assert!(format!("{err:#}").contains("not found"), "{err:#}");
    }

    #[test]
    fn engine_loader_check_validates_files_without_loading() {
        struct TempFile(PathBuf);
        impl Drop for TempFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }

        let loader = EngineLoader {
            n_gpu_layers: 0,
            threads: 1,
            n_ctx: 512,
            max_concurrent: 1,
        };
        assert!(loader.check("/nonexistent/model.gguf").is_err());

        let dir = std::env::temp_dir();
        assert!(loader.check(&dir.to_string_lossy()).is_err());

        let temp_path = dir.join(format!("vej-check-test-{}.gguf", std::process::id()));
        std::fs::write(&temp_path, b"not-a-real-gguf").expect("write temp file");
        let _guard = TempFile(temp_path.clone());
        loader
            .check(&temp_path.to_string_lossy())
            .expect("existing file passes check without loading GGUF");
    }
}
