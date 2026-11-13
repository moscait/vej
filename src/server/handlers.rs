//! Route handlers. Inference runs on the blocking pool via [`run_inference`].

use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::{AppState, ClientPolicy};
use crate::engine::{
    BatchDecisionRequest, BatchDecisionResponse, DecisionModel, DecisionRequest, UnifiedResponse,
};
use crate::keep_alive::KeepAlive;
use crate::model_manager::{ModelLease, ModelLoader};
use crate::models::{RECOMMENDED_MODELS, find_model_spec, get_cache_dir};

pub(super) type ApiError = (StatusCode, String);

const DEFAULT_MAX_GENERATED_TOKENS: usize = 1024;

#[derive(Serialize)]
pub(super) struct ModelInfo {
    id: &'static str,
    name: &'static str,
    vendor: &'static str,
    country: &'static str,
    size_gb: f32,
    aliases: &'static [&'static str],
    is_downloaded: bool,
    is_active: bool,
}

#[derive(Deserialize)]
pub(super) struct SwitchModelRequest {
    model: String,
}

#[derive(Deserialize)]
pub(super) struct GenerateRequest {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    max_tokens: Option<usize>,
    #[serde(default)]
    keep_alive: Option<KeepAlive>,
}

#[derive(Serialize)]
pub(super) struct StatusResponse {
    status: &'static str,
    /// The loaded model, or the default model that the next request will load.
    active_model: String,
    loaded: bool,
    version: &'static str,
}

#[derive(Serialize)]
pub(super) struct RunningModel {
    model: String,
    in_flight: usize,
    keep_alive: KeepAlive,
    /// Milliseconds until an idle model unloads; null while in use or when kept forever.
    expires_in_ms: Option<u64>,
}

#[derive(Serialize)]
pub(super) struct RunningModels {
    models: Vec<RunningModel>,
}

/// The slot and lease are released inside the blocking task, so client disconnects keep the
/// permit held until inference finishes and `keep_alive = 0` unloads there.
async fn run_inference<L, T, F>(
    state: &AppState<L>,
    policy: ClientPolicy,
    requested: Option<&str>,
    keep_alive: Option<KeepAlive>,
    error_label: &'static str,
    work: F,
) -> Result<T, ApiError>
where
    L: ModelLoader,
    T: Send + 'static,
    F: FnOnce(&ModelLease<L::Model>) -> anyhow::Result<T> + Send + 'static,
{
    let internal = |msg: String| (StatusCode::INTERNAL_SERVER_ERROR, msg);
    let slot = Arc::clone(&state.inference_slots)
        .acquire_owned()
        .await
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Server is shutting down".to_string(),
            )
        })?;
    let lease = state
        .manager
        .acquire(requested, keep_alive)
        .await
        .map_err(|e| model_unavailable(policy, &e))?;
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let lease = lease;
        work(&lease)
    })
    .await
    .map_err(|e| internal(format!("{error_label}: task failed: {e}")))?
    .map_err(|e| internal(format!("{error_label}: {e}")))
}

/// Load errors name model files and paths, which remote clients must not see.
fn model_unavailable(policy: ClientPolicy, error: &anyhow::Error) -> ApiError {
    if policy.expose_internal_errors {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load model: {error:#}"),
        );
    }
    tracing::error!(error = %format!("{error:#}"), "Model load failed for a remote request");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Model is unavailable; see the vej server logs".to_string(),
    )
}

impl ClientPolicy {
    fn requested_model(self, requested: Option<&str>) -> Option<&'static str> {
        if self.allow_model_selection {
            catalog_model(requested)
        } else {
            None
        }
    }

    fn keep_alive(self, keep_alive: Option<KeepAlive>) -> Option<KeepAlive> {
        if self.allow_keep_alive {
            keep_alive
        } else {
            None
        }
    }
}

/// Requests may only select catalog models; loading arbitrary paths is reserved for the
/// switch endpoint. Unknown names fall back to the serving model.
fn catalog_model(requested: Option<&str>) -> Option<&'static str> {
    requested.and_then(find_model_spec).map(|spec| spec.id)
}

pub(super) async fn root_handler<L: ModelLoader>(
    headers: HeaderMap,
    state: State<Arc<AppState<L>>>,
) -> Response {
    let accepts_html = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"));
    if accepts_html {
        return Redirect::temporary("/playground/").into_response();
    }
    health_handler(state).await.into_response()
}

