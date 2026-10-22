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
//! * In **direct** mode (`ZEEK_CLIENT_IP_HEADER`, for a host whose own edge
//!   is the only way in, such as Railway) the public listener also serves
//!   requests without the secret, so the app can call it straight from the
//!   browser. The client is then the last address in that header, the one
//!   the host's edge appended (earlier entries are whatever the client
//!   sent). A request that carries a wrong secret is still refused.
//!
//! Rate limits ([`crate::relay::RateLimiter`]) key on the resulting
//! [`ClientIp`].

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderName, StatusCode};
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

/// The client a request is counted against. IPv6 clients are counted per
/// /64, the block one subscriber usually holds (review N5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

impl ClientIp {
    /// The client for address `ip`.
    pub fn new(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(_) => ClientIp(ip),
            IpAddr::V6(v6) => {
                let mut octets = v6.octets();
                for o in &mut octets[8..] {
                    *o = 0;
                }
                ClientIp(IpAddr::from(octets))
            }
        }
    }
}

/// Requests per minute one client may make to the public listener, and all
/// clients together.
pub const PUBLIC_PER_CLIENT_PER_MIN: u32 = 600;
pub const PUBLIC_GLOBAL_PER_MIN: u32 = 6_000;

/// The public listener's limits.
pub struct PublicLimits {
    per_client: crate::relay::RateLimiter,
    global: crate::relay::RateLimiter,
}

impl Default for PublicLimits {
    fn default() -> Self {
        PublicLimits {
            per_client: crate::relay::RateLimiter::new(PUBLIC_PER_CLIENT_PER_MIN),
            global: crate::relay::RateLimiter::new(PUBLIC_GLOBAL_PER_MIN),
        }
    }
}

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
    request.extensions_mut().insert(ClientIp::new(ip));
    next.run(request).await
}

/// Who may call the public listener.
#[derive(Clone)]
pub struct PublicAccess {
    secret: ProxySecret,
    /// Direct mode: the header the host's edge sets to the client address.
    direct: Option<HeaderName>,
}

impl PublicAccess {
    /// Only the proxy (the secret is required).
    pub fn proxy_only(secret: ProxySecret) -> Self {
        PublicAccess {
            secret,
            direct: None,
        }
    }

    /// The proxy, and anyone else through the host's edge, which names the
    /// client in `header`.
    pub fn direct(secret: ProxySecret, header: &str) -> Result<Self, EngineError> {
        let header = HeaderName::try_from(header.trim().to_ascii_lowercase())
            .map_err(|_| EngineError::Config(format!("`{header}` is not a header name")))?;
        if header.as_str() == PROXY_HEADER || header.as_str() == CLIENT_HEADER {
            return Err(EngineError::Config(format!(
                "the client address header cannot be `{header}`"
            )));
        }
        Ok(PublicAccess {
            secret,
            direct: Some(header),
        })
    }

    /// Whether requests without the secret are served.
    pub fn is_direct(&self) -> bool {
        self.direct.is_some()
    }
}

/// The last address in a header value: `203.0.113.7`, `a, b, 203.0.113.7`,
/// `[2001:db8::1]:443` or `203.0.113.7:443`.
fn last_address(value: &str) -> Option<IpAddr> {
    let last = value.rsplit(',').next()?.trim();
    last.parse::<IpAddr>()
        .ok()
        .or_else(|| last.parse::<SocketAddr>().ok().map(|a| a.ip()))
}

async fn proxied(
    State((access, limits)): State<(PublicAccess, Arc<PublicLimits>)>,
    mut request: Request,
    next: Next,
) -> Response {
    let given = request.headers().get(PROXY_HEADER);
    let authentic = given.is_some_and(|v| access.secret.matches(v.as_bytes()));
    let client = if authentic {
        let client = request
            .headers()
            .get(CLIENT_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<IpAddr>().ok());
        let Some(ip) = client else {
            return refused(StatusCode::BAD_REQUEST, "the proxy did not name the client");
        };
        ip
    } else {
        // A wrong secret is refused in every mode; no secret is fine only
        // in direct mode.
        let Some(header) = access.direct.as_ref().filter(|_| given.is_none()) else {
            return refused(
                StatusCode::FORBIDDEN,
                "use the public site; this address only serves its proxy",
            );
        };
        let client = request
            .headers()
            .get(header)
            .and_then(|v| v.to_str().ok())
            .and_then(last_address);
        let Some(ip) = client else {
            return refused(StatusCode::BAD_REQUEST, "the client address is missing");
        };
        ip
    };
    // Nothing downstream needs the secret or the forwarded address header.
    request.headers_mut().remove(PROXY_HEADER);
    request.headers_mut().remove(CLIENT_HEADER);
    let ip = client;
    let client = ClientIp::new(ip);
    let now = std::time::Instant::now();
    if !(limits.global.allow(IpAddr::from([0, 0, 0, 0]), now).await
        && limits.per_client.allow(client.0, now).await)
    {
        return refused(
            StatusCode::TOO_MANY_REQUESTS,
            "too many requests; try again in a minute",
        );
    }
    request.extensions_mut().insert(client);
    next.run(request).await
}

/// The router for the local listener.
pub fn local(router: Router) -> Router {
    router.layer(middleware::from_fn(peer))
}

/// The router for the public listener: the proxy (and, in direct mode,
/// anyone through the host's edge) may call it, and every route is rate
/// limited per client and in total.
pub fn public(router: Router, access: PublicAccess) -> Router {
    router.layer(middleware::from_fn_with_state(
        (access, Arc::new(PublicLimits::default())),
        proxied,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use tower::ServiceExt;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn secret() -> ProxySecret {
        ProxySecret::new(SECRET).expect("secret")
    }

    fn app(access: PublicAccess) -> Router {
        let echo = Router::new().route(
            "/who",
            get(|request: Request| async move {
                // The forwarded address header never reaches the routes.
                assert!(request.headers().get(CLIENT_HEADER).is_none());
                ClientIp::of(&request).to_string()
            }),
        );
        public(echo, access)
    }

    async fn call(headers: &[(&str, &str)]) -> (StatusCode, String) {
        call_with(PublicAccess::proxy_only(secret()), headers).await
    }

    async fn call_direct(headers: &[(&str, &str)]) -> (StatusCode, String) {
        let access = PublicAccess::direct(secret(), "X-Forwarded-For").expect("header");
        call_with(access, headers).await
    }

    async fn call_with(access: PublicAccess, headers: &[(&str, &str)]) -> (StatusCode, String) {
        let mut builder = HttpRequest::builder().uri("/who");
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        let response = app(access)
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

    #[tokio::test]
    async fn direct_mode_takes_the_address_the_edge_appended() {
        // What the client sent itself comes first; the edge's entry is last.
        assert_eq!(
            call_direct(&[("x-forwarded-for", "10.0.0.1, 203.0.113.9")]).await,
            (StatusCode::OK, "203.0.113.9".to_owned())
        );
        assert_eq!(
            call_direct(&[("x-forwarded-for", "[2001:db8:1:2::5]:443")]).await,
            (StatusCode::OK, "2001:db8:1:2::".to_owned())
        );
        // A client cannot pick its bucket with the proxy's header.
        assert_eq!(
            call_direct(&[
                ("x-forwarded-for", "203.0.113.9"),
                (CLIENT_HEADER, "10.9.9.9")
            ])
            .await,
            (StatusCode::OK, "203.0.113.9".to_owned())
        );
        assert_eq!(call_direct(&[]).await.0, StatusCode::BAD_REQUEST);
        // The proxy still names its client.
        assert_eq!(
            call_direct(&[(PROXY_HEADER, SECRET), (CLIENT_HEADER, "198.51.100.4")]).await,
            (StatusCode::OK, "198.51.100.4".to_owned())
        );
        // A wrong secret is refused, not served as a direct request.
        assert_eq!(
            call_direct(&[(PROXY_HEADER, "wrong"), ("x-forwarded-for", "203.0.113.9")])
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert!(PublicAccess::direct(secret(), "x-zeek-client").is_err());
        assert!(PublicAccess::direct(secret(), "not a header").is_err());
    }

    #[test]
    fn ipv6_clients_count_per_64() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().expect("ip");
        let b: IpAddr = "2001:db8:1:2:bbbb::9".parse().expect("ip");
        let c: IpAddr = "2001:db8:1:3::1".parse().expect("ip");
        assert_eq!(ClientIp::new(a), ClientIp::new(b));
        assert_ne!(ClientIp::new(a), ClientIp::new(c));
        let v4: IpAddr = "203.0.113.7".parse().expect("ip");
        assert_eq!(ClientIp::new(v4).0, v4);
    }

    #[test]
    fn short_secrets_are_refused() {
        assert!(ProxySecret::new("short").is_err());
        assert!(ProxySecret::new(SECRET).is_ok());
    }
}
