use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::engine::{BatchDecisionRequest, DecisionEngine, DecisionRequest};
use crate::models::{RECOMMENDED_MODELS, find_model_spec, get_cache_dir, resolve_model_path};

pub struct ServerState {
    pub engine: RwLock<DecisionEngine>,
    pub config: Config,
    pub shutdown_notify: Arc<tokio::sync::Notify>,
}

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    pub country: &'static str,
    pub size_gb: f32,
    pub aliases: &'static [&'static str],
    pub is_downloaded: bool,
    pub is_active: bool,
}

#[derive(Deserialize)]
pub struct SwitchModelRequest {
    pub model: String,
}

#[derive(Deserialize)]
pub struct GenerateRequest {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub keep_alive: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub status: &'static str,
    pub active_model: String,
    pub version: &'static str,
}

use crate::access::{self, AccessPolicy};
use tower_http::services::ServeDir;

pub async fn run_server(
    engine: DecisionEngine,
    config: Config,
    host: &str,
    port: u16,
    allowed_origins: &[String],
) -> Result<()> {
    let policy = Arc::new(AccessPolicy::new(host, port, allowed_origins));
    let shutdown_notify = Arc::new(tokio::sync::Notify::new());
    let state = Arc::new(ServerState {
        engine: RwLock::new(engine),
        config,
        shutdown_notify: shutdown_notify.clone(),
    });

    let playground_dir = std::path::Path::new("playground");
    let mut app = Router::new()
        .route("/health", get(health_handler))
        .route("/v1/systemone", post(decision_handler))
        .route("/v1/systemone/batch", post(batch_decision_handler))
        .route("/v1/batch", post(batch_decision_handler))
        .route("/v1/generate", post(generate_handler))
        .route("/api/generate", post(generate_handler))
        .route("/api/tags", get(api_tags_handler))
        .route("/v1/models", get(list_models_handler))
        .route("/v1/models/switch", post(switch_model_handler))
        .route("/v1/shutdown", post(shutdown_handler))
        .route("/shutdown", post(shutdown_handler))
        .route("/v1/proxy/jev", post(jev_proxy_handler));

    if playground_dir.exists() {
        app = app
            .nest_service("/playground", ServeDir::new("playground"))
            .route("/", get(root_handler));
    } else {
        app = app.route("/", get(health_handler));
    }

    let app = app
        .with_state(state)
        .layer(policy.cors_layer())
        .layer(axum::middleware::from_fn_with_state(policy, access::guard));

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("vej Jev-compatible server listening on http://{}", addr);
    println!("🚀 vej decision & generation daemon running at http://{}", addr);
    println!("📌 POST /v1/systemone       - Evaluate typed decisions");
    println!("📌 POST /v1/systemone/batch - Evaluate batched decisions in a single call");
    println!("📌 POST /v1/generate        - Generate text / JSON from active GGUF model");
    println!("📌 GET  /v1/models          - List configured models");
    println!("📌 POST /v1/models/switch   - Hot-switch active model");
    println!("📌 POST /v1/shutdown        - Gracefully stop the vej daemon");
    if playground_dir.exists() {
        println!("✨ Web Playground    - http://{}/playground", addr);
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_notify.notified().await;
            tracing::info!("Shutdown requested via HTTP; stopping vej daemon...");
        })
        .await?;
    Ok(())
}

async fn api_tags_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let engine = state.engine.read().await;
    let active = engine.current_model().to_string();
    Json(serde_json::json!({
        "models": [
            { "name": active }
        ]
    }))
}

async fn generate_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<GenerateRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if req.prompt.trim().is_empty() {
        return Ok(Json(serde_json::json!({
            "response": "",
            "done": true
        })));
    }
    let engine = state.engine.read().await;
    let max_tokens = req.max_tokens.unwrap_or(1024);
    let text = engine
        .generate_text(&req.prompt, max_tokens)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Generation error: {}", e)))?;
    Ok(Json(serde_json::json!({
        "model": engine.current_model(),
        "response": text,
        "done": true
    })))
}

async fn shutdown_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    tracing::info!("Received POST /v1/shutdown request");
    state.shutdown_notify.notify_waiters();
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        std::process::exit(0);
    });
    Json(serde_json::json!({
        "status": "shutting_down"
    }))
}

async fn root_handler(
    headers: axum::http::HeaderMap,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let accepts_html = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false);

    if accepts_html && std::path::Path::new("playground/index.html").exists() {
        return axum::response::Redirect::temporary("/playground/").into_response();
    }

    health_handler(State(state)).await.into_response()
}

