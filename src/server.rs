use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};

use crate::access::{self, AccessPolicy};
use crate::engine::{
    BatchDecisionRequest, BatchDecisionResponse, DecisionModel, DecisionRequest, UnifiedResponse,
};
use crate::idle::{self, ActivityTracker};
use crate::keep_alive::KeepAlive;
use crate::model_manager::{ModelLease, ModelLoader, ModelManager};
use crate::models::{RECOMMENDED_MODELS, find_model_spec, get_cache_dir};

type ApiError = (StatusCode, String);

/// Time in-flight requests get to finish after shutdown starts before the process exits.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const DEFAULT_MAX_GENERATED_TOKENS: usize = 1024;

/// Runtime playground assets, compiled into the binary so the daemon works from any
/// working directory (e.g. under launchd/systemd, where cwd is `/`).
#[derive(RustEmbed)]
#[folder = "playground/"]
#[include = "index.html"]
#[include = "src/*"]
struct PlaygroundAssets;

#[derive(Serialize)]
struct ModelInfo {
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
struct SwitchModelRequest {
    model: String,
}

#[derive(Deserialize)]
struct GenerateRequest {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    max_tokens: Option<usize>,
    #[serde(default)]
    keep_alive: Option<KeepAlive>,
}

#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
    /// The loaded model, or the default model that the next request will load.
    active_model: String,
    loaded: bool,
    version: &'static str,
}

#[derive(Serialize)]
struct RunningModel {
    model: String,
    in_flight: usize,
    keep_alive: KeepAlive,
    /// Milliseconds until an idle model unloads; null while in use or when kept forever.
    expires_in_ms: Option<u64>,
}

#[derive(Serialize)]
struct RunningModels {
    models: Vec<RunningModel>,
}

pub struct AppState<L: ModelLoader> {
    pub manager: Arc<ModelManager<L>>,
    /// Bounds concurrent inferences; each one allocates its own KV cache.
    inference_slots: Arc<Semaphore>,
    /// Signalled by `POST /v1/shutdown` and by `--idle-exit`.
    shutdown: Notify,
    /// Request activity, used by `--idle-exit`.
    pub activity: Arc<ActivityTracker>,
}

impl<L: ModelLoader> AppState<L> {
    pub fn new(manager: Arc<ModelManager<L>>, max_concurrent: usize) -> Arc<Self> {
        Arc::new(Self {
            manager,
            inference_slots: Arc::new(Semaphore::new(max_concurrent.max(1))),
            shutdown: Notify::new(),
            activity: ActivityTracker::new(),
        })
    }

    /// The loaded model, or the default that the next request will load.
    fn serving_model(&self) -> (String, bool) {
        match self.manager.status() {
            Some(status) => (status.model_id, true),
            None => (self.manager.default_model(), false),
        }
    }
}

pub fn build_router<L>(state: Arc<AppState<L>>, policy: Arc<AccessPolicy>) -> Router
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    Router::new()
        .route("/", get(root_handler::<L>))
        .route("/health", get(health_handler::<L>))
        .route("/v1/systemone", post(decision_handler::<L>))
        .route("/v1/systemone/batch", post(batch_decision_handler::<L>))
        .route("/v1/batch", post(batch_decision_handler::<L>))
        .route("/v1/generate", post(generate_handler::<L>))
        .route("/api/generate", post(generate_handler::<L>))
        .route("/api/tags", get(api_tags_handler::<L>))
        .route("/v1/models", get(list_models_handler::<L>))
        .route("/v1/models/switch", post(switch_model_handler::<L>))
        .route("/v1/ps", get(running_models_handler::<L>))
        .route("/v1/unload", post(unload_handler::<L>))
        .route("/v1/shutdown", post(shutdown_handler::<L>))
        .route("/shutdown", post(shutdown_handler::<L>))
        .route("/v1/proxy/jev", post(jev_proxy_handler))
        .route(
            "/playground",
            get(|| async { Redirect::temporary("/playground/") }),
        )
        .route(
            "/playground/",
            get(|| async { serve_playground_asset("index.html") }),
        )
        .route(
            "/playground/{*path}",
            get(|Path(path): Path<String>| async move { serve_playground_asset(&path) }),
        )
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state.activity),
            idle::track_activity,
        ))
        .with_state(state)
        .layer(policy.cors_layer())
        .layer(axum::middleware::from_fn_with_state(policy, access::guard))
}

