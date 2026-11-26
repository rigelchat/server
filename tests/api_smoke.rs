use std::sync::Arc;

use reqwest::StatusCode;
use serde_json::Value;
use sqlx::mysql::MySqlPoolOptions;
use tokio::net::TcpListener;

use rigel_server::{
    build_app,
    config::{
        AppConfig, AuthConfig, FeatureConfig, GuildCreationPolicy, InstanceConfig, LoginMode,
        RegistrationMode, TagStyle,
    },
    gateway::hub::GatewayHub,
    state::AppState,
};

fn test_config(
    registration_mode: RegistrationMode,
    cors_allowed_origins: Vec<String>,
) -> AppConfig {
    AppConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        public_http_url: "http://127.0.0.1:3000".to_string(),
        public_ws_url: "ws://127.0.0.1:3000".to_string(),
        cors_allowed_origins,
        database_url: "mysql://rigel:rigel@127.0.0.1:3306/rigel".to_string(),
        db_max_connections: 1,
        auth: AuthConfig {
            secret: "test_secret_with_at_least_32_characters_for_hmac".to_string(),
            login_mode: LoginMode::Username,
            tag_style: TagStyle::None,
            registration_mode,
        },
        instance: InstanceConfig {
            name: "Rigel Test".to_string(),
            description: Some("Test instance".to_string()),
            image: None,
            color: Some("#112233".to_string()),
            country_code: "FR".to_string(),
            contact_email: None,
            owner_id: None,
            website_url: None,
            terms_url: None,
            privacy_url: None,
            guidelines_url: None,
            webapp_url: None,
        },
        features: FeatureConfig {
            guild_creation_policy: GuildCreationPolicy::Staff,
            enable_attachments: false,
        },
    }
}

