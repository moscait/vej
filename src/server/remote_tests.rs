//! Remote surface: bearer-token auth, public-only routes, client policy, peer dispatch.

use super::tests::{decision, post_json, send};
use super::*;
use crate::keep_alive::KeepAlive;
use crate::remote::tokens::{self, TokenStore};
use crate::test_support::{FakeLoader, Probe};
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const REMOTE: ClientPolicy = ClientPolicy {
    allow_model_selection: false,
    allow_keep_alive: false,
    expose_internal_errors: false,
};

struct RemoteApp {
    app: Router,
    probe: Probe,
    token: String,
    state: Arc<AppState<FakeLoader>>,
}

fn temp_tokens_file() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("vej-remote-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("remote_tokens")
}

fn remote_app_with(
    loader: FakeLoader,
    probe: Probe,
    require_token: bool,
    policy: ClientPolicy,
) -> RemoteApp {
    let path = temp_tokens_file();
    let token = tokens::create(&path, "laptop").unwrap();
    let manager = ModelManager::new(loader, "model-a", KeepAlive::default());
    let state = AppState::new(manager, 2);
    let auth = RemoteAuth::new(TokenStore::open(&path), require_token);
    let app = build_remote_router(Arc::clone(&state), auth, policy);
    RemoteApp {
        app,
        probe,
        token,
        state,
    }
}

fn remote_app(policy: ClientPolicy) -> RemoteApp {
    let (loader, probe) = FakeLoader::new();
    remote_app_with(loader, probe, true, policy)
}

fn request(method: Method, uri: &str, bearer: Option<&str>, body: Option<&Value>) -> Request<Body> {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    match body {
        Some(json) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    }
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, _, bytes) = send(app, request(method, uri, bearer, body.as_ref())).await;
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, value)
}

