//! Router-level tests using a fake model loader.

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

fn app_and_state(loader: FakeLoader, max_concurrent: usize) -> (Router, Arc<AppState<FakeLoader>>) {
    let manager = ModelManager::new(loader, "model-a", KeepAlive::default());
    let state = AppState::new(manager, max_concurrent);
    (build_local_router(Arc::clone(&state), test_policy()), state)
}

fn app_with(loader: FakeLoader, max_concurrent: usize) -> Router {
    app_and_state(loader, max_concurrent).0
}

pub(super) fn app() -> (Router, Probe) {
    let (loader, probe) = FakeLoader::new();
    (app_with(loader, 2), probe)
}

pub(super) async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
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

pub(super) async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::get(uri).body(Body::empty()).unwrap();
    let (status, _, body) = send(app, req).await;
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

pub(super) async fn post_json(app: &Router, uri: &str, payload: Value) -> (StatusCode, Value) {
    let req = Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let (status, _, body) = send(app, req).await;
    let value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
    (status, value)
}

pub(super) fn decision(extra: Value) -> Value {
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

    let (status, _) = post_json(&app, "/v1/systemone", decision(json!({ "keep_alive": 0 }))).await;

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

    let (status, body) = post_json(&app, "/v1/models/switch", json!({ "model": "model-b" })).await;

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
    let cases = [
        (
            "empty choices",
            "/v1/systemone",
            json!({ "state": "x", "choices": [] }),
        ),
        (
            "empty questions",
            "/v1/systemone",
            json!({ "state": "x", "questions": {} }),
        ),
        (
            "too many levels",
            "/v1/systemone",
            json!({ "state": "x", "questions": { "rating": { "type": "score", "levels": 102 } } }),
        ),
        (
            "oversized batch",
            "/v1/batch",
            json!({
                "choices": ["a", "b"],
                "items": (0..257).map(|i| json!({ "id": format!("i{i}"), "state": "s" })).collect::<Vec<_>>()
            }),
        ),
    ];
    for (label, path, body) in cases {
        let (status, _) = post_json(&app, path, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}");
    }
}

