//! Who a request comes from, on each of the engine's two listeners.
//!
//! * The **local** listener (`ZEEK_BIND`, localhost) counts each request
//!   against its TCP peer address.
//! * The **public** listener (`ZEEK_PUBLIC_BIND`) is the one the Cloudflare
//!   tunnel forwards to. Every request on it must carry the proxy secret in
//!   [`PROXY_HEADER`], which only the Pages Function knows; anything else
//!   (for example a request sent straight to the tunnel's hostname) is
//!   refused. Behind the tunnel every TCP peer is the local `cloudflared`, so
//!   the client is the address the Function reports in [`CLIENT_HEADER`],
//!   trusted only because the secret matched.
//!
//! Rate limits ([`crate::relay::RateLimiter`]) key on the resulting
//! [`ClientIp`].

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};

use crate::api::ErrorBody;
use crate::error::EngineError;

/// Header carrying the shared secret from the Pages Function.
pub const PROXY_HEADER: &str = "x-zeek-proxy";
/// Header carrying the client address, as Cloudflare reported it to the
/// Function (`CF-Connecting-IP`).
pub const CLIENT_HEADER: &str = "x-zeek-client";
/// Shortest accepted proxy secret.
pub const MIN_SECRET_LEN: usize = 32;

/// The client a request is counted against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

impl ClientIp {
    /// The client of `request`, or the unspecified address when no listener
    /// layer recorded one (tests).
    pub fn of(request: &Request) -> IpAddr {
        request
            .extensions()
            .get::<ClientIp>()
            .map_or(IpAddr::from([0, 0, 0, 0]), |c| c.0)
    }
}

/// The proxy secret (never logged).
#[derive(Clone)]
pub struct ProxySecret(Arc<str>);

impl ProxySecret {
    /// Wraps `secret`; refuses one shorter than [`MIN_SECRET_LEN`].
    pub fn new(secret: &str) -> Result<Self, EngineError> {
        let secret = secret.trim();
        if secret.len() < MIN_SECRET_LEN {
            return Err(EngineError::Config(format!(
                "the proxy secret must be at least {MIN_SECRET_LEN} characters"
            )));
        }
        Ok(ProxySecret(secret.into()))
    }

    /// Constant-time comparison.
    fn matches(&self, given: &[u8]) -> bool {
        let expected = self.0.as_bytes();
        if given.len() != expected.len() {
            return false;
        }
        given
            .iter()
            .zip(expected)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

fn refused(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.to_owned(),
        }),
    )
        .into_response()
}

async fn peer(mut request: Request, next: Next) -> Response {
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::from([0, 0, 0, 0]), |c| c.0.ip());
    request.extensions_mut().insert(ClientIp(ip));
    next.run(request).await
}

async fn proxied(State(secret): State<ProxySecret>, mut request: Request, next: Next) -> Response {
    let authentic = request
        .headers()
        .get(PROXY_HEADER)
        .is_some_and(|v| secret.matches(v.as_bytes()));
    if !authentic {
        return refused(
            StatusCode::FORBIDDEN,
            "use the public site; this address only serves its proxy",
        );
    }
    let client = request
        .headers()
        .get(CLIENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<IpAddr>().ok());
    let Some(ip) = client else {
        return refused(StatusCode::BAD_REQUEST, "the proxy did not name the client");
    };
    // Nothing downstream needs the secret or the forwarded address header.
    request.headers_mut().remove(PROXY_HEADER);
    request.extensions_mut().insert(ClientIp(ip));
    next.run(request).await
}

/// The router for the local listener.
pub fn local(router: Router) -> Router {
    router.layer(middleware::from_fn(peer))
}

/// The router for the public listener: only the proxy may call it.
pub fn public(router: Router, secret: ProxySecret) -> Router {
    router.layer(middleware::from_fn_with_state(secret, proxied))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use tower::ServiceExt;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn app() -> Router {
        let echo = Router::new().route(
            "/who",
            get(|request: Request| async move { ClientIp::of(&request).to_string() }),
        );
        public(echo, ProxySecret::new(SECRET).expect("secret"))
    }

    async fn call(headers: &[(&str, &str)]) -> (StatusCode, String) {
        let mut builder = HttpRequest::builder().uri("/who");
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        let response = app()
            .oneshot(builder.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn only_the_proxy_gets_through_and_names_the_client() {
        assert_eq!(
            call(&[(PROXY_HEADER, SECRET), (CLIENT_HEADER, "203.0.113.7")]).await,
            (StatusCode::OK, "203.0.113.7".to_owned())
        );
        assert_eq!(
            call(&[(CLIENT_HEADER, "203.0.113.7")]).await.0,
            StatusCode::FORBIDDEN
        );
        let mut wrong = SECRET.to_owned();
        wrong.replace_range(0..1, "x");
        assert_eq!(
            call(&[(PROXY_HEADER, &wrong), (CLIENT_HEADER, "203.0.113.7")])
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&[(PROXY_HEADER, SECRET)]).await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(&[(PROXY_HEADER, SECRET), (CLIENT_HEADER, "not an ip")])
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn short_secrets_are_refused() {
        assert!(ProxySecret::new("short").is_err());
        assert!(ProxySecret::new(SECRET).is_ok());
    }
}