#[tokio::test]
async fn remote_api_requires_a_valid_bearer_token() {
    let r = remote_app(REMOTE);
    let body = decision(json!({}));

    let missing = send(
        &r.app,
        request(Method::POST, "/v1/systemone", None, Some(&body)),
    )
    .await;
    assert_eq!(missing.0, StatusCode::UNAUTHORIZED);
    assert!(
        missing.1[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap()
            .starts_with("Bearer")
    );

    let wrong = call(
        &r.app,
        Method::POST,
        "/v1/systemone",
        Some("vej_nope"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(wrong.0, StatusCode::UNAUTHORIZED);

    let basic = Request::post("/v1/systemone")
        .header(header::AUTHORIZATION, "Basic bGFwdG9wOnB3")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    assert_eq!(send(&r.app, basic).await.0, StatusCode::UNAUTHORIZED);
    assert!(
        r.probe.events().is_empty(),
        "rejected requests must not load models"
    );

    let ok = call(
        &r.app,
        Method::POST,
        "/v1/systemone",
        Some(&r.token),
        Some(body),
    )
    .await;
    assert_eq!(ok.0, StatusCode::OK, "{}", ok.1);
    assert_eq!(ok.1["decision"], "refund");
}

#[tokio::test]
async fn remote_exposes_only_the_public_api() {
    let r = remote_app(REMOTE);
    let t = Some(r.token.as_str());

    for uri in ["/health", "/v1/models", "/v1/ps", "/api/tags"] {
        assert_eq!(
            call(&r.app, Method::GET, uri, t, None).await.0,
            StatusCode::OK,
            "GET {uri}"
        );
    }
    let batch = json!({ "choices": ["a", "b"], "items": [{ "id": "1", "state": "x" }] });
    for (uri, body) in [
        ("/v1/systemone", decision(json!({}))),
        ("/v1/systemone/batch", batch.clone()),
        ("/v1/batch", batch),
        ("/v1/generate", json!({ "prompt": "hi" })),
        ("/api/generate", json!({ "prompt": "hi" })),
    ] {
        assert_eq!(
            call(&r.app, Method::POST, uri, t, Some(body)).await.0,
            StatusCode::OK,
            "POST {uri}"
        );
    }

    for uri in [
        "/v1/models/switch",
        "/v1/unload",
        "/v1/shutdown",
        "/shutdown",
        "/v1/proxy/jev",
    ] {
        let status = call(
            &r.app,
            Method::POST,
            uri,
            t,
            Some(json!({ "model": "model-b" })),
        )
        .await
        .0;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "admin route POST {uri} must not exist remotely"
        );
    }
    for uri in ["/", "/playground/", "/playground/src/app.js"] {
        assert_eq!(
            call(&r.app, Method::GET, uri, t, None).await.0,
            StatusCode::NOT_FOUND,
            "GET {uri}"
        );
    }
    tokio::time::timeout(
        std::time::Duration::from_millis(50),
        r.state.shutdown.notified(),
    )
    .await
    .expect_err("remote shutdown attempt must not signal the server");
}

#[tokio::test]
async fn optional_tokens_admit_anonymous_callers_but_reject_bad_tokens() {
    let (loader, probe) = FakeLoader::new();
    let r = remote_app_with(loader, probe, false, REMOTE);

    assert_eq!(
        call(&r.app, Method::GET, "/v1/ps", None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&r.app, Method::GET, "/v1/ps", Some("vej_nope"), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "a presented token must be valid even when tokens are optional"
    );
    assert_eq!(
        call(&r.app, Method::GET, "/v1/ps", Some(&r.token), None)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn remote_clients_cannot_switch_models_unless_allowed() {
    let r = remote_app(REMOTE);
    let (_, body) = call(
        &r.app,
        Method::POST,
        "/v1/systemone",
        Some(&r.token),
        Some(decision(json!({ "model": "llama-3.2-1b" }))),
    )
    .await;
    assert_eq!(body["model"], "model-a");
    assert_eq!(r.probe.loads(), ["model-a"]);

    let allowed = remote_app(ClientPolicy {
        allow_model_selection: true,
        ..REMOTE
    });
    let (_, body) = call(
        &allowed.app,
        Method::POST,
        "/v1/systemone",
        Some(&allowed.token),
        Some(decision(json!({ "model": "llama-3.2-1b" }))),
    )
    .await;
    assert_eq!(body["model"], "llama-3.2-1b");
}

#[tokio::test]
async fn remote_clients_can_only_select_catalog_models() {
    // A `[models.<name>]` override defines a local-only name: even with model selection
    // allowed, remote requests naming it keep the serving model.
    let allowed = remote_app(ClientPolicy {
        allow_model_selection: true,
        ..REMOTE
    });
    let (status, body) = call(
        &allowed.app,
        Method::POST,
        "/v1/systemone",
        Some(&allowed.token),
        Some(decision(json!({ "model": "my-custom-gguf" }))),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["model"], "model-a");
    assert_eq!(allowed.probe.loads(), ["model-a"]);
}

#[tokio::test]
async fn remote_keep_alive_is_ignored_unless_allowed() {
    let r = remote_app(REMOTE);
    let t = Some(r.token.as_str());
    call(
        &r.app,
        Method::POST,
        "/v1/systemone",
        t,
        Some(decision(json!({ "keep_alive": 0 }))),
    )
    .await;
    assert!(
        r.state.manager.status().is_some(),
        "remote keep_alive 0 must not unload"
    );

    let (_, body) = call(
        &r.app,
        Method::POST,
        "/api/generate",
        t,
        Some(json!({ "keep_alive": 0 })),
    )
    .await;
    assert_ne!(body["done_reason"], "unload");
    assert!(
        r.state.manager.status().is_some(),
        "Ollama unload idiom must not work remotely"
    );

    let allowed = remote_app(ClientPolicy {
        allow_keep_alive: true,
        ..REMOTE
    });
    call(
        &allowed.app,
        Method::POST,
        "/v1/systemone",
        Some(&allowed.token),
        Some(decision(json!({ "keep_alive": 0 }))),
    )
    .await;
    assert!(allowed.state.manager.status().is_none());
}

#[tokio::test]
async fn remote_errors_hide_internal_details() {
    let (loader, probe) = FakeLoader::new();
    let r = remote_app_with(loader.failing_first(1), probe, true, REMOTE);

    let (status, body) = call(
        &r.app,
        Method::POST,
        "/v1/systemone",
        Some(&r.token),
        Some(decision(json!({}))),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let text = body.to_string();
    assert!(!text.contains("simulated load failure"), "{text}");
    assert!(text.contains("unavailable"), "{text}");
}

#[test]
fn startup_check_requires_tokens_only_when_tokens_are_required() {
    let empty = temp_tokens_file();
    let err = RemoteAuth::new(TokenStore::open(&empty), true)
        .ensure_usable()
        .unwrap_err();
    assert!(err.to_string().contains("vej remote token create"), "{err}");
    assert!(
        RemoteAuth::new(TokenStore::open(&empty), false)
            .ensure_usable()
            .is_ok()
    );

    let with_token = temp_tokens_file();
    tokens::create(&with_token, "laptop").unwrap();
    assert!(
        RemoteAuth::new(TokenStore::open(&with_token), true)
            .ensure_usable()
            .is_ok()
    );
}

#[tokio::test]
async fn loopback_peers_get_the_local_api_and_others_the_remote_api() {
    let r = remote_app(REMOTE);
    let local = build_local_router(
        Arc::clone(&r.state),
        Arc::new(crate::access::AccessPolicy::new("0.0.0.0", 8080, &[])),
    );
    let app = dispatch_by_peer(local, r.app.clone());
    let switch = |peer: &str, bearer: Option<&str>| {
        let mut req = request(
            Method::POST,
            "/v1/models/switch",
            bearer,
            Some(&json!({ "model": "model-b" })),
        );
        req.extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        req
    };

    assert_eq!(
        send(&app, switch("127.0.0.1:50000", None)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, switch("[::ffff:127.0.0.1]:50000", None)).await.0,
        StatusCode::OK,
        "IPv4-mapped loopback"
    );
    assert_eq!(
        send(&app, switch("192.168.1.20:50000", None)).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, switch("192.168.1.20:50000", Some(&r.token)))
            .await
            .0,
        StatusCode::NOT_FOUND,
        "admin routes don't exist for remote peers"
    );
}

#[tokio::test]
async fn local_router_still_honours_keep_alive_and_model_selection() {
    let (app, probe) = super::tests::app();
    post_json(
        &app,
        "/v1/systemone",
        decision(json!({ "model": "llama-3.2-1b", "keep_alive": 0 })),
    )
    .await;
    assert_eq!(probe.loads(), ["llama-3.2-1b"]);
    assert_eq!(probe.drops(), ["llama-3.2-1b"]);
}

/// Minimal HTTP/1.1 exchange over a Unix socket; returns the raw response.
#[cfg(unix)]
async fn unix_request(socket: &std::path::Path, method: &str, uri: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    let request = format!(
        "{method} {uri} HTTP/1.1\r\nHost: vej\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

#[cfg(unix)]
#[tokio::test]
async fn socket_only_server_serves_the_remote_api_and_cleans_up() {
    let tokens_file = temp_tokens_file();
    let socket_path = tokens_file.with_file_name("r.sock");
    let socket = crate::activation::bind_unix_socket(&socket_path).unwrap();
    let (loader, _probe) = FakeLoader::new();
    let state = AppState::new(
        ModelManager::new(loader, "model-a", KeepAlive::default()),
        2,
    );
    let options = ServeOptions {
        host: "127.0.0.1".into(),
        allowed_origins: Vec::new(),
        idle_exit: None,
        remote_auth: RemoteAuth::new(TokenStore::open(&tokens_file), false),
        remote_policy: REMOTE,
        remote_socket: Some(socket),
    };
    let server = tokio::spawn(run_server(Arc::clone(&state), None, options));

    let health = unix_request(&socket_path, "GET", "/health").await;
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    let unload = unix_request(&socket_path, "POST", "/v1/unload").await;
    assert!(
        unload.starts_with("HTTP/1.1 404"),
        "admin routes stay hidden without TCP: {unload}"
    );

    state.shutdown.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), server)
        .await
        .expect("server stops promptly")
        .unwrap()
        .unwrap();
    assert!(!socket_path.exists(), "socket file is removed on shutdown");
}