pub(super) async fn health_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<StatusResponse> {
    let (active_model, loaded) = state.serving_model();
    Json(StatusResponse {
        status: "ok",
        active_model,
        loaded,
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub(super) async fn decision_handler<L>(
    State(state): State<Arc<AppState<L>>>,
    Extension(policy): Extension<ClientPolicy>,
    Json(req): Json<DecisionRequest>,
) -> Result<Json<UnifiedResponse>, ApiError>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    let requested = policy.requested_model(req.model.as_deref());
    let keep_alive = policy.keep_alive(req.keep_alive);
    run_inference(
        &state,
        policy,
        requested,
        keep_alive,
        "Evaluation error",
        move |lease| lease.model().evaluate_unified(&req),
    )
    .await
    .map(Json)
}

pub(super) async fn batch_decision_handler<L>(
    State(state): State<Arc<AppState<L>>>,
    Extension(policy): Extension<ClientPolicy>,
    Json(req): Json<BatchDecisionRequest>,
) -> Result<Json<BatchDecisionResponse>, ApiError>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    let requested = policy.requested_model(req.model.as_deref());
    let keep_alive = policy.keep_alive(req.keep_alive);
    run_inference(
        &state,
        policy,
        requested,
        keep_alive,
        "Batch evaluation error",
        move |lease| lease.model().evaluate_batch(&req),
    )
    .await
    .map(Json)
}

/// Ollama-compatible generation. An empty prompt with `keep_alive: 0` unloads the model,
/// which is how Ollama clients (e.g. Wordstudy) release it.
pub(super) async fn generate_handler<L>(
    State(state): State<Arc<AppState<L>>>,
    Extension(policy): Extension<ClientPolicy>,
    Json(req): Json<GenerateRequest>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    let keep_alive = policy.keep_alive(req.keep_alive);
    if req.prompt.trim().is_empty() {
        if keep_alive.is_some_and(|k| k.unloads_immediately()) {
            let unloaded = state.manager.unload();
            return Ok(Json(serde_json::json!({
                "model": unloaded,
                "response": "",
                "done": true,
                "done_reason": "unload",
            })));
        }
        return Ok(Json(serde_json::json!({ "response": "", "done": true })));
    }

    let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_GENERATED_TOKENS);
    let prompt = req.prompt;
    let (model, text) = run_inference(
        &state,
        policy,
        None,
        keep_alive,
        "Generation error",
        move |lease| {
            let text = lease.model().generate_text(&prompt, max_tokens)?;
            Ok((lease.model_id().to_string(), text))
        },
    )
    .await?;
    Ok(Json(serde_json::json!({
        "model": model,
        "response": text,
        "done": true,
    })))
}

pub(super) async fn api_tags_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<serde_json::Value> {
    let (active, _) = state.serving_model();
    Json(serde_json::json!({ "models": [{ "name": active }] }))
}

pub(super) async fn shutdown_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<serde_json::Value> {
    tracing::info!("Received shutdown request");
    // notify_one stores a permit, so the request is never lost even if the server's
    // shutdown future is not currently waiting.
    state.shutdown.notify_one();
    Json(serde_json::json!({ "status": "shutting_down" }))
}

pub(super) async fn list_models_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<Vec<ModelInfo>> {
    let (active_id, _) = state.serving_model();
    let cache_dir = get_cache_dir();
    let models = RECOMMENDED_MODELS
        .iter()
        .map(|m| ModelInfo {
            id: m.id,
            name: m.name,
            vendor: m.vendor,
            country: m.country,
            size_gb: m.size_gb,
            aliases: m.aliases,
            is_downloaded: cache_dir.join(m.filename).exists(),
            is_active: m.id.eq_ignore_ascii_case(&active_id),
        })
        .collect();
    Json(models)
}

pub(super) async fn switch_model_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
    Json(req): Json<SwitchModelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let model_id = req.model.trim();
    if model_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'model' must not be empty".to_string(),
        ));
    }
    let lease = state
        .manager
        .acquire(Some(model_id), None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load model: {e:#}"),
            )
        })?;
    Ok(Json(serde_json::json!({
        "status": "success",
        "switched_to": lease.model_id(),
    })))
}

pub(super) async fn running_models_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<RunningModels> {
    let models = state
        .manager
        .status()
        .map(|s| RunningModel {
            model: s.model_id,
            in_flight: s.in_flight,
            keep_alive: s.keep_alive,
            expires_in_ms: s
                .expires_in
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
        })
        .into_iter()
        .collect();
    Json(RunningModels { models })
}

pub(super) async fn unload_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "unloaded": state.manager.unload(),
    }))
}

const JEV_SYSTEMONE_URL: &str = "https://api.typesafe.ai/v1/systemone";
const JEV_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const JEV_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) fn build_jev_client(
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(connect_timeout)
        .timeout(request_timeout)
        .build()
        .map_err(|e| e.to_string())
}

pub(super) fn shared_jev_client() -> Result<&'static reqwest::Client, &'static str> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| build_jev_client(JEV_CONNECT_TIMEOUT, JEV_REQUEST_TIMEOUT))
        .as_ref()
        .map_err(String::as_str)
}

fn jev_error_response(status: StatusCode, message: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

fn map_jev_error(err: &reqwest::Error) -> Response {
    let status = if err.is_timeout() {
        StatusCode::GATEWAY_TIMEOUT
    } else {
        StatusCode::BAD_GATEWAY
    };
    jev_error_response(status, format!("Failed to reach TypeSafe AI: {err}"))
}

pub(super) async fn forward_jev_request(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let mut req = client
        .post(url)
        .header(header::CONTENT_TYPE, "application/json");

    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        req = req.header(header::AUTHORIZATION, auth.clone());
    }

    let res = match req.body(body).send().await {
        Ok(res) => res,
        Err(e) => return map_jev_error(&e),
    };
    let status =
        StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match res.bytes().await {
        Ok(bytes) => (status, [(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
        Err(e) => map_jev_error(&e),
    }
}

pub(super) async fn jev_proxy_handler(headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let client = match shared_jev_client() {
        Ok(client) => client,
        Err(e) => {
            return jev_error_response(
                StatusCode::BAD_GATEWAY,
                format!("Failed to reach TypeSafe AI: {e}"),
            );
        }
    };
    forward_jev_request(client, JEV_SYSTEMONE_URL, &headers, body).await
}