/// Serves on `listener` (bound by vej or handed over by launchd/systemd) until Ctrl-C,
/// SIGTERM, `POST /v1/shutdown`, or `idle_exit` without requests.
pub async fn run_server<L>(
    state: Arc<AppState<L>>,
    listener: std::net::TcpListener,
    host: &str,
    allowed_origins: &[String],
    idle_exit: Option<Duration>,
) -> Result<()>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let port = listener.local_addr()?.port();
    let policy = Arc::new(AccessPolicy::new(host, port, allowed_origins));
    let app = build_router(Arc::clone(&state), policy);
    if let Some(idle) = idle_exit {
        tracing::info!(idle = ?idle, "Will exit after this long without requests");
        spawn_idle_exit(Arc::clone(&state), idle);
    }

    let addr = format!("{host}:{port}");
    tracing::info!("vej Jev-compatible server listening on http://{addr}");
    println!("🚀 vej decision & generation daemon running at http://{addr}");
    println!("📌 POST /v1/systemone       - Evaluate typed decisions");
    println!("📌 POST /v1/systemone/batch - Evaluate batched decisions in a single call");
    println!("📌 POST /v1/generate        - Generate text / JSON from the active GGUF model");
    println!("📌 GET  /v1/models          - List configured models");
    println!("📌 POST /v1/models/switch   - Hot-switch active model");
    println!("📌 GET  /v1/ps              - Show the loaded model and when it unloads");
    println!("📌 POST /v1/unload          - Release the loaded model now");
    println!("📌 POST /v1/shutdown        - Gracefully stop the vej daemon");
    println!("✨ Web Playground           - http://{addr}/playground");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                () = shutdown_signal() => {}
                () = state.shutdown.notified() => {
                    tracing::info!("Shutdown requested (POST /v1/shutdown or idle exit); stopping vej daemon");
                }
            }
            // Don't let a long generation hold the process open indefinitely.
            tokio::spawn(async {
                tokio::time::sleep(SHUTDOWN_GRACE).await;
                tracing::warn!("In-flight requests did not finish in time; exiting");
                std::process::exit(0);
            });
        })
        .await?;
    Ok(())
}

/// Requests shutdown once no request has arrived for `idle`.
fn spawn_idle_exit<L: ModelLoader>(state: Arc<AppState<L>>, idle: Duration) {
    tokio::spawn(async move {
        idle::wait_until_idle_for(&state.activity, idle).await;
        tracing::info!(idle = ?idle, "No requests for the idle-exit period; shutting down");
        state.shutdown.notify_one();
    });
}

/// Runs `work` against a leased model on the blocking pool, within the concurrency limit.
/// The lease is released inside the blocking task, so keep_alive = 0 unloads there.
async fn run_inference<L, T, F>(
    state: &AppState<L>,
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
    let _slot = Arc::clone(&state.inference_slots)
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
        .map_err(|e| internal(format!("Failed to load model: {e:#}")))?;
    tokio::task::spawn_blocking(move || work(&lease))
        .await
        .map_err(|e| internal(format!("{error_label}: task failed: {e}")))?
        .map_err(|e| internal(format!("{error_label}: {e}")))
}

/// Requests may only select catalog models; loading arbitrary paths is reserved for the
/// switch endpoint. Unknown names fall back to the serving model.
fn catalog_model(requested: Option<&str>) -> Option<&'static str> {
    requested.and_then(find_model_spec).map(|spec| spec.id)
}

/// Resolves on Ctrl-C or SIGTERM (what launchd/systemd send on stop).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "Failed to listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("Shutdown signal received; draining in-flight requests");
}

