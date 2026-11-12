//! Browser access control for the vej daemon.
//!
//! Two checks run on every request:
//!
//! * **Host check** (DNS-rebinding protection): the `Host` header must name the
//!   daemon by an IP literal, `localhost`, or a host from an explicitly allowed
//!   origin. A rebinding attack needs a page on an attacker-controlled DNS name
//!   whose record is later pointed at 127.0.0.1, so its requests carry that DNS
//!   name in `Host` and are rejected here.
//! * **Origin check**: when a browser sends an `Origin` header, it must be the
//!   daemon's own origin (same as `Host`), a loopback alias of it on the same
//!   port, or an origin listed in `allowed_origins`. Requests from any other
//!   site are rejected with 403 *before* reaching a handler, which matters for
//!   "simple" cross-site POSTs (e.g. a form posting to `/v1/shutdown`) that CORS
//!   alone would not stop.
//!
//! Non-browser clients (curl, the Dart/Python SDKs) send no `Origin` header and
//! are unaffected.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::IpAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

const LOOPBACK_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

#[derive(Debug, Clone)]
pub struct AccessPolicy {
    port: u16,
    /// Exact origins (scheme://host[:port], lowercase, no trailing slash).
    allowed_origins: Vec<String>,
    /// Hostnames (no port) taken from `allowed_origins`, accepted in `Host`.
    extra_hosts: Vec<String>,
}

impl AccessPolicy {
    pub fn new(bind_host: &str, port: u16, extra_origins: &[String]) -> Self {
        let mut allowed_origins: Vec<String> = LOOPBACK_HOSTS
            .iter()
            .map(|h| format!("http://{h}:{port}"))
            .collect();

        let bind_is_unspecified = bind_host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .map(|ip| ip.is_unspecified())
            .unwrap_or(false);
        if !bind_is_unspecified {
            allowed_origins.push(format!("http://{}:{port}", bracket_ipv6(bind_host)));
        }

        let mut extra_hosts = Vec::new();
        for o in extra_origins {
            let o = normalize_origin(o);
            if let Some((host, _)) = split_origin(&o) {
                extra_hosts.push(host.to_string());
            }
            allowed_origins.push(o);
        }
        allowed_origins.dedup();

        Self {
            port,
            allowed_origins,
            extra_hosts,
        }
    }

    /// Returns true if the `Host` header value is acceptable.
    pub fn host_allowed(&self, host_header: &str) -> bool {
        let hostname = strip_port(&host_header.to_ascii_lowercase());
        if hostname == "localhost" || is_ip_literal(&hostname) {
            return true;
        }
        self.extra_hosts.contains(&hostname)
    }

    /// Returns true if a browser request with this `Origin` may proceed.
    /// `host_header` is the request's (already validated) `Host`.
    pub fn origin_allowed(&self, origin: &str, host_header: Option<&str>) -> bool {
        let origin = normalize_origin(origin);
        if self.allowed_origins.contains(&origin) {
            return true;
        }
        // Same-origin: the page was served by this daemon under the Host the
        // browser is using (e.g. a LAN IP when bound to 0.0.0.0).
        if let (Some(host), Some((o_host, o_port))) = (host_header, split_origin(&origin)) {
            if !origin.starts_with("http://") {
                return false;
            }
            let host = host.to_ascii_lowercase();
            let h_name = strip_port(&host);
            let h_port = port_of(&host).unwrap_or(80);
            return o_host == h_name && o_port == h_port && h_port == self.port;
        }
        false
    }

    pub fn cors_layer(self: &Arc<Self>) -> CorsLayer {
        let policy = self.clone();
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(
                move |origin: &HeaderValue, parts| {
                    let host = parts
                        .headers
                        .get(header::HOST)
                        .and_then(|v| v.to_str().ok());
                    origin
                        .to_str()
                        .map(|o| policy.origin_allowed(o, host))
                        .unwrap_or(false)
                },
            ))
            .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
    }

    fn check(&self, headers: &HeaderMap) -> Result<(), &'static str> {
        let host = match headers.get(header::HOST) {
            Some(v) => Some(v.to_str().map_err(|_| "invalid Host header")?),
            None => None,
        };
        if let Some(h) = host
            && !self.host_allowed(h)
        {
            return Err("Host not allowed");
        }
        if let Some(origin) = headers.get(header::ORIGIN) {
            let origin = origin.to_str().map_err(|_| "invalid Origin header")?;
            if !self.origin_allowed(origin, host) {
                return Err("Origin not allowed");
            }
        }
        Ok(())
    }
}

