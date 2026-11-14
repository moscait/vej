//! Test doubles for the model lifecycle and HTTP layers, so tests run without GGUF files.

use anyhow::{Result, anyhow};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::engine::{
    BatchDecisionRequest, BatchDecisionResponse, BatchItemResult, DecisionModel, DecisionRequest,
    DecisionResponse, UnifiedResponse,
};
use crate::model_manager::ModelLoader;

/// Shared, inspectable record of what the fake loader and its models did.
#[derive(Clone, Default)]
pub struct Probe {
    events: Arc<Mutex<Vec<String>>>,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    active_loads: Arc<AtomicUsize>,
    max_active_loads: Arc<AtomicUsize>,
}

impl Probe {
    fn record(&self, event: String) {
        self.events.lock().expect("probe lock").push(event);
    }

    /// Chronological log, e.g. `["load model-a", "drop model-a", "load model-b"]`.
    pub fn events(&self) -> Vec<String> {
        self.events.lock().expect("probe lock").clone()
    }

    pub fn loads(&self) -> Vec<String> {
        self.filtered("load ")
    }

    pub fn drops(&self) -> Vec<String> {
        self.filtered("drop ")
    }

    pub fn max_concurrent_evaluations(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }

    pub fn max_concurrent_loads(&self) -> usize {
        self.max_active_loads.load(Ordering::SeqCst)
    }

    fn filtered(&self, prefix: &str) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|e| e.strip_prefix(prefix).map(str::to_string))
            .collect()
    }
}

#[derive(Clone, Default)]
enum FakeEvalOutcome {
    #[default]
    Ok,
    Invalid(String),
    Internal(String),
}

pub struct FakeModel {
    id: String,
    eval_delay: Duration,
    probe: Probe,
    eval_outcome: FakeEvalOutcome,
}

impl Drop for FakeModel {
    fn drop(&mut self) {
        self.probe.record(format!("drop {}", self.id));
    }
}

impl FakeModel {
    /// Picks the first choice, tracking how many evaluations run at once.
    fn decide(&self, choices: Option<&Vec<String>>) -> Result<DecisionResponse> {
        let now_active = self.probe.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.probe
            .max_active
            .fetch_max(now_active, Ordering::SeqCst);
        std::thread::sleep(self.eval_delay);
        self.probe.active.fetch_sub(1, Ordering::SeqCst);

        match &self.eval_outcome {
            FakeEvalOutcome::Ok => {}
            FakeEvalOutcome::Invalid(msg) => {
                return Err(anyhow!(crate::engine::InvalidRequest(msg.clone())));
            }
            FakeEvalOutcome::Internal(msg) => return Err(anyhow!("{msg}")),
        }

        let decision = choices
            .and_then(|c| c.first())
            .cloned()
            .ok_or_else(|| anyhow!("Choices list cannot be empty"))?;
        Ok(DecisionResponse {
            probabilities: HashMap::from([(decision.clone(), 1.0)]),
            decision,
            confidence: 1.0,
            model: self.id.clone(),
            latency_ms: 0.0,
            tokens_evaluated: 1,
        })
    }
}

impl DecisionModel for FakeModel {
    fn evaluate_unified(&self, req: &DecisionRequest) -> Result<UnifiedResponse> {
        self.decide(req.choices.as_ref()).map(UnifiedResponse::Flat)
    }

