use axum::http::HeaderMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientDialect {
    #[default]
    Fluxer,
    Discord,
}

impl ClientDialect {
    pub fn from_http(path: &str, headers: &HeaderMap) -> Self {
        for name in headers.keys() {
            if name.as_str().starts_with("x-fluxer-") {
                return Self::Fluxer;
            }
        }

        if let Some(ua) = headers.get("user-agent").and_then(|v| v.to_str().ok()) {
            let ua_lower = ua.to_ascii_lowercase();
            if ua_lower.contains("fluxer") {
                return Self::Fluxer;
            }
            if ua_lower.contains("discord") || ua_lower.contains("spacebar") {
                return Self::Discord;
            }
        }

        if headers.contains_key("x-super-properties") {
            return Self::Discord;
        }

        if let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
            if auth.starts_with("Bot ") {
                return Self::Discord;
            }
        }

        if path == "/api/v1" || path.starts_with("/api/v1/") {
            return Self::Fluxer;
        }
        if path == "/api/v9"
            || path.starts_with("/api/v9/")
            || path == "/api/v10"
            || path.starts_with("/api/v10/")
        {
            return Self::Discord;
        }

        Self::Fluxer
    }

    pub fn from_gateway_version(version: Option<u8>) -> Self {
        match version {
            Some(1) | None => Self::Fluxer,
            Some(_) => Self::Discord,
        }
    }
}
