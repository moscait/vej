//! Bearer-token authentication for remote listeners.

use anyhow::{Result, bail};
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use std::time::Instant;

use super::tokens::TokenStore;

/// The authenticated client (token name), or "anonymous" when tokens are not required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller(pub String);

pub struct RemoteAuth {
    tokens: TokenStore,
    require_token: bool,
}

impl RemoteAuth {
    pub fn new(tokens: TokenStore, require_token: bool) -> Arc<Self> {
        Arc::new(Self {
            tokens,
            require_token,
        })
    }

    /// Fails at startup when tokens are required but none exist, instead of rejecting
    /// every remote request later.
    pub fn ensure_usable(&self) -> Result<()> {
        if self.require_token && self.tokens.is_empty()? {
            bail!(
                "remote access requires a bearer token, but {} has none. Create one with \
                 `vej remote token create <name>`, or set `require_token = false` under [remote] \
                 when another layer (e.g. OpenZiti) authenticates callers.",
                self.tokens.path().display()
            );
        }
        Ok(())
    }

    /// Resolves the caller for a request, or the response rejecting it.
    fn identify(&self, headers: &HeaderMap) -> Result<String, Rejection> {
        let token = match bearer_token(headers) {
            Credential::Missing if self.require_token => {
                return Err(Rejection::Unauthorized("missing bearer token"));
            }
            Credential::Missing => return Ok(ANONYMOUS.to_string()),
            Credential::Malformed => {
                return Err(Rejection::Unauthorized(
                    "expected 'Authorization: Bearer <token>'",
                ));
            }
            Credential::Bearer(token) => token,
        };
        match self.tokens.verify(token) {
            Ok(Some(name)) => Ok(name),
            Ok(None) => Err(Rejection::Unauthorized("invalid token")),
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "Cannot read remote tokens; denying request");
                Err(Rejection::Unavailable)
            }
        }
    }
}

const ANONYMOUS: &str = "anonymous";

enum Rejection {
    Unauthorized(&'static str),
    /// The token store could not be read: deny rather than guess.
    Unavailable,
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized(reason) => unauthorized(reason),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "authentication unavailable",
            )
                .into_response(),
        }
    }
}

enum Credential<'a> {
    Missing,
    Malformed,
    Bearer(&'a str),
}

fn bearer_token(headers: &HeaderMap) -> Credential<'_> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Credential::Missing;
    };
    value
        .to_str()
        .ok()
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map_or(Credential::Malformed, Credential::Bearer)
}

fn unauthorized(reason: &'static str) -> Response {
    tracing::warn!(reason, "Rejected remote request");
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"vej\"")],
        Json(serde_json::json!({ "error": reason })),
    )
        .into_response()
}

/// Axum middleware: admits requests with a valid bearer token (or none, when tokens are
/// optional), records the [`Caller`], and logs each remote request.
pub async fn authenticate(
    State(auth): State<Arc<RemoteAuth>>,
    mut req: Request,
    next: Next,
) -> Response {
    let caller = match auth.identify(req.headers()) {
        Ok(caller) => caller,
        Err(rejection) => return rejection.into_response(),
    };
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    req.extensions_mut().insert(Caller(caller.clone()));

    let started = Instant::now();
    let response = next.run(req).await;
    tracing::info!(
        caller = %caller,
        %method,
        path,
        status = response.status().as_u16(),
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "Remote request"
    );
    response
}
