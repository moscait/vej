//! HTTP server: router, listeners, and shutdown. Handlers live in [`handlers`].

mod handlers;
mod playground;
#[cfg(test)]
mod remote_tests;
#[cfg(test)]
mod tests;

use anyhow::Result;
use axum::{
    Extension, Router,
    extract::{ConnectInfo, Path},
    response::Redirect,
    routing::{get, post},
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use tower::ServiceExt;

use crate::access::{self, AccessPolicy};
use crate::activation::OwnedUnixSocket;
use crate::engine::DecisionModel;
use crate::idle::{self, ActivityTracker};
use crate::model_manager::{ModelLoader, ModelManager};
use crate::remote::{self, RemoteAuth};
use handlers::*;
use playground::serve_playground_asset;

/// Time in-flight requests get to finish after shutdown starts before the process exits.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

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

/// What a class of client may do beyond plain inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientPolicy {
    /// May name a catalog model in a request (which can trigger a model switch).
    pub allow_model_selection: bool,
    /// May set `keep_alive` (pin a model, or unload it with 0).
    pub allow_keep_alive: bool,
    /// Error responses may include internal details such as model file paths.
    pub expose_internal_errors: bool,
}

impl ClientPolicy {
    /// Loopback clients: the machine's own user, trusted with everything.
    pub const LOCAL: Self = Self {
        allow_model_selection: true,
        allow_keep_alive: true,
        expose_internal_errors: true,
    };
}

/// Remote clients (non-loopback peers or the remote socket): public API only, behind
/// bearer-token auth, with `policy` limiting what they can change.
pub fn build_remote_router<L>(
    state: Arc<AppState<L>>,
    auth: Arc<RemoteAuth>,
    policy: ClientPolicy,
) -> Router
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    public_routes::<L>()
        .layer(Extension(policy))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state.activity),
            idle::track_activity,
        ))
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            auth,
            remote::auth::authenticate,
        ))
}

/// Serves loopback peers with `local` and everyone else with `remote`. Needs
/// `ConnectInfo<SocketAddr>`, i.e. `into_make_service_with_connect_info`.
pub fn dispatch_by_peer(local: Router, remote: Router) -> Router {
    Router::new().fallback(
        move |ConnectInfo(peer): ConnectInfo<SocketAddr>, req: axum::extract::Request| {
            // IPv4-mapped addresses (::ffff:127.0.0.1) arrive on dual-stack sockets.
            let router = if peer.ip().to_canonical().is_loopback() {
                local.clone()
            } else {
                remote.clone()
            };
            async move {
                match router.oneshot(req).await {
                    Ok(response) => response,
                    Err(never) => match never {},
                }
            }
        },
    )
}

/// Inference and read-only status routes, offered to every client.
fn public_routes<L>() -> Router<Arc<AppState<L>>>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    Router::new()
        .route("/health", get(health_handler::<L>))
        .route("/v1/systemone", post(decision_handler::<L>))
        .route("/v1/systemone/batch", post(batch_decision_handler::<L>))
        .route("/v1/batch", post(batch_decision_handler::<L>))
        .route("/v1/generate", post(generate_handler::<L>))
        .route("/api/generate", post(generate_handler::<L>))
        .route("/api/tags", get(api_tags_handler::<L>))
        .route("/v1/models", get(list_models_handler::<L>))
        .route("/v1/ps", get(running_models_handler::<L>))
}

/// Routes that change server state, relay to third parties, or serve the playground:
/// loopback clients only.
fn admin_routes<L>() -> Router<Arc<AppState<L>>>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    Router::new()
        .route("/", get(root_handler::<L>))
        .route("/v1/models/switch", post(switch_model_handler::<L>))
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
}

/// Loopback clients: the full API plus browser protections (CORS, Host/Origin checks).
pub fn build_local_router<L>(state: Arc<AppState<L>>, policy: Arc<AccessPolicy>) -> Router
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    public_routes::<L>()
        .merge(admin_routes::<L>())
        .layer(Extension(ClientPolicy::LOCAL))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state.activity),
            idle::track_activity,
        ))
        .with_state(state)
        .layer(policy.cors_layer())
        .layer(axum::middleware::from_fn_with_state(policy, access::guard))
}

/// Everything [`run_server`] needs besides the model state.
pub struct ServeOptions {
    pub host: String,
    pub allowed_origins: Vec<String>,
    pub idle_exit: Option<Duration>,
    pub remote_auth: Arc<RemoteAuth>,
    pub remote_policy: ClientPolicy,
    /// Unix socket serving the remote API (OpenZiti sidecar / TLS proxy).
    pub remote_socket: Option<OwnedUnixSocket>,
}