async fn health_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let engine = state.engine.read().await;
    Json(StatusResponse {
        status: "ok",
        active_model: engine.current_model().to_string(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn decision_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DecisionRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let engine = state.engine.read().await;

    // If request specifies a specific model and it's different from the active one:
    if let Some(ref req_model) = req.model {
        if !req_model.eq_ignore_ascii_case(engine.current_model()) {
            if let Some(spec) = find_model_spec(req_model) {
                if !spec.id.eq_ignore_ascii_case(engine.current_model()) {
                    drop(engine);
                    let mut write_engine = state.engine.write().await;
                    if !spec.id.eq_ignore_ascii_case(write_engine.current_model()) {
                        let path = resolve_model_path(spec.id).map_err(|e| {
                            (StatusCode::BAD_REQUEST, format!("Model resolution error: {}", e))
                        })?;
                        let backend = write_engine.backend();
                        *write_engine = DecisionEngine::load_with_backend(
                            backend,
                            &path,
                            spec.id.to_string(),
                            Some(spec.template),
                            state.config.n_gpu_layers,
                            state.config.threads,
                            state.config.n_ctx,
                        )
                        .map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Failed to hot-switch model: {}", e),
                            )
                        })?;
                    }
                    let res = write_engine.evaluate_unified(&req).map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("Evaluation error: {}", e))
                    })?;
                    return Ok(Json(res));
                }
            }
        }
    }

    let res = engine
        .evaluate_unified(&req)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Evaluation error: {}", e)))?;

    Ok(Json(res))
}

async fn batch_decision_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<BatchDecisionRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let engine = state.engine.read().await;

    // If request specifies a specific model and it's different from the active one:
    if let Some(ref req_model) = req.model {
        if !req_model.eq_ignore_ascii_case(engine.current_model()) {
            if let Some(spec) = find_model_spec(req_model) {
                if !spec.id.eq_ignore_ascii_case(engine.current_model()) {
                    drop(engine);
                    let mut write_engine = state.engine.write().await;
                    if !spec.id.eq_ignore_ascii_case(write_engine.current_model()) {
                        let path = resolve_model_path(spec.id).map_err(|e| {
                            (StatusCode::BAD_REQUEST, format!("Model resolution error: {}", e))
                        })?;
                        let backend = write_engine.backend();
                        *write_engine = DecisionEngine::load_with_backend(
                            backend,
                            &path,
                            spec.id.to_string(),
                            Some(spec.template),
                            state.config.n_gpu_layers,
                            state.config.threads,
                            state.config.n_ctx,
                        )
                        .map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Failed to hot-switch model: {}", e),
                            )
                        })?;
                    }
                    let res = write_engine.evaluate_batch(&req).map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("Evaluation error: {}", e))
                    })?;
                    return Ok(Json(res));
                }
            }
        }
    }

    let res = engine
        .evaluate_batch(&req)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Batch evaluation error: {}", e)))?;

    Ok(Json(res))
}

async fn list_models_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let engine = state.engine.read().await;
    let active_id = engine.current_model();
    let cache_dir = get_cache_dir();

    let models: Vec<ModelInfo> = RECOMMENDED_MODELS
        .iter()
        .map(|m| {
            let p = cache_dir.join(m.filename);
            ModelInfo {
                id: m.id,
                name: m.name,
                vendor: m.vendor,
                country: m.country,
                size_gb: m.size_gb,
                aliases: m.aliases,
                is_downloaded: p.exists(),
                is_active: m.id.eq_ignore_ascii_case(active_id),
            }
        })
        .collect();

    Json(models)
}

async fn switch_model_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<SwitchModelRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut engine = state.engine.write().await;
    let model_id = req.model.trim();

    let path = resolve_model_path(model_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Model error: {}", e)))?;

    let spec = find_model_spec(model_id);
    let id_str = spec.map(|s| s.id).unwrap_or(model_id).to_string();
    let tmpl = spec.map(|s| s.template);

    let backend = engine.backend();
    *engine = DecisionEngine::load_with_backend(
        backend,
        &path,
        id_str.clone(),
        tmpl,
        state.config.n_gpu_layers,
        state.config.threads,
        state.config.n_ctx,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load model: {}", e),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "success",
        "switched_to": id_str
    })))
}

async fn jev_proxy_handler(
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let client = reqwest::Client::new();
    let mut req = client
        .post("https://api.typesafe.ai/v1/systemone")
        .header(axum::http::header::CONTENT_TYPE, "application/json");

    if let Some(auth) = headers.get(axum::http::header::AUTHORIZATION) {
        req = req.header(axum::http::header::AUTHORIZATION, auth.clone());
    }

    match req.body(body).send().await {
        Ok(res) => {
            let status = StatusCode::from_u16(res.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let bytes = res.bytes().await.unwrap_or_default();
            (
                status,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                bytes,
            )
                .into_response()
        }
        Err(e) => {
            let err_json = serde_json::json!({
                "error": format!("Failed to reach TypeSafe AI: {}", e)
            });
            (
                StatusCode::BAD_GATEWAY,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                axum::Json(err_json),
            )
                .into_response()
        }
    }
}
