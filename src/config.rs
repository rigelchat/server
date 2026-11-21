use std::env;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginMode {
    Username,
    Email,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagStyle {
    None,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationMode {
    Open,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuildCreationPolicy {
    Staff,
    Everyone,
}

#[derive(Clone, Debug)]
pub struct InstanceConfig {
    pub name: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub color: Option<String>,
    pub country_code: String,
    pub contact_email: Option<String>,
    pub owner_id: Option<String>,
    pub website_url: Option<String>,
    pub terms_url: Option<String>,
    pub privacy_url: Option<String>,
    pub guidelines_url: Option<String>,
    pub webapp_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AuthConfig {
    pub secret: String,
    pub login_mode: LoginMode,
    pub tag_style: TagStyle,
    pub registration_mode: RegistrationMode,
}

#[derive(Clone, Debug)]
pub struct FeatureConfig {
    pub guild_creation_policy: GuildCreationPolicy,
    pub enable_attachments: bool,
    pub enable_auto_emoji_sync: bool,
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub public_http_url: String,
    pub public_ws_url: String,
    pub cors_allowed_origins: Vec<String>,
    pub database_url: String,
    pub db_max_connections: u32,
    pub auth: AuthConfig,
    pub instance: InstanceConfig,
    pub features: FeatureConfig,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let cors_allowed_origins = optional_env("CORS_ALLOWED_ORIGINS")
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let login_mode = match optional_env("AUTH_LOGIN_MODE")
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("email") => LoginMode::Email,
            _ => LoginMode::Username,
        };

        let tag_style = match optional_env("AUTH_TAG_STYLE")
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("random") => TagStyle::Random,
            _ => TagStyle::None,
        };

        let registration_mode = match optional_env("AUTH_REGISTRATION_MODE")
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("closed") => RegistrationMode::Closed,
            _ => RegistrationMode::Open,
        };

        let guild_creation_policy = match optional_env("GUILD_CREATION_POLICY")
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("everyone") => GuildCreationPolicy::Everyone,
            _ => GuildCreationPolicy::Staff,
        };

        Self {
            host: optional_env("HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: env_parse("PORT", 3000),
            public_http_url: require_env("PUBLIC_HTTP_URL"),
            public_ws_url: require_env("PUBLIC_WS_URL"),
            cors_allowed_origins,
            database_url: require_env("DATABASE_URL"),
            db_max_connections: env_parse("DB_MAX_CONNECTIONS", 16),
            auth: AuthConfig {
                secret: {
                    let s = require_env("AUTH_SECRET");
                    if s == "replace_with_a_secure_random_secret" || s.len() < 32 {
                        tracing::warn!(
                            "AUTH_SECRET is weak or uses the example placeholder; set a random secret of at least 32 characters in production"
                        );
                    }
                    s
                },
                login_mode,
                tag_style,
                registration_mode,
            },
            instance: InstanceConfig {
                name: optional_env("INSTANCE_NAME").unwrap_or_else(|| "Rigel".to_string()),
                description: optional_env("INSTANCE_DESCRIPTION"),
                image: optional_env("INSTANCE_IMAGE"),
                color: optional_env("INSTANCE_COLOR"),
                country_code: optional_env("INSTANCE_COUNTRY_CODE")
                    .unwrap_or_else(|| "FR".to_string())
                    .to_ascii_uppercase(),
                contact_email: optional_env("INSTANCE_CONTACT_EMAIL"),
                owner_id: optional_env("INSTANCE_OWNER_ID"),
                website_url: optional_env("INSTANCE_WEBSITE_URL"),
                terms_url: optional_env("INSTANCE_TERMS_URL"),
                privacy_url: optional_env("INSTANCE_PRIVACY_URL"),
                guidelines_url: optional_env("INSTANCE_GUIDELINES_URL"),
                webapp_url: optional_env("INSTANCE_WEBAPP_URL"),
            },
            features: FeatureConfig {
                guild_creation_policy,
                enable_attachments: env_bool("ENABLE_ATTACHMENTS", false),
                enable_auto_emoji_sync: env_bool("ENABLE_AUTO_EMOJI_SYNC", true),
            },
        }
    }
}

fn optional_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn require_env(key: &str) -> String {
    optional_env(key).unwrap_or_else(|| panic!("Environment variable '{key}' must be set"))
}

fn env_bool(key: &str, default: bool) -> bool {
    match optional_env(key)
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("true" | "1" | "yes") => true,
        Some("false" | "0" | "no") => false,
        _ => default,
    }
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    optional_env(key)
        .and_then(|v| v.parse::<T>().ok())
        .unwrap_or(default)
}