/// Serves until Ctrl-C, SIGTERM, `POST /v1/shutdown`, or `idle_exit` without requests.
/// On `tcp` (bound by vej or handed over by launchd/systemd) loopback peers get the full
/// API and other peers the remote API; the remote socket always gets the remote API.
/// Without `tcp` (`--no-tcp`) only the remote socket is served.
pub async fn run_server<L>(
    state: Arc<AppState<L>>,
    tcp: Option<std::net::TcpListener>,
    options: ServeOptions,
) -> Result<()>
where
    L: ModelLoader,
    L::Model: DecisionModel,
{
    let remote = build_remote_router(
        Arc::clone(&state),
        options.remote_auth,
        options.remote_policy,
    );
    let tcp = match tcp {
        Some(listener) => {
            listener.set_nonblocking(true)?;
            let listener = tokio::net::TcpListener::from_std(listener)?;
            let port = listener.local_addr()?.port();
            let access = Arc::new(AccessPolicy::new(
                &options.host,
                port,
                &options.allowed_origins,
            ));
            let local = build_local_router(Arc::clone(&state), access);
            Some((listener, dispatch_by_peer(local, remote.clone())))
        }
        None => None,
    };
    if let Some(idle) = options.idle_exit {
        tracing::info!(idle = ?idle, "Will exit after this long without requests");
        spawn_idle_exit(Arc::clone(&state), idle);
    }
    let port = match &tcp {
        Some((listener, _)) => Some(listener.local_addr()?.port()),
        None => None,
    };
    print_banner(&options.host, port, options.remote_socket.as_ref());

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(trigger_shutdown(Arc::clone(&state), stop_tx));
    let tcp_server = serve_tcp(tcp, stop_rx.clone());
    let remote_server = serve_remote_socket(options.remote_socket, remote, stop_rx);
    tokio::try_join!(tcp_server, remote_server)?;
    Ok(())
}

async fn serve_tcp(
    tcp: Option<(tokio::net::TcpListener, Router)>,
    stop: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let Some((listener, app)) = tcp else {
        return Ok(());
    };
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(stopped(stop))
    .await?;
    Ok(())
}

fn print_banner(host: &str, port: Option<u16>, remote_socket: Option<&OwnedUnixSocket>) {
    match port {
        Some(port) => {
            let addr = format!("{host}:{port}");
            tracing::info!("vej Jev-compatible server listening on http://{addr}");
            println!("🚀 vej decision & generation daemon running at http://{addr}");
        }
        None => {
            tracing::info!("vej serving the remote socket only (no TCP listener)");
            println!(
                "🚀 vej decision & generation daemon running without TCP (remote socket only)"
            );
        }
    }
    println!("📌 POST /v1/systemone       - Evaluate typed decisions");
    println!("📌 POST /v1/systemone/batch - Evaluate batched decisions in a single call");
    println!("📌 POST /v1/generate        - Generate text / JSON from the active GGUF model");
    println!("📌 GET  /v1/models          - List configured models");
    println!("📌 GET  /v1/ps              - Show the loaded model and when it unloads");
    if let Some(port) = port {
        let addr = format!("{host}:{port}");
        println!("📌 POST /v1/models/switch   - Hot-switch active model (local only)");
        println!("📌 POST /v1/unload          - Release the loaded model now (local only)");
        println!("📌 POST /v1/shutdown        - Gracefully stop the vej daemon (local only)");
        println!("✨ Web Playground           - http://{addr}/playground (local only)");
    }
    #[cfg(unix)]
    if let Some(socket) = remote_socket {
        println!("🔌 Remote API socket        - {}", socket.path().display());
    }
    #[cfg(not(unix))]
    let _ = remote_socket;
}

/// Waits for a shutdown trigger, tells every listener to drain, and bounds the drain.
async fn trigger_shutdown<L: ModelLoader>(
    state: Arc<AppState<L>>,
    stop: tokio::sync::watch::Sender<bool>,
) {
    tokio::select! {
        () = shutdown_signal() => {}
        () = state.shutdown.notified() => {
            tracing::info!("Shutdown requested (POST /v1/shutdown or idle exit); stopping vej daemon");
        }
    }
    // Receivers only disappear once every server has stopped, so a send error is moot.
    let _ = stop.send(true);
    // Don't let a long generation hold the process open indefinitely.
    tokio::time::sleep(SHUTDOWN_GRACE).await;
    tracing::warn!("In-flight requests did not finish in time; exiting");
    std::process::exit(0);
}

async fn stopped(mut stop: tokio::sync::watch::Receiver<bool>) {
    // An error means the sender is gone, which also means: stop.
    let _ = stop.wait_for(|&stopping| stopping).await;
}

#[cfg(unix)]
async fn serve_remote_socket(
    socket: Option<OwnedUnixSocket>,
    app: Router,
    stop: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let Some(mut socket) = socket else {
        return Ok(());
    };
    let listener = socket
        .take_listener()
        .ok_or_else(|| anyhow::anyhow!("remote socket listener was already taken"))?;
    listener.set_nonblocking(true)?;
    let listener = tokio::net::UnixListener::from_std(listener)?;
    axum::serve(listener, app)
        .with_graceful_shutdown(stopped(stop))
        .await?;
    drop(socket); // removes the socket file
    Ok(())
}

#[cfg(not(unix))]
async fn serve_remote_socket(
    socket: Option<OwnedUnixSocket>,
    _app: Router,
    _stop: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    match socket {
        Some(socket) => socket.unreachable(),
        None => Ok(()),
    }
}

/// Requests shutdown once no request has arrived for `idle`.
fn spawn_idle_exit<L: ModelLoader>(state: Arc<AppState<L>>, idle: Duration) {
    tokio::spawn(async move {
        idle::wait_until_idle_for(&state.activity, idle).await;
        tracing::info!(idle = ?idle, "No requests for the idle-exit period; shutting down");
        state.shutdown.notify_one();
    });
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
