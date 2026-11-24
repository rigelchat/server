use std::collections::BTreeMap;

use axum::{Json, Router, extract::State, routing::get};

use crate::AppState;
use crate::config::{GuildCreationPolicy, LoginMode, RegistrationMode, TagStyle};
use crate::constants::API_VERSION;
use crate::constants::limits::{
    MAX_ATTACHMENT_SIZE_BYTES, MAX_BIO_LENGTH, MAX_CHANNELS_PER_GUILD, MAX_GUILDS_PER_USER,
    MAX_INVITES_PER_GUILD, MAX_MESSAGE_LENGTH,
};
use crate::models::rest::well_known::fluxer;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_config))
}

async fn get_config(State(state): State<AppState>) -> Json<fluxer::InstanceResponse> {
    let cfg = &state.config;

    let account_identity = match cfg.auth.login_mode {
        LoginMode::Username => fluxer::AccountIdentity::Username,
        LoginMode::Email => fluxer::AccountIdentity::Email,
    };

    let tag_style = match cfg.auth.tag_style {
        TagStyle::None => fluxer::TagStyle::None,
        TagStyle::Random => fluxer::TagStyle::Random,
    };

    let email_delivery_state = match cfg.auth.login_mode {
        LoginMode::Username => fluxer::EmailDeliveryState::OffUsernameMode,
        LoginMode::Email => fluxer::EmailDeliveryState::OffDisabled,
    };

    let registration_mode = match cfg.auth.registration_mode {
        RegistrationMode::Open => fluxer::RegistrationModes::Open,
        RegistrationMode::Closed => fluxer::RegistrationModes::Closed,
    };

    let mut limit_overrides = BTreeMap::new();
    limit_overrides.insert("max_message_length".to_string(), MAX_MESSAGE_LENGTH as i64);
    limit_overrides.insert("max_bio_length".to_string(), MAX_BIO_LENGTH as i64);
    limit_overrides.insert("max_guilds".to_string(), MAX_GUILDS_PER_USER);
    limit_overrides.insert(
        "max_guild_channels".to_string(),
        MAX_CHANNELS_PER_GUILD as i64,
    );
    limit_overrides.insert(
        "max_guild_invites".to_string(),
        MAX_INVITES_PER_GUILD as i64,
    );
    limit_overrides.insert(
        "max_attachment_file_size".to_string(),
        MAX_ATTACHMENT_SIZE_BYTES as i64,
    );

    Json(fluxer::InstanceResponse {
        codename: None,
        api_code_version: API_VERSION,
        endpoints: fluxer::Endpoints {
            api: format!("{}/api", cfg.public_http_url),
            api_client: format!("{}/api", cfg.public_http_url),
            api_public: format!("{}/api", cfg.public_http_url),
            api_transport: None,
            gateway: format!("{}/gateway", cfg.public_ws_url),
            media: format!("{}/media", cfg.public_http_url),
            upload_relay: None,
            static_cdn: format!("{}/static", cfg.public_http_url),
            marketing: cfg.instance.website_url.clone().unwrap_or_default(),
            docs: None,
            admin: format!("{}/admin", cfg.public_http_url),
            invite: format!("{}/invite", cfg.public_http_url),
            gift: format!("{}/gifts", cfg.public_http_url),
            webapp: cfg.instance.webapp_url.clone().unwrap_or_default(),
        },
        captcha: fluxer::Captcha {
            provider: fluxer::CaptchaProviders::None,
        },
        features: fluxer::Features {
            voice_enabled: false,
            stripe_enabled: false,
            premium_enabled: false,
            stripe_serviceable: false,
            self_hosted: true,
            presigned_attachment_uploads: cfg.features.enable_attachments,
            proxy_attachments_through_api: Some(false),
            emails_enabled: false,
            phone_verification_enabled: false,
            discovery_enabled: Some(true),
            desktop_modules_enabled: None,
            account_deletion_grace_period_hours: None,
            max_background_gateway_connections: None,
            account_identity,
            tag_style,
            identifier_first: Some(false),
            instant_accounts: Some(false),
            email_codes: Some(false),
            date_of_birth_step: Some(false),
            interaction_tier: Some(fluxer::InteractionTier::Legacy),
            guild_join_gate: Some("off".to_string()),
            email_delivery_state: Some(email_delivery_state),
            verification_basis: Some(fluxer::VerificationBasis::LocalCredential),
        },
        gif: fluxer::Gif {
            provider: fluxer::GifProviders::Klipy,
            display_name: "Klipy".to_string(),
            attribution_required: false,
        },
        sso: fluxer::Sso {
            enabled: false,
            enforced: false,
            username_from_provider: false,
            display_name: None,
            button_label: None,
            redirect_uri: String::new(),
            available: Some(false),
        },
        registration: fluxer::Registration {
            mode: registration_mode,
            admin_registration_urls_enabled: false,
        },
        community: fluxer::Community {
            single_community: false,
            single_community_guild_id: None,
            direct_messages_disabled: true,
            friends_disabled: Some(true),
            personal_notes_enabled: Some(false),
            guild_create_access: cfg.features.guild_creation_policy
                == GuildCreationPolicy::Everyone,
        },
        services: fluxer::Services {
            gif_enabled: false,
            youtube_enabled: false,
            bluesky_enabled: false,
            connection_providers: vec![],
        },
        limits: fluxer::Limits {
            version: 2,
            trait_definitions: vec![],
            rules: vec![fluxer::LimitRule {
                id: "default".to_string(),
                filters: None,
                overrides: limit_overrides,
            }],
            defaults_hash: "default".to_string(),
        },
        push: fluxer::Push {
            public_vapid_key: None,
            delivery_mode: None,
            event_resolution_path: None,
            external_relay_url: None,
            relay_signing_key_hashes: None,
        },
        app_public: fluxer::AppPublic {
            branding: fluxer::Branding {
                product_name: cfg.instance.name.clone(),
                icon_url: cfg.instance.image.clone(),
                symbol_url: None,
                logo_url: cfg.instance.image.clone(),
                wordmark_url: None,
                favicon_url: cfg.instance.image.clone(),
                favicon_dark_url: None,
                theme_color: cfg.instance.color.clone(),
                status_page_url: None,
                status_page_instatus: false,
                status_page_incident_history_url: None,
                premium_product_name: "Neutronium".to_string(),
                premium_info_url: None,
                sign_in_backdrop_url: None,
                premium_page_color: None,
                premium_showcase_expressions_url: None,
                premium_showcase_profile_url: None,
                premium_showcase_stream_url: None,
                premium_showcase_upload_url: None,
                custom_css: None,
                upload_limit_message: None,
                trusted_domains: vec![],
            },
            setup: fluxer::Setup {
                configured: true,
                admin_url: None,
                account_identity_locked: Some(true),
            },
            legal: fluxer::Legal {
                terms_url: cfg.instance.terms_url.clone(),
                privacy_url: cfg.instance.privacy_url.clone(),
                guidelines_url: cfg.instance.guidelines_url.clone(),
            },
            registration: fluxer::AppRegistration {
                collect_date_of_birth: false,
            },
        },
        age_policy: fluxer::AgePolicy { geos: vec![] },
        domain_migration: None,
    })
}
