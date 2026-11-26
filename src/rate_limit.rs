use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json,
    body::Body,
    extract::{ConnectInfo, State},
    http::{HeaderValue, Method, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Serialize;

const MAX_TRACKED_BUCKETS: usize = 20_000;
const CLEANUP_THRESHOLD: usize = 10_000;
const MAX_CLIENT_KEY_LEN: usize = 64;

#[derive(Serialize)]
struct RateLimitBody {
    code: &'static str,
    message: &'static str,
    global: bool,
    retry_after: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BucketKind {
    AuthStrict,
    AuthProbe,
    ReadStateAckBulk,
    ApiWrite,
    ApiRead,
}

impl BucketKind {
    const fn policy(self) -> (u32, Duration) {
        match self {
            Self::AuthStrict => (15, Duration::from_secs(60)),
            Self::AuthProbe => (30, Duration::from_secs(60)),
            Self::ReadStateAckBulk => (20, Duration::from_secs(10)),
            Self::ApiWrite => (60, Duration::from_secs(10)),
            Self::ApiRead => (120, Duration::from_secs(10)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    count: u32,
    reset_at: Instant,
}

#[derive(Clone, Default)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<(BucketKind, String), Bucket>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check(&self, kind: BucketKind, client_key: &str, now: Instant) -> Result<(), Duration> {
        let (limit, window) = kind.policy();
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());

        if map.len() >= CLEANUP_THRESHOLD {
            map.retain(|_, bucket| now < bucket.reset_at);
            if map.len() >= MAX_TRACKED_BUCKETS {
                map.clear();
            }
        }

        let key = (kind, client_key.to_string());
        match map.get_mut(&key) {
            Some(bucket) if now < bucket.reset_at => {
                if bucket.count >= limit {
                    return Err(bucket.reset_at.saturating_duration_since(now));
                }
                bucket.count += 1;
                Ok(())
            }
            Some(bucket) => {
                bucket.count = 1;
                bucket.reset_at = now + window;
                Ok(())
            }
            None => {
                map.insert(
                    key,
                    Bucket {
                        count: 1,
                        reset_at: now + window,
                    },
                );
                Ok(())
            }
        }
    }
}

fn classify_route(method: &Method, path: &str) -> Option<BucketKind> {
    if !path.starts_with("/api") {
        return None;
    }

    let stripped = path
        .strip_prefix("/api/v9")
        .or_else(|| path.strip_prefix("/api"))
        .unwrap_or(path);

    if *method == Method::POST && (stripped == "/auth/login" || stripped == "/auth/register") {
        return Some(BucketKind::AuthStrict);
    }

    if stripped == "/auth/username-availability" || stripped == "/auth/username-suggestions" {
        return Some(BucketKind::AuthProbe);
    }

    if *method == Method::POST
        && (stripped == "/read-states/ack" || stripped == "/read-states/ack-bulk")
    {
        return Some(BucketKind::ReadStateAckBulk);
    }

    if matches!(
        *method,
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE
    ) {
        Some(BucketKind::ApiWrite)
    } else {
        Some(BucketKind::ApiRead)
    }
}

fn extract_client_key(req: &Request<Body>) -> String {
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    if let Some(ip) = peer_ip
        && !ip.is_loopback()
    {
        return ip.to_string();
    }

    if let Some(real_ip) = req
        .headers()
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return real_ip.chars().take(MAX_CLIENT_KEY_LEN).collect();
    }

    if let Some(forwarded) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return forwarded.chars().take(MAX_CLIENT_KEY_LEN).collect();
    }

    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "local".to_string())
}

pub async fn middleware(
    State(limiter): State<RateLimiter>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(kind) = classify_route(req.method(), req.uri().path()) else {
        return next.run(req).await;
    };

    let client_key = extract_client_key(&req);
    if let Err(wait) = limiter.check(kind, &client_key, Instant::now()) {
        let retry_after = ((wait.as_secs_f64() * 1000.0).ceil() / 1000.0).max(0.001);
        let retry_after_header = (wait.as_secs_f64().ceil() as u64).max(1);
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(RateLimitBody {
                code: "RATE_LIMITED",
                message: "You're being rate limited.",
                global: false,
                retry_after,
            }),
        )
            .into_response();
        if let Ok(val) = HeaderValue::from_str(&retry_after_header.to_string()) {
            resp.headers_mut().insert(header::RETRY_AFTER, val);
        }
        return resp;
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_classification_skips_static_and_gateway_and_isolates_auth() {
        assert_eq!(
            classify_route(&Method::GET, "/static/emoji/1f600.svg"),
            None
        );
        assert_eq!(classify_route(&Method::GET, "/gateway"), None);
        assert_eq!(
            classify_route(&Method::POST, "/api/auth/login"),
            Some(BucketKind::AuthStrict)
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/v9/auth/register"),
            Some(BucketKind::AuthStrict)
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/auth/username-availability"),
            Some(BucketKind::AuthProbe)
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/channels/123/messages"),
            Some(BucketKind::ApiWrite)
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/read-states/ack"),
            Some(BucketKind::ReadStateAckBulk)
        );
        assert_eq!(
            classify_route(&Method::POST, "/api/v9/read-states/ack-bulk"),
            Some(BucketKind::ReadStateAckBulk)
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/ping"),
            Some(BucketKind::ApiRead)
        );
    }

    #[test]
    fn limiter_enforces_quota_and_resets_after_window() {
        let limiter = RateLimiter::new();
        let start = Instant::now();

        for _ in 0..15 {
            assert!(
                limiter
                    .check(BucketKind::AuthStrict, "1.2.3.4", start)
                    .is_ok()
            );
        }
        assert!(
            limiter
                .check(BucketKind::AuthStrict, "1.2.3.4", start)
                .is_err()
        );
        assert!(
            limiter
                .check(BucketKind::AuthStrict, "5.6.7.8", start)
                .is_ok()
        );

        let after_window = start + Duration::from_secs(61);
        assert!(
            limiter
                .check(BucketKind::AuthStrict, "1.2.3.4", after_window)
                .is_ok()
        );

        for _ in 0..20 {
            assert!(
                limiter
                    .check(BucketKind::ReadStateAckBulk, "1.2.3.4", start)
                    .is_ok()
            );
        }
        assert!(
            limiter
                .check(BucketKind::ReadStateAckBulk, "1.2.3.4", start)
                .is_err()
        );
        assert!(
            limiter
                .check(
                    BucketKind::ReadStateAckBulk,
                    "1.2.3.4",
                    start + Duration::from_secs(11)
                )
                .is_ok()
        );
    }
}