    fn evaluate_batch(&self, req: &BatchDecisionRequest) -> Result<BatchDecisionResponse> {
        let results = req
            .items
            .iter()
            .map(|item| {
                let choices = item.choices.as_ref().or(req.choices.as_ref());
                Ok(BatchItemResult {
                    id: item.id.clone(),
                    response: UnifiedResponse::Flat(self.decide(choices)?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(BatchDecisionResponse {
            model: self.id.clone(),
            total_tokens: results.len(),
            results,
            latency_ms: 0.0,
        })
    }

    fn generate_text(
        &self,
        prompt: &str,
        system: Option<&str>,
        _max_tokens: usize,
    ) -> Result<String> {
        match &self.eval_outcome {
            FakeEvalOutcome::Ok => match system.map(str::trim).filter(|s| !s.is_empty()) {
                Some(sys) => Ok(format!("echo [{sys}]: {prompt}")),
                None => Ok(format!("echo: {prompt}")),
            },
            FakeEvalOutcome::Invalid(msg) => {
                Err(anyhow!(crate::engine::InvalidRequest(msg.clone())))
            }
            FakeEvalOutcome::Internal(msg) => Err(anyhow!("{msg}")),
        }
    }
}

#[derive(Default)]
pub struct FakeLoader {
    probe: Probe,
    load_delay: Duration,
    eval_delay: Duration,
    failures_remaining: AtomicUsize,
    failing_checks: HashSet<String>,
    eval_outcome: FakeEvalOutcome,
}

impl FakeLoader {
    pub fn new() -> (Self, Probe) {
        let loader = Self::default();
        let probe = loader.probe.clone();
        (loader, probe)
    }

    pub fn with_load_delay(mut self, delay: Duration) -> Self {
        self.load_delay = delay;
        self
    }

    pub fn with_eval_delay(mut self, delay: Duration) -> Self {
        self.eval_delay = delay;
        self
    }

    pub fn failing_first(self, attempts: usize) -> Self {
        self.failures_remaining.store(attempts, Ordering::SeqCst);
        self
    }

    pub fn with_failing_checks<I, S>(mut self, model_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.failing_checks
            .extend(model_ids.into_iter().map(Into::into));
        self
    }

    pub fn with_eval_invalid_request(mut self, msg: impl Into<String>) -> Self {
        self.eval_outcome = FakeEvalOutcome::Invalid(msg.into());
        self
    }

    pub fn with_eval_internal_error(mut self, msg: impl Into<String>) -> Self {
        self.eval_outcome = FakeEvalOutcome::Internal(msg.into());
        self
    }
}

impl ModelLoader for FakeLoader {
    type Model = FakeModel;

    fn canonical_id(&self, query: &str) -> String {
        match query.trim().to_lowercase().as_str() {
            "a-alias" => "model-a".to_string(),
            other => other.to_string(),
        }
    }

    fn check(&self, model_id: &str) -> Result<()> {
        if self.failing_checks.contains(model_id) {
            return Err(anyhow!("model '{model_id}' failed preflight check"));
        }
        Ok(())
    }

    fn load(&self, model_id: &str) -> Result<FakeModel> {
        struct ActiveLoad<'a>(&'a AtomicUsize);
        impl Drop for ActiveLoad<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }

        self.check(model_id)?;
        let now_active = self.probe.active_loads.fetch_add(1, Ordering::SeqCst) + 1;
        self.probe
            .max_active_loads
            .fetch_max(now_active, Ordering::SeqCst);
        let _active = ActiveLoad(&self.probe.active_loads);
        std::thread::sleep(self.load_delay);
        // Take one simulated failure if any remain. A manual CAS loop: `fetch_update` is
        // deprecated on current stable and its replacement is missing on older toolchains.
        let should_fail = {
            let remaining = &self.failures_remaining;
            let mut current = remaining.load(Ordering::SeqCst);
            loop {
                let Some(next) = current.checked_sub(1) else {
                    break false;
                };
                match remaining.compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
                {
                    Ok(_) => break true,
                    Err(actual) => current = actual,
                }
            }
        };
        if should_fail {
            self.probe.record(format!("load-failed {model_id}"));
            return Err(anyhow!("simulated load failure for {model_id}"));
        }
        self.probe.record(format!("load {model_id}"));
        Ok(FakeModel {
            id: model_id.to_string(),
            eval_delay: self.eval_delay,
            probe: self.probe.clone(),
            eval_outcome: self.eval_outcome.clone(),
        })
    }
}