#[tokio::test]
async fn in_engine_invalid_request_returns_400_and_internal_error_returns_500() {
    let invalid_loader = FakeLoader::new()
        .0
        .with_eval_invalid_request("input is 9000 tokens; the limit is 8192");
    let invalid_app = app_with(invalid_loader, 4);

    let (status, body) = post_json(
        &invalid_app,
        "/v1/systemone",
        json!({ "state": "x", "choices": ["yes", "no"] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.as_str()
            .is_some_and(|s| s.contains("input is 9000 tokens; the limit is 8192")),
        "{body}"
    );

    let (batch_status, batch_body) = post_json(
        &invalid_app,
        "/v1/batch",
        json!({ "choices": ["yes", "no"], "items": [{ "id": "i0", "state": "x" }] }),
    )
    .await;
    assert_eq!(batch_status, StatusCode::BAD_REQUEST);
    assert!(
        batch_body
            .as_str()
            .is_some_and(|s| s.contains("input is 9000 tokens; the limit is 8192")),
        "{batch_body}"
    );

    let (gen_status, gen_body) =
        post_json(&invalid_app, "/v1/generate", json!({ "prompt": "hi" })).await;
    assert_eq!(gen_status, StatusCode::BAD_REQUEST);
    assert!(
        gen_body
            .as_str()
            .is_some_and(|s| s.contains("input is 9000 tokens; the limit is 8192")),
        "{gen_body}"
    );

    let internal_loader = FakeLoader::new()
        .0
        .with_eval_internal_error("gpu decode blew up");
    let internal_app = app_with(internal_loader, 4);
    let (err_status, err_body) = post_json(
        &internal_app,
        "/v1/systemone",
        json!({ "state": "x", "choices": ["yes", "no"] }),
    )
    .await;
    assert_eq!(err_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        err_body
            .as_str()
            .is_some_and(|s| s.contains("gpu decode blew up")),
        "{err_body}"
    );
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_request_keeps_inference_slot_until_completion() {
    let (loader, probe) = FakeLoader::new();
    let app = app_with(loader.with_eval_delay(Duration::from_millis(300)), 1);

    let timed_out = tokio::time::timeout(
        Duration::from_millis(50),
        post_json(&app, "/v1/systemone", decision(json!({}))),
    )
    .await;
    assert!(timed_out.is_err(), "request A should time out");

    let (status, _) = post_json(&app, "/v1/systemone", decision(json!({}))).await;
    assert_eq!(status, StatusCode::OK);
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

    let (status, _, _) = send(&app, request("127.0.0.1:8080", Some("http://evil.example"))).await;
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

#[tokio::test]
async fn validation_rejects_oversized_or_malformed_requests_without_loading_model() {
    let (app, probe) = app();

    let too_many_choices: Vec<String> = (0..27).map(|i| format!("c{i}")).collect();
    let (status, _) = post_json(
        &app,
        "/v1/systemone",
        json!({ "state": "x", "choices": too_many_choices }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "27 choices");

    let (status, _) = post_json(
        &app,
        "/v1/systemone",
        json!({
            "state": "x",
            "questions": { "rating": { "type": "score", "levels": 102 } }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "102 levels");

    let mut too_many_questions = serde_json::Map::new();
    for i in 0..33 {
        too_many_questions.insert(format!("q{i}"), json!({ "type": "noul" }));
    }
    let (status, _) = post_json(
        &app,
        "/v1/systemone",
        json!({ "state": "x", "questions": too_many_questions }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "33 questions");

    let too_many_items: Vec<Value> = (0..257)
        .map(|i| json!({ "id": format!("item-{i}"), "state": "s" }))
        .collect();
    let (status, _) = post_json(
        &app,
        "/v1/batch",
        json!({ "choices": ["a", "b"], "items": too_many_items }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "257 batch items");

    let mut q32 = serde_json::Map::new();
    for i in 0..32 {
        q32.insert(format!("q{i}"), json!({ "type": "noul" }));
    }
    let items_33: Vec<Value> = (0..33)
        .map(|i| json!({ "id": format!("item-{i}"), "state": "s" }))
        .collect();
    let (status, _) = post_json(
        &app,
        "/v1/batch",
        json!({ "questions": q32, "items": items_33 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "33 * 32 = 1056 > 1024");

    let huge_state = "x".repeat(crate::engine::MAX_REQUEST_BODY_BYTES + 128);
    let oversized_body = serde_json::to_vec(&json!({
        "state": huge_state,
        "choices": ["a", "b"],
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/v1/systemone")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(oversized_body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        ">2 MiB request body must be rejected before loading"
    );

    assert!(
        probe.loads().is_empty(),
        "invalid shapes must be rejected before loading a model, got: {:?}",
        probe.loads()
    );

    let max_choices: Vec<String> = (0..26)
        .map(|i| char::from(b'A' + i as u8).to_string())
        .collect();
    let (status, _) = post_json(
        &app,
        "/v1/systemone",
        json!({ "state": "x", "choices": max_choices }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "26 choices A..Z must succeed");
}

#[tokio::test]
async fn jev_proxy_maps_timeout_to_504_with_json_error() {
    let client = handlers::build_jev_client(Duration::from_millis(50), Duration::from_millis(80))
        .expect("build jev client");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        if let Ok((_stream, _)) = listener.accept().await {
            std::future::pending::<()>().await;
        }
    });

    let res = handlers::forward_jev_request(
        &client,
        &format!("http://{addr}/v1/systemone"),
        &HeaderMap::new(),
        Bytes::from_static(b"{}"),
    )
    .await;

    assert_eq!(res.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let err_json: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        err_json["error"]
            .as_str()
            .is_some_and(|s| s.contains("TypeSafe AI")),
        "unexpected error body: {err_json}"
    );
    task.abort();
}

#[tokio::test]
async fn jev_proxy_maps_body_timeout_to_504_with_json_error() {
    use tokio::io::AsyncWriteExt;

    let client = handlers::build_jev_client(Duration::from_millis(50), Duration::from_millis(80))
        .expect("build jev client");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"pa")
                .await;
            std::future::pending::<()>().await;
        }
    });

    let res = handlers::forward_jev_request(
        &client,
        &format!("http://{addr}/v1/systemone"),
        &HeaderMap::new(),
        Bytes::from_static(b"{}"),
    )
    .await;

    assert_eq!(res.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let err_json: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        err_json["error"]
            .as_str()
            .is_some_and(|s| s.starts_with("Failed to reach TypeSafe AI:")),
        "unexpected error body: {err_json}"
    );
    task.abort();
}

#[tokio::test]
async fn jev_proxy_maps_connection_failure_to_502_with_json_error() {
    let client = handlers::build_jev_client(Duration::from_millis(50), Duration::from_millis(80))
        .expect("build jev client");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let res = handlers::forward_jev_request(
        &client,
        &format!("http://{addr}/v1/systemone"),
        &HeaderMap::new(),
        Bytes::from_static(b"{}"),
    )
    .await;

    assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let err_json: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        err_json["error"]
            .as_str()
            .is_some_and(|s| s.starts_with("Failed to reach TypeSafe AI:")),
        "unexpected error body: {err_json}"
    );
}

#[tokio::test]
async fn jev_proxy_forwards_ok_response_and_reuses_shared_client() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let client = handlers::build_jev_client(Duration::from_millis(200), Duration::from_secs(2))
        .expect("build jev client");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).await.unwrap();
        let raw_req = String::from_utf8_lossy(&buf[..n]).to_string();
        let payload = r#"{"decision":"ok"}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
        stream.write_all(reply.as_bytes()).await.unwrap();
        raw_req
    });

    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, "Bearer secret-key".parse().unwrap());
    let res = handlers::forward_jev_request(
        &client,
        &format!("http://{addr}/v1/systemone"),
        &headers,
        Bytes::from_static(br#"{"state":"hi"}"#),
    )
    .await;

    assert_eq!(res.status(), StatusCode::OK);
    let ok_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let ok_json: Value = serde_json::from_slice(&ok_bytes).unwrap();
    assert_eq!(ok_json, json!({ "decision": "ok" }));
    let raw_req = server_task.await.unwrap();
    assert!(
        raw_req
            .to_lowercase()
            .contains("authorization: bearer secret-key")
    );

    let a = handlers::shared_jev_client().unwrap();
    let b = handlers::shared_jev_client().unwrap();
    assert!(std::ptr::eq(a, b));
}