fn serve_playground_asset(path: &str) -> Response {
    match PlaygroundAssets::get(path) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, file.metadata.mimetype().to_string()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            file.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn root_handler<L: ModelLoader>(
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

async fn health_handler<L: ModelLoader>(
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

async fn decision_handler<L>(
    State(state): State<Arc<AppState<L>>>,
    Json(req): Json<DecisionRequest>,
) -> Result<Json<UnifiedResponse>, ApiError>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    let requested = catalog_model(req.model.as_deref());
    let keep_alive = req.keep_alive;
    run_inference(
        &state,
        requested,
        keep_alive,
        "Evaluation error",
        move |lease| lease.model().evaluate_unified(&req),
    )
    .await
    .map(Json)
}

async fn batch_decision_handler<L>(
    State(state): State<Arc<AppState<L>>>,
    Json(req): Json<BatchDecisionRequest>,
) -> Result<Json<BatchDecisionResponse>, ApiError>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    let requested = catalog_model(req.model.as_deref());
    let keep_alive = req.keep_alive;
    run_inference(
        &state,
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
async fn generate_handler<L>(
    State(state): State<Arc<AppState<L>>>,
    Json(req): Json<GenerateRequest>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    if req.prompt.trim().is_empty() {
        if req.keep_alive.is_some_and(|k| k.unloads_immediately()) {
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
        None,
        req.keep_alive,
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

async fn api_tags_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<serde_json::Value> {
    let (active, _) = state.serving_model();
    Json(serde_json::json!({ "models": [{ "name": active }] }))
}

async fn shutdown_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<serde_json::Value> {
    tracing::info!("Received shutdown request");
    // notify_one stores a permit, so the request is never lost even if the server's
    // shutdown future is not currently waiting.
    state.shutdown.notify_one();
    Json(serde_json::json!({ "status": "shutting_down" }))
}

async fn list_models_handler<L: ModelLoader>(
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

async fn switch_model_handler<L: ModelLoader>(
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

async fn running_models_handler<L: ModelLoader>(
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

async fn unload_handler<L: ModelLoader>(
    State(state): State<Arc<AppState<L>>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "success",
        "unloaded": state.manager.unload(),
    }))
}

async fn jev_proxy_handler(headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let client = reqwest::Client::new();
    let mut req = client
        .post("https://api.typesafe.ai/v1/systemone")
        .header(header::CONTENT_TYPE, "application/json");

    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        req = req.header(header::AUTHORIZATION, auth.clone());
    }

    match req.body(body).send().await {
        Ok(res) => {
            let status = StatusCode::from_u16(res.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let bytes = res.bytes().await.unwrap_or_default();
            (status, [(header::CONTENT_TYPE, "application/json")], bytes).into_response()
        }
        Err(e) => {
            let err_json = serde_json::json!({
                "error": format!("Failed to reach TypeSafe AI: {}", e)
            });
            (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                Json(err_json),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keep_alive::KeepAlive;
    use crate::test_support::{FakeLoader, Probe};
    use axum::body::{Body, Bytes};
    use axum::http::{HeaderMap, Request, StatusCode, header};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::time::Duration;
    use tower::ServiceExt;

    fn test_policy() -> Arc<AccessPolicy> {
        Arc::new(AccessPolicy::new("127.0.0.1", 8080, &[]))
    }

    fn app_and_state(
        loader: FakeLoader,
        max_concurrent: usize,
    ) -> (Router, Arc<AppState<FakeLoader>>) {
        let manager = ModelManager::new(loader, "model-a", KeepAlive::default());
        let state = AppState::new(manager, max_concurrent);
        (build_router(Arc::clone(&state), test_policy()), state)
    }

    fn app_with(loader: FakeLoader, max_concurrent: usize) -> Router {
        app_and_state(loader, max_concurrent).0
    }

    fn app() -> (Router, Probe) {
        let (loader, probe) = FakeLoader::new();
        (app_with(loader, 2), probe)
    }

    async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
        let res = app
            .clone()
            .oneshot(req)
            .await
            .expect("router is infallible");
        let status = res.status();
        let headers = res.headers().clone();
        let body = res.into_body().collect().await.expect("body").to_bytes();
        (status, headers, body)
    }

    async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
        let req = Request::get(uri).body(Body::empty()).unwrap();
        let (status, _, body) = send(app, req).await;
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn post_json(app: &Router, uri: &str, payload: Value) -> (StatusCode, Value) {
        let req = Request::post(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap();
        let (status, _, body) = send(app, req).await;
        let value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        (status, value)
    }

    fn decision(extra: Value) -> Value {
        let mut body = json!({ "state": "I was double charged", "choices": ["refund", "support"] });
        if let (Some(obj), Some(more)) = (body.as_object_mut(), extra.as_object()) {
            obj.extend(more.clone());
        }
        body
    }

    #[tokio::test]
    async fn health_reports_default_model_without_loading_it() {
        let (app, probe) = app();

        let (status, body) = get_json(&app, "/health").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["active_model"], "model-a");
        assert_eq!(body["loaded"], false);
        assert!(
            probe.events().is_empty(),
            "health checks must not load models"
        );
    }

    #[tokio::test]
    async fn decision_lazily_loads_model_and_answers() {
        let (app, probe) = app();

        let (status, body) = post_json(&app, "/v1/systemone", decision(json!({}))).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["decision"], "refund");
        assert_eq!(body["model"], "model-a");
        assert_eq!(probe.loads(), ["model-a"]);
        assert_eq!(get_json(&app, "/health").await.1["loaded"], true);
    }

    #[tokio::test]
    async fn decision_with_keep_alive_zero_unloads_after_responding() {
        let (app, probe) = app();

        let (status, _) =
            post_json(&app, "/v1/systemone", decision(json!({ "keep_alive": 0 }))).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(get_json(&app, "/v1/ps").await.1, json!({ "models": [] }));
        assert_eq!(probe.drops(), ["model-a"]);
    }

    #[tokio::test]
    async fn invalid_keep_alive_is_rejected() {
        let (app, probe) = app();

        let (status, _) = post_json(
            &app,
            "/v1/systemone",
            decision(json!({ "keep_alive": "soon" })),
        )
        .await;

        assert!(status.is_client_error(), "got {status}");
        assert!(probe.events().is_empty());
    }

    #[tokio::test]
    async fn ps_lists_loaded_model_with_expiry() {
        let (app, _probe) = app();
        post_json(
            &app,
            "/v1/systemone",
            decision(json!({ "keep_alive": "10m" })),
        )
        .await;

        let (status, body) = get_json(&app, "/v1/ps").await;

        assert_eq!(status, StatusCode::OK);
        let entry = &body["models"][0];
        assert_eq!(entry["model"], "model-a");
        assert_eq!(entry["in_flight"], 0);
        assert_eq!(entry["keep_alive"], "10m");
        let expires_ms = entry["expires_in_ms"]
            .as_u64()
            .expect("idle model has an expiry");
        assert!(
            expires_ms > 590_000 && expires_ms <= 600_000,
            "{expires_ms}"
        );
    }

    #[tokio::test]
    async fn unload_endpoint_releases_the_model() {
        let (app, probe) = app();
        post_json(&app, "/v1/systemone", decision(json!({}))).await;

        let (status, body) = post_json(&app, "/v1/unload", json!({})).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "status": "success", "unloaded": "model-a" }));
        assert_eq!(probe.drops(), ["model-a"]);
        assert_eq!(get_json(&app, "/v1/ps").await.1, json!({ "models": [] }));
    }

    #[tokio::test]
    async fn unload_with_nothing_loaded_reports_null() {
        let (app, _probe) = app();
        let (status, body) = post_json(&app, "/v1/unload", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["unloaded"], Value::Null);
    }

    #[tokio::test]
    async fn request_naming_a_catalog_model_switches_to_it() {
        let (app, probe) = app();

        let (status, body) = post_json(
            &app,
            "/v1/systemone",
            decision(json!({ "model": "llama-3.2-1b" })),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["model"], "llama-3.2-1b");
        assert_eq!(probe.loads(), ["llama-3.2-1b"]);
    }

    #[tokio::test]
    async fn request_naming_an_unknown_model_cannot_load_arbitrary_files() {
        let (app, probe) = app();

        let (status, body) = post_json(
            &app,
            "/v1/systemone",
            decision(json!({ "model": "/etc/passwd" })),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["model"], "model-a");
        assert_eq!(probe.loads(), ["model-a"]);
    }

    #[tokio::test]
    async fn switch_endpoint_loads_the_requested_model() {
        let (app, probe) = app();

        let (status, body) =
            post_json(&app, "/v1/models/switch", json!({ "model": "model-b" })).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({ "status": "success", "switched_to": "model-b" })
        );
        assert_eq!(probe.loads(), ["model-b"]);
        assert_eq!(get_json(&app, "/health").await.1["active_model"], "model-b");
    }

    #[tokio::test]
    async fn switched_model_survives_unload_and_reloads_on_next_request() {
        let (app, probe) = app();
        post_json(&app, "/v1/models/switch", json!({ "model": "model-b" })).await;
        post_json(&app, "/v1/unload", json!({})).await;

        let health = get_json(&app, "/health").await.1;
        assert_eq!(health["active_model"], "model-b");
        assert_eq!(health["loaded"], false);

        let (_, body) = post_json(&app, "/v1/systemone", decision(json!({}))).await;
        assert_eq!(body["model"], "model-b");
        assert_eq!(probe.loads(), ["model-b", "model-b"]);
    }

    #[tokio::test(start_paused = true)]
    async fn requests_are_recorded_as_activity() {
        let (loader, _probe) = FakeLoader::new();
        let (app, state) = app_and_state(loader, 2);

        tokio::time::sleep(Duration::from_secs(10)).await;
        get_json(&app, "/v1/models").await;

        assert_eq!(
            state.activity.idle_since(),
            Some(tokio::time::Instant::now())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn status_probes_do_not_keep_an_idle_daemon_alive() {
        // `vej service status` and supervisors poll these; counting them would mean
        // watching the daemon prevents it from ever idling out.
        let (loader, _probe) = FakeLoader::new();
        let (app, state) = app_and_state(loader, 2);
        let started = tokio::time::Instant::now();

        tokio::time::sleep(Duration::from_secs(10)).await;
        for path in ["/health", "/v1/ps", "/"] {
            get_json(&app, path).await;
        }

        assert_eq!(state.activity.idle_since(), Some(started));
    }

    #[tokio::test(start_paused = true)]
    async fn idle_exit_requests_shutdown_after_a_quiet_period() {
        let (loader, _probe) = FakeLoader::new();
        let (app, state) = app_and_state(loader, 2);
        spawn_idle_exit(Arc::clone(&state), Duration::from_secs(60));

        tokio::time::sleep(Duration::from_secs(50)).await;
        get_json(&app, "/v1/models").await;
        let early = tokio::time::timeout(Duration::from_secs(59), state.shutdown.notified()).await;
        assert!(early.is_err(), "activity at t=50s postpones exit to t=110s");

        let on_time = tokio::time::timeout(Duration::from_secs(2), state.shutdown.notified()).await;
        assert!(on_time.is_ok(), "shutdown requested once idle for 60s");
    }

    #[tokio::test]
    async fn failed_model_load_returns_an_error_status() {
        let (loader, _probe) = FakeLoader::new();
        let app = app_with(loader.failing_first(1), 2);

        let (status, body) = post_json(&app, "/v1/systemone", decision(json!({}))).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            body.as_str()
                .unwrap_or_default()
                .contains("simulated load failure"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn evaluation_errors_return_an_error_status() {
        let (app, _probe) = app();
        let (status, _) = post_json(
            &app,
            "/v1/systemone",
            json!({ "state": "x", "choices": [] }),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn limits_concurrent_evaluations() {
        let (loader, probe) = FakeLoader::new();
        let app = app_with(loader.with_eval_delay(Duration::from_millis(30)), 1);

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let app = app.clone();
            tasks.spawn(async move {
                post_json(&app, "/v1/systemone", decision(json!({})))
                    .await
                    .0
            });
        }
        while let Some(status) = tasks.join_next().await {
            assert_eq!(status.unwrap(), StatusCode::OK);
        }

        assert_eq!(probe.max_concurrent_evaluations(), 1);
    }

    #[tokio::test]
    async fn models_list_marks_the_serving_model_active() {
        let (app, _probe) = app();
        post_json(
            &app,
            "/v1/systemone",
            decision(json!({ "model": "llama-3.2-1b" })),
        )
        .await;

        let (status, body) = get_json(&app, "/v1/models").await;

        assert_eq!(status, StatusCode::OK);
        let active: Vec<&str> = body
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["is_active"] == true)
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(active, ["llama-3.2-1b"]);
    }

    #[tokio::test]
    async fn serves_embedded_playground_assets() {
        let (app, _probe) = app();

        let (status, headers, body) = send(
            &app,
            Request::get("/playground/").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert!(String::from_utf8_lossy(&body).contains("<html"));

        let (status, headers, _) = send(
            &app,
            Request::get("/playground/src/app.js?v=11")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .contains("javascript")
        );

        let (status, _, _) = send(
            &app,
            Request::get("/playground/package.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "only runtime assets are embedded"
        );
    }

    #[tokio::test]
    async fn root_redirects_browsers_to_playground_and_reports_health_otherwise() {
        let (app, _probe) = app();

        let browser = Request::get("/")
            .header(header::ACCEPT, "text/html")
            .body(Body::empty())
            .unwrap();
        let (status, headers, _) = send(&app, browser).await;
        assert!(status.is_redirection());
        assert_eq!(headers[header::LOCATION], "/playground/");

        let (status, body) = get_json(&app, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");

        let (status, headers, _) = send(
            &app,
            Request::get("/playground").body(Body::empty()).unwrap(),
        )
        .await;
        assert!(status.is_redirection());
        assert_eq!(headers[header::LOCATION], "/playground/");
    }

    fn batch(extra: Value) -> Value {
        let mut body = json!({
            "choices": ["relevant", "unrelated"],
            "items": [
                { "id": "jn-3-16", "state": "For God so loved the world" },
                { "id": "ps-23-1", "state": "The Lord is my shepherd", "choices": ["comfort", "law"] },
            ],
        });
        if let (Some(obj), Some(more)) = (body.as_object_mut(), extra.as_object()) {
            obj.extend(more.clone());
        }
        body
    }

    #[tokio::test]
    async fn batch_endpoints_evaluate_every_item_through_the_manager() {
        for path in ["/v1/systemone/batch", "/v1/batch"] {
            let (app, probe) = app();

            let (status, body) = post_json(&app, path, batch(json!({}))).await;

            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            assert_eq!(body["model"], "model-a");
            let results = body["results"].as_array().unwrap();
            assert_eq!(results.len(), 2);
            assert_eq!(results[0]["id"], "jn-3-16");
            assert_eq!(results[0]["decision"], "relevant");
            assert_eq!(results[1]["decision"], "comfort", "per-item choices win");
            assert_eq!(probe.loads(), ["model-a"]);
        }
    }

    #[tokio::test]
    async fn batch_honors_keep_alive() {
        let (app, probe) = app();

        let (status, _) = post_json(
            &app,
            "/v1/systemone/batch",
            batch(json!({ "keep_alive": 0 })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(probe.drops(), ["model-a"]);
    }

    #[tokio::test]
    async fn generate_endpoints_return_text_from_the_serving_model() {
        for path in ["/v1/generate", "/api/generate"] {
            let (app, probe) = app();

            let (status, body) =
                post_json(&app, path, json!({ "prompt": "Summarize John 3:16" })).await;

            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            assert_eq!(body["model"], "model-a");
            assert_eq!(body["response"], "echo: Summarize John 3:16");
            assert_eq!(body["done"], true);
            assert_eq!(probe.loads(), ["model-a"]);
        }
    }

    #[tokio::test]
    async fn generate_with_empty_prompt_and_keep_alive_zero_unloads_like_ollama() {
        let (app, probe) = app();
        post_json(&app, "/v1/systemone", decision(json!({}))).await;

        let (status, body) = post_json(
            &app,
            "/api/generate",
            json!({ "model": "gemma-4-E2B-it-Q4_K_M.gguf", "keep_alive": 0 }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["done_reason"], "unload");
        assert_eq!(body["model"], "model-a");
        assert_eq!(probe.drops(), ["model-a"]);
        assert_eq!(get_json(&app, "/v1/ps").await.1, json!({ "models": [] }));
    }

    #[tokio::test]
    async fn generate_with_empty_prompt_does_not_load_a_model() {
        let (app, probe) = app();

        let (status, body) = post_json(&app, "/api/generate", json!({ "prompt": "  " })).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "response": "", "done": true }));
        assert!(probe.events().is_empty());
    }

    #[tokio::test]
    async fn api_tags_lists_the_serving_model() {
        let (app, _probe) = app();
        let (status, body) = get_json(&app, "/api/tags").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "models": [{ "name": "model-a" }] }));
    }

    #[tokio::test]
    async fn shutdown_endpoints_signal_the_server() {
        for path in ["/v1/shutdown", "/shutdown"] {
            let (loader, _probe) = FakeLoader::new();
            let (app, state) = app_and_state(loader, 2);

            let (status, body) = post_json(&app, path, json!({})).await;

            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, json!({ "status": "shutting_down" }));
            tokio::time::timeout(Duration::from_millis(100), state.shutdown.notified())
                .await
                .unwrap_or_else(|_| panic!("{path} did not signal shutdown"));
        }
    }

    #[tokio::test]
    async fn access_guard_rejects_cross_site_and_rebinding_requests() {
        let (app, probe) = app();
        let request = |host: &str, origin: Option<&str>| {
            let mut req = Request::post("/v1/shutdown").header(header::HOST, host);
            if let Some(origin) = origin {
                req = req.header(header::ORIGIN, origin);
            }
            req.body(Body::empty()).unwrap()
        };

        let (status, _, _) =
            send(&app, request("127.0.0.1:8080", Some("http://evil.example"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "cross-site browser POST");

        let (status, _, _) = send(&app, request("evil.example:8080", None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "DNS-rebinding Host");

        let (status, _, _) = send(
            &app,
            request("127.0.0.1:8080", Some("http://127.0.0.1:8080")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "same-origin playground request");
        assert!(probe.events().is_empty());
    }
}