/// Axum middleware enforcing [`AccessPolicy`].
pub async fn guard(State(policy): State<Arc<AccessPolicy>>, req: Request, next: Next) -> Response {
    match policy.check(req.headers()) {
        Ok(()) => next.run(req).await,
        Err(reason) => {
            tracing::warn!(
                host = ?req.headers().get(header::HOST),
                origin = ?req.headers().get(header::ORIGIN),
                "rejected request: {reason}"
            );
            (StatusCode::FORBIDDEN, reason).into_response()
        }
    }
}

fn normalize_origin(o: &str) -> String {
    let o = o.trim().trim_end_matches('/').to_ascii_lowercase();
    // Browsers omit default ports in Origin; do the same for configured ones.
    o.strip_suffix(":80")
        .filter(|s| s.starts_with("http://"))
        .or_else(|| o.strip_suffix(":443").filter(|s| s.starts_with("https://")))
        .map(str::to_string)
        .unwrap_or(o)
}

/// Splits "scheme://host[:port]" into (host, port), defaulting the port by scheme.
fn split_origin(origin: &str) -> Option<(String, u16)> {
    let (scheme, rest) = origin.split_once("://")?;
    let default = match scheme {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    Some((strip_port(rest), port_of(rest).unwrap_or(default)))
}

/// "host:port" / "[v6]:port" / "host" -> hostname (IPv6 keeps brackets).
fn strip_port(authority: &str) -> String {
    if authority.starts_with('[') {
        match authority.find(']') {
            Some(i) => authority[..=i].to_string(),
            None => authority.to_string(),
        }
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    }
}

fn port_of(authority: &str) -> Option<u16> {
    let after = if authority.starts_with('[') {
        authority.split_once(']')?.1
    } else {
        authority
    };
    after.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
}

fn is_ip_literal(hostname: &str) -> bool {
    hostname.trim_matches(['[', ']']).parse::<IpAddr>().is_ok()
}

fn bracket_ipv6(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> AccessPolicy {
        AccessPolicy::new("127.0.0.1", 8080, &["http://localhost:3000/".to_string()])
    }

    #[test]
    fn host_check_blocks_rebinding_names() {
        let p = policy();
        assert!(p.host_allowed("127.0.0.1:8080"));
        assert!(p.host_allowed("localhost:8080"));
        assert!(p.host_allowed("[::1]:8080"));
        assert!(p.host_allowed("192.168.1.5:8080"));
        assert!(p.host_allowed("localhost")); // from allowed origin
        assert!(!p.host_allowed("evil.example.com:8080"));
        assert!(!p.host_allowed("127.0.0.1.nip.io:8080"));
    }

    #[test]
    fn origin_check() {
        let p = policy();
        let h = Some("127.0.0.1:8080");
        assert!(p.origin_allowed("http://127.0.0.1:8080", h));
        assert!(p.origin_allowed("http://localhost:8080", h));
        assert!(p.origin_allowed("http://localhost:3000", h));
        assert!(!p.origin_allowed("https://evil.example.com", h));
        assert!(!p.origin_allowed("null", h));
        assert!(!p.origin_allowed("http://localhost:9999", h));
        assert!(!p.origin_allowed("http://127.0.0.1:8081", h));
    }

    #[test]
    fn same_origin_on_lan_bind() {
        let p = AccessPolicy::new("0.0.0.0", 8080, &[]);
        assert!(p.origin_allowed("http://192.168.1.5:8080", Some("192.168.1.5:8080")));
        assert!(!p.origin_allowed("http://192.168.1.5:8080", Some("10.0.0.1:8080")));
        assert!(!p.origin_allowed("http://0.0.0.0:8080", None));
    }
}
