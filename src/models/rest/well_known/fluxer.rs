#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::Serialize;

/// See: `https://docs.fluxer.app/http-api/instance`
#[derive(Serialize)]
pub struct InstanceResponse {
    pub codename: Option<String>,
    pub api_code_version: u8,
    pub endpoints: Endpoints,
    pub captcha: Captcha,
    pub features: Features,
    pub gif: Gif,
    pub sso: Sso,
    pub registration: Registration,
    pub community: Community,
    pub services: Services,
    pub limits: Limits,
    pub push: Push,
    pub app_public: AppPublic,
    pub age_policy: AgePolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_migration: Option<DomainMigration>,
}

#[derive(Serialize)]
pub struct Endpoints {
    pub api: String,
    pub api_client: String,
    pub api_public: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_transport: Option<String>,
    pub gateway: String,
    pub media: String,
    pub upload_relay: Option<String>,
    pub static_cdn: String,
    pub marketing: String,
    pub docs: Option<String>,
    pub admin: String,
    pub invite: String,
    pub gift: String,
    pub webapp: String,
}

#[derive(Serialize)]
pub struct Captcha {
    pub provider: CaptchaProviders,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptchaProviders {
    Altcha,
    None,
}

#[derive(Serialize)]
pub struct Features {
    pub voice_enabled: bool,
    pub stripe_enabled: bool,
    pub premium_enabled: bool,
    pub stripe_serviceable: bool,
    pub self_hosted: bool,
    pub presigned_attachment_uploads: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_attachments_through_api: Option<bool>,
    pub emails_enabled: bool,
    pub phone_verification_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery_enabled: Option<bool>,
    pub desktop_modules_enabled: Option<bool>,
    pub account_deletion_grace_period_hours: Option<u32>,
    pub max_background_gateway_connections: Option<u8>,
    pub account_identity: AccountIdentity,
    pub tag_style: TagStyle,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier_first: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instant_accounts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_codes: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_of_birth_step: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction_tier: Option<InteractionTier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_join_gate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_delivery_state: Option<EmailDeliveryState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_basis: Option<VerificationBasis>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountIdentity {
    Email,
    Username,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TagStyle {
    None,
    Random,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InteractionTier {
    Legacy,
    Enforced,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmailDeliveryState {
    On,
    OffUsernameMode,
    OffDisabled,
    OffIncomplete,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationBasis {
    ConfirmedEmail,
    LocalCredential,
}

#[derive(Serialize)]
pub struct Gif {
    pub provider: GifProviders,
    pub display_name: String,
    pub attribution_required: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GifProviders {
    Klipy,
}

#[derive(Serialize)]
pub struct Sso {
    pub enabled: bool,
    pub enforced: bool,
    pub username_from_provider: bool,
    pub display_name: Option<String>,
    pub button_label: Option<String>,
    pub redirect_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available: Option<bool>,
}

#[derive(Serialize)]
pub struct Registration {
    pub mode: RegistrationModes,
    pub admin_registration_urls_enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistrationModes {
    Open,
    Approval,
    Closed,
}

#[derive(Serialize)]
pub struct Community {
    pub single_community: bool,
    pub single_community_guild_id: Option<String>,
    pub direct_messages_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friends_disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub personal_notes_enabled: Option<bool>,
    pub guild_create_access: bool,
}

#[derive(Serialize)]
pub struct ConnectionProvider {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
}

#[derive(Serialize)]
pub struct Services {
    pub gif_enabled: bool,
    pub youtube_enabled: bool,
    pub bluesky_enabled: bool,
    pub connection_providers: Vec<ConnectionProvider>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub version: u8,
    pub trait_definitions: Vec<String>,
    pub rules: Vec<LimitRule>,
    pub defaults_hash: String,
}

#[derive(Serialize)]
pub struct LimitRule {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<LimitFilters>,
    pub overrides: BTreeMap<String, i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traits: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_features: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct Push {
    pub public_vapid_key: Option<String>,
    pub delivery_mode: Option<PushDeliveryMode>,
    pub event_resolution_path: Option<String>,
    pub external_relay_url: Option<String>,
    pub relay_signing_key_hashes: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PushDeliveryMode {
    Direct,
    ExternalRelay,
    Hybrid,
}

#[derive(Serialize)]
pub struct AppPublic {
    pub branding: Branding,
    pub setup: Setup,
    pub legal: Legal,
    pub registration: AppRegistration,
}

#[derive(Serialize)]
pub struct Branding {
    pub product_name: String,
    pub icon_url: Option<String>,
    pub symbol_url: Option<String>,
    pub logo_url: Option<String>,
    pub wordmark_url: Option<String>,
    pub favicon_url: Option<String>,
    pub favicon_dark_url: Option<String>,
    pub theme_color: Option<String>,
    pub status_page_url: Option<String>,
    pub status_page_instatus: bool,
    pub status_page_incident_history_url: Option<String>,
    pub premium_product_name: String,
    pub premium_info_url: Option<String>,
    pub sign_in_backdrop_url: Option<String>,
    pub premium_page_color: Option<String>,
    pub premium_showcase_expressions_url: Option<String>,
    pub premium_showcase_profile_url: Option<String>,
    pub premium_showcase_stream_url: Option<String>,
    pub premium_showcase_upload_url: Option<String>,
    pub custom_css: Option<String>,
    pub upload_limit_message: Option<String>,
    pub trusted_domains: Vec<String>,
}

#[derive(Serialize)]
pub struct Setup {
    pub configured: bool,
    pub admin_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_identity_locked: Option<bool>,
}

#[derive(Serialize)]
pub struct Legal {
    pub terms_url: Option<String>,
    pub privacy_url: Option<String>,
    pub guidelines_url: Option<String>,
}

#[derive(Serialize)]
pub struct AppRegistration {
    pub collect_date_of_birth: bool,
}

#[derive(Serialize)]
pub struct AgePolicy {
    pub geos: Vec<AgePolicyGeo>,
}

#[derive(Serialize)]
pub struct AgePolicyGeo {
    pub country_code: String,
    pub region_code: Option<String>,
    pub action: AgePolicyAction,
    pub card_verification_available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgePolicyAction {
    Restrict,
    Block,
}

#[derive(Serialize)]
pub struct DomainMigration {
    pub enabled: bool,
    pub anonymous_rollout_basis_points: u32,
    pub rollout_salt: String,
    pub standalone_forwarding: bool,
}