async fn spawn_test_server(
    registration_mode: RegistrationMode,
    cors_allowed_origins: Vec<String>,
) -> String {
    let cfg = test_config(registration_mode, cors_allowed_origins);
    let db = MySqlPoolOptions::new()
        .connect_lazy(&cfg.database_url)
        .expect("valid lazy mysql url");
    let state = AppState {
        config: Arc::new(cfg),
        db,
        hub: GatewayHub::new(),
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_app(state);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    format!("http://{addr}")
}

#[tokio::test]
async fn discovery_and_well_known_endpoints_respond_with_expected_schema() {
    let base = spawn_test_server(RegistrationMode::Open, vec!["*".to_string()]).await;
    let client = reqwest::Client::new();

    let ping_res = client.get(format!("{base}/api/ping")).send().await.unwrap();
    assert_eq!(ping_res.status(), StatusCode::OK);
    let ping_json: Value = serde_json::from_slice(&ping_res.bytes().await.unwrap()).unwrap();
    assert_eq!(ping_json["ping"], "pong!");
    assert_eq!(ping_json["instance"]["name"], "Rigel Test");

    let ping_v9 = client
        .get(format!("{base}/api/v9/ping"))
        .send()
        .await
        .unwrap();
    assert_eq!(ping_v9.status(), StatusCode::OK);

    let fluxer_res = client
        .get(format!("{base}/.well-known/fluxer"))
        .send()
        .await
        .unwrap();
    assert_eq!(fluxer_res.status(), StatusCode::OK);
    let fluxer_json: Value = serde_json::from_slice(&fluxer_res.bytes().await.unwrap()).unwrap();
    assert_eq!(fluxer_json["api_code_version"], 9);
    assert_eq!(fluxer_json["features"]["account_identity"], "username");

    let spacebar_res = client
        .get(format!("{base}/.well-known/spacebar/client"))
        .send()
        .await
        .unwrap();
    assert_eq!(spacebar_res.status(), StatusCode::OK);
    let spacebar_json: Value =
        serde_json::from_slice(&spacebar_res.bytes().await.unwrap()).unwrap();
    assert_eq!(spacebar_json["api"]["apiVersions"]["default"], 9);

    let cats_res = client
        .get(format!("{base}/api/discovery/categories"))
        .send()
        .await
        .unwrap();
    assert_eq!(cats_res.status(), StatusCode::OK);
    let cats: Vec<Value> = serde_json::from_slice(&cats_res.bytes().await.unwrap()).unwrap();
    assert_eq!(cats.len(), 9);

    let gw_res = client
        .get(format!("{base}/api/gateway/bot"))
        .send()
        .await
        .unwrap();
    assert_eq!(gw_res.status(), StatusCode::OK);
    let gw_json: Value = serde_json::from_slice(&gw_res.bytes().await.unwrap()).unwrap();
    assert_eq!(gw_json["url"], "ws://127.0.0.1:3000/gateway");
    assert_eq!(gw_json["shards"], 1);
}

#[tokio::test]
async fn auth_rejection_validation_and_closed_registration_work_deterministically() {
    let base = spawn_test_server(RegistrationMode::Closed, vec!["*".to_string()]).await;
    let client = reqwest::Client::new();

    for (method, path) in [
        (reqwest::Method::GET, "/api/users/@me"),
        (reqwest::Method::GET, "/api/users/@me/guilds"),
        (reqwest::Method::PATCH, "/api/users/@me/settings"),
        (reqwest::Method::POST, "/api/guilds"),
        (reqwest::Method::GET, "/api/channels/123456/messages"),
    ] {
        let res = client
            .request(method, format!("{base}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body: Value = serde_json::from_slice(&res.bytes().await.unwrap()).unwrap();
        assert_eq!(body["code"], "MISSING_AUTHORIZATION");
    }

    let bad_token = client
        .get(format!("{base}/api/users/@me"))
        .header("authorization", "Bearer invalid.session.sig")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_token.status(), StatusCode::UNAUTHORIZED);
    let body: Value = serde_json::from_slice(&bad_token.bytes().await.unwrap()).unwrap();
    assert_eq!(body["code"], "INVALID_AUTH_TOKEN");

    let bad_json = client
        .post(format!("{base}/api/auth/login"))
        .header("content-type", "application/json")
        .body("{not valid json")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_json.status(), StatusCode::BAD_REQUEST);
    let body: Value = serde_json::from_slice(&bad_json.bytes().await.unwrap()).unwrap();
    assert_eq!(body["code"], "INVALID_FORM_BODY");

    let reg_closed = client
        .post(format!("{base}/api/auth/register"))
        .header("content-type", "application/json")
        .body(r#"{"username":"alice","password":"strongpassword"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(reg_closed.status(), StatusCode::FORBIDDEN);
    let body: Value = serde_json::from_slice(&reg_closed.bytes().await.unwrap()).unwrap();
    assert_eq!(body["code"], "REGISTRATION_CLOSED");

    let suggest = client
        .post(format!("{base}/api/auth/username-suggestions"))
        .header("content-type", "application/json")
        .body(r#"{"global_name":"Jean Dupont!"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(suggest.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&suggest.bytes().await.unwrap()).unwrap();
    assert_eq!(body["suggestions"][0], "jean_dupont");
}

#[tokio::test]
async fn cors_allows_only_configured_origins_and_auth_rate_limit_triggers_429() {
    let base = spawn_test_server(
        RegistrationMode::Open,
        vec!["https://app.example.com".to_string()],
    )
    .await;
    let client = reqwest::Client::new();

    let allowed = client
        .get(format!("{base}/api/ping"))
        .header("origin", "https://app.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(
        allowed
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://app.example.com")
    );

    let disallowed = client
        .get(format!("{base}/api/ping"))
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert!(
        disallowed
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );

    for _ in 0..15 {
        let res = client
            .post(format!("{base}/api/auth/login"))
            .header("content-type", "application/json")
            .body("{bad")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    let throttled = client
        .post(format!("{base}/api/auth/login"))
        .header("content-type", "application/json")
        .body("{bad")
        .send()
        .await
        .unwrap();
    assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(throttled.headers().contains_key("retry-after"));
    let body: Value = serde_json::from_slice(&throttled.bytes().await.unwrap()).unwrap();
    assert_eq!(body["code"], "RATE_LIMITED");
    assert_eq!(body["global"], false);
    assert!(body["retry_after"].as_f64().is_some_and(|v| v > 0.0));
}

#[tokio::test]
async fn read_states_endpoints_require_auth_and_enforce_ack_bulk_rate_limit() {
    let base = spawn_test_server(RegistrationMode::Open, vec!["*".to_string()]).await;
    let client = reqwest::Client::new();

    for path in [
        "/api/read-states/ack",
        "/api/read-states/ack-bulk",
        "/api/channels/123456/messages/789012/ack",
    ] {
        let res = client
            .post(format!("{base}{path}"))
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body: Value = serde_json::from_slice(&res.bytes().await.unwrap()).unwrap();
        assert_eq!(body["code"], "MISSING_AUTHORIZATION");
    }

    for _ in 0..18 {
        let res = client
            .post(format!("{base}/api/read-states/ack-bulk"))
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    let throttled = client
        .post(format!("{base}/api/read-states/ack-bulk"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: Value = serde_json::from_slice(&throttled.bytes().await.unwrap()).unwrap();
    assert_eq!(body["code"], "RATE_LIMITED");
}
