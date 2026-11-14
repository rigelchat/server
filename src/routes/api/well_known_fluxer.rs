use axum::{Router, Json, extract::State, routing::get};
use std::collections::BTreeMap;

use crate::AppState;
use crate::models::rest::well_known::fluxer;
use crate::constants::API_VERSION;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_config))
}

async fn get_config(
    State(state): State<AppState>
) -> Json<fluxer::InstanceResponse> {
    Json(fluxer::InstanceResponse {
        codename: None,
        api_code_version: API_VERSION,
        endpoints: fluxer::Endpoints {
            api: format!("{}/api", state.config.public_http_url),
            api_client: format!("{}/api", state.config.public_http_url),
            api_public: format!("{}/api", state.config.public_http_url),
            gateway: format!("{}/gateway", state.config.public_ws_url),
            media: format!("{}/media", state.config.public_http_url),
            upload_relay: None,
            static_cdn: format!("{}/static", state.config.public_http_url),
            marketing: state.config.instance.website_url.clone().unwrap_or_default(),
            docs: None,
            admin: format!("{}/admin", state.config.public_http_url),
            invite: format!("{}/invite", state.config.public_http_url),
            gift: format!("{}/gifts", state.config.public_http_url),
            webapp: state.config.instance.webapp_url.clone().unwrap_or_default()
        },
        captcha: fluxer::Captcha {
            provider: fluxer::CaptchaProviders::None
        },
        features: fluxer::Features {
            voice_enabled: false,
            stripe_enabled: false,
            premium_enabled: false,
            stripe_serviceable: false,
            self_hosted: true,
            presigned_attachment_uploads: false,
            emails_enabled: false,
            phone_verification_enabled: false,
            desktop_modules_enabled: None,
            account_deletion_grace_period_hours: None,
            max_background_gateway_connections: None,
            account_identity: Some(fluxer::AccountIdentity::Username),
            tag_style: Some(fluxer::TagStyle::None)
        },
        gif: fluxer::Gif {
            provider: fluxer::GifProviders::Klipy,
            display_name: "Klipy".to_string(),
            attribution_required: false
        },
        sso: fluxer::Sso {
            enabled: false,
            enforced: false,
            display_name: None,
            redirect_uri: "".to_string(),
            available: Some(false)
        },
        registration: fluxer::Registration {
            mode: fluxer::RegistrationModes::Open,
            admin_registration_urls_enabled: false
        },
        community: fluxer::Community {
            single_community: false,
            single_community_guild_id: None,
            direct_messages_disabled: true,
            guild_create_access: false
        },
        services: fluxer::Services {
            gif_enabled: false,
            youtube_enabled: false,
            bluesky_enabled: false
        },
        limits: fluxer::Limits {
            version: 2,
            trait_definitions: vec![],
            rules: vec![
                fluxer::LimitRule {
                    id: "default".to_string(),
                    filters: None,
                    overrides: BTreeMap::new()
                }
            ],
            defaults_hash: "default".to_string()
        },
        push: fluxer::Push {
            public_vapid_key: None,
            delivery_mode: None,
            event_resolution_path: None,
            external_relay_url: None,
            relay_signing_key_hashes: None
        },
        app_public: fluxer::AppPublic {
            branding: fluxer::Branding {
                product_name: state.config.instance.name.clone(),
                icon_url: state.config.instance.image.clone(),
                symbol_url: None,
                logo_url: state.config.instance.image.clone(),
                wordmark_url: None,
                favicon_url: state.config.instance.image.clone(),
                theme_color: state.config.instance.color.clone(),
                status_page_url: None,
                status_page_incident_history_url: None,
                premium_product_name: "Neutronium".to_string(),
                premium_info_url: None
            },
            setup: fluxer::Setup {
                configured: true,
                admin_url: None,
                account_identity_locked: Some(false)
            },
            legal: fluxer::Legal {
                terms_url: state.config.instance.terms_url.clone(),
                privacy_url: state.config.instance.privacy_url.clone()
            },
            registration: fluxer::AppRegistration {
                collect_date_of_birth: false
            }
        },
        age_policy: None,
        domain_migration: None
    })
}