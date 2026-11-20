use std::fmt;

use axum::{
    Json,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::constants::dialect::ClientDialect;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorCode {
    BadRequest,
    InvalidFormBody,
    Unauthorized,
    MissingAuthorization,
    InvalidAuthToken,
    MissingPermissions,
    NotFound,
    UnknownUser,
    UnknownGuild,
    UnknownChannel,
    UnknownMessage,
    UnknownInvite,
    UnknownMember,
    InvalidChannelType,
    CannotSendEmptyMessage,
    CannotSendMessagesInNonTextChannel,
    CannotEditOtherUserMessage,
    UserBannedFromGuild,
    Conflict,
    RegistrationClosed,
    MaxInvites,
    MaxGuilds,
    InternalServerError,
}

impl ApiErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "BAD_REQUEST",
            Self::InvalidFormBody => "INVALID_FORM_BODY",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::MissingAuthorization => "MISSING_AUTHORIZATION",
            Self::InvalidAuthToken => "INVALID_AUTH_TOKEN",
            Self::MissingPermissions => "MISSING_PERMISSIONS",
            Self::NotFound => "NOT_FOUND",
            Self::UnknownUser => "UNKNOWN_USER",
            Self::UnknownGuild => "UNKNOWN_GUILD",
            Self::UnknownChannel => "UNKNOWN_CHANNEL",
            Self::UnknownMessage => "UNKNOWN_MESSAGE",
            Self::UnknownInvite => "UNKNOWN_INVITE",
            Self::UnknownMember => "UNKNOWN_MEMBER",
            Self::InvalidChannelType => "INVALID_CHANNEL_TYPE",
            Self::CannotSendEmptyMessage => "CANNOT_SEND_EMPTY_MESSAGE",
            Self::CannotSendMessagesInNonTextChannel => "CANNOT_SEND_MESSAGES_IN_NON_TEXT_CHANNEL",
            Self::CannotEditOtherUserMessage => "CANNOT_EDIT_OTHER_USER_MESSAGE",
            Self::UserBannedFromGuild => "USER_BANNED_FROM_GUILD",
            Self::Conflict => "CONFLICT",
            Self::RegistrationClosed => "REGISTRATION_CLOSED",
            Self::MaxInvites => "MAX_INVITES",
            Self::MaxGuilds => "MAX_GUILDS",
            Self::InternalServerError => "INTERNAL_SERVER_ERROR",
        }
    }

    pub fn discord_code(self) -> u32 {
        match self {
            Self::InvalidFormBody => 50035,
            Self::Unauthorized | Self::MissingAuthorization => 40001,
            Self::InvalidAuthToken => 50014,
            Self::MissingPermissions => 50013,
            Self::UnknownChannel => 10003,
            Self::UnknownGuild => 10004,
            Self::UnknownInvite => 10006,
            Self::UnknownMember => 10007,
            Self::UnknownMessage => 10008,
            Self::UnknownUser => 10013,
            Self::MaxGuilds => 30001,
            Self::MaxInvites => 30016,
            Self::InvalidChannelType => 50024,
            Self::CannotSendMessagesInNonTextChannel => 50008,
            Self::CannotEditOtherUserMessage => 50005,
            Self::CannotSendEmptyMessage => 50006,
            Self::UserBannedFromGuild => 40007,
            _ => 0,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationCode {
    InvalidLoginOrPassword,
    UsernameLengthInvalid,
    UsernameInvalidCharacters,
    UsernameReservedValue,
    UsernameCannotContainReservedTerms,
    UsernameAlreadyTaken,
    GlobalNameLengthInvalid,
    PasswordLengthInvalid,
    EmailIsRequired,
    InvalidEmailFormat,
    EmailAlreadyInUse,
    ContentExceedsMaxLength,
    ColorValueTooHigh,
    StringLengthInvalid,
    InvalidFormat,
    CannotLeaveGuildAsOwner,
}

impl ValidationCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidLoginOrPassword => "INVALID_LOGIN_OR_PASSWORD",
            Self::UsernameLengthInvalid => "USERNAME_LENGTH_INVALID",
            Self::UsernameInvalidCharacters => "USERNAME_INVALID_CHARACTERS",
            Self::UsernameReservedValue => "USERNAME_RESERVED_VALUE",
            Self::UsernameCannotContainReservedTerms => "USERNAME_CANNOT_CONTAIN_RESERVED_TERMS",
            Self::UsernameAlreadyTaken => "USERNAME_ALREADY_TAKEN",
            Self::GlobalNameLengthInvalid => "GLOBAL_NAME_LENGTH_INVALID",
            Self::PasswordLengthInvalid => "PASSWORD_LENGTH_INVALID",
            Self::EmailIsRequired => "EMAIL_IS_REQUIRED",
            Self::InvalidEmailFormat => "INVALID_EMAIL_FORMAT",
            Self::EmailAlreadyInUse => "EMAIL_ALREADY_IN_USE",
            Self::ContentExceedsMaxLength => "CONTENT_EXCEEDS_MAX_LENGTH",
            Self::ColorValueTooHigh => "COLOR_VALUE_TOO_HIGH",
            Self::StringLengthInvalid => "STRING_LENGTH_INVALID",
            Self::InvalidFormat => "INVALID_FORMAT",
            Self::CannotLeaveGuildAsOwner => "CANNOT_LEAVE_GUILD_AS_OWNER",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationError {
    pub path: String,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone)]
struct ApiErrorMeta {
    status: StatusCode,
    code: ApiErrorCode,
    message: String,
}

fn slice_is_empty(errors: &&[ValidationError]) -> bool {
    errors.is_empty()
}

#[derive(Serialize)]
struct FluxerErrorBody<'a> {
    code: &'static str,
    message: &'a str,
    #[serde(skip_serializing_if = "slice_is_empty")]
    errors: &'a [ValidationError],
}

#[derive(Serialize)]
struct DiscordErrorBody<'a> {
    code: u32,
    message: &'a str,
}

#[derive(Debug)]
pub enum AppError {
    Internal(String),
    Coded {
        status: StatusCode,
        code: ApiErrorCode,
        message: String,
        errors: Vec<ValidationError>,
    },
}

#[allow(dead_code)]
impl AppError {
    pub fn coded(status: StatusCode, code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self::Coded {
            status,
            code,
            message: message.into(),
            errors: Vec::new(),
        }
    }

    pub fn validation(
        path: impl Into<String>,
        code: ValidationCode,
        message: impl Into<String>,
    ) -> Self {
        Self::Coded {
            status: StatusCode::BAD_REQUEST,
            code: ApiErrorCode::InvalidFormBody,
            message: "Invalid form body.".to_string(),
            errors: vec![ValidationError {
                path: path.into(),
                code: code.as_str(),
                message: message.into(),
            }],
        }
    }

    pub fn invalid_form_body() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::InvalidFormBody,
            "Invalid form body.",
        )
    }

    pub fn missing_auth() -> Self {
        Self::coded(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::MissingAuthorization,
            "Missing or invalid authorization header.",
        )
    }

    pub fn invalid_token(message: impl Into<String>) -> Self {
        Self::coded(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::InvalidAuthToken,
            message,
        )
    }

    pub fn unauthorized() -> Self {
        Self::coded(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::Unauthorized,
            "Unauthorized",
        )
    }

    pub fn missing_permissions() -> Self {
        Self::coded(
            StatusCode::FORBIDDEN,
            ApiErrorCode::MissingPermissions,
            "You don't have the permissions required to perform this action.",
        )
    }

    pub fn missing_permissions_with(message: impl Into<String>) -> Self {
        Self::coded(
            StatusCode::FORBIDDEN,
            ApiErrorCode::MissingPermissions,
            message,
        )
    }

    pub fn registration_closed(message: impl Into<String>) -> Self {
        Self::coded(
            StatusCode::FORBIDDEN,
            ApiErrorCode::RegistrationClosed,
            message,
        )
    }

    pub fn unknown_user() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownUser,
            "Unknown user",
        )
    }

    pub fn unknown_guild() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownGuild,
            "Unknown guild",
        )
    }

    pub fn unknown_channel() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownChannel,
            "Unknown channel",
        )
    }

    pub fn unknown_message() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownMessage,
            "Unknown message",
        )
    }

    pub fn unknown_invite() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownInvite,
            "Unknown invite",
        )
    }

    pub fn unknown_member() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownMember,
            "Unknown member",
        )
    }

    pub fn banned_from_guild() -> Self {
        Self::coded(
            StatusCode::FORBIDDEN,
            ApiErrorCode::UserBannedFromGuild,
            "This user is banned from this community.",
        )
    }

    pub fn empty_message() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::CannotSendEmptyMessage,
            "You can't send an empty message.",
        )
    }

    pub fn cannot_send_in_non_text_channel() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::CannotSendMessagesInNonTextChannel,
            "You can't send messages in a non-text channel.",
        )
    }

    pub fn invalid_channel_type() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::InvalidChannelType,
            "Invalid channel type.",
        )
    }

    pub fn cannot_edit_other_message() -> Self {
        Self::coded(
            StatusCode::FORBIDDEN,
            ApiErrorCode::CannotEditOtherUserMessage,
            "You can't edit another user's message.",
        )
    }

    pub fn max_guilds() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxGuilds,
            "You have reached the maximum number of communities.",
        )
    }

    pub fn max_invites() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxInvites,
            "Maximum number of invites reached for this community.",
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Internal(msg) => write!(f, "{msg}"),
            Self::Coded { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message, errors) = match self {
            Self::Internal(err) => {
                tracing::error!(error = %err, "Internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ApiErrorCode::InternalServerError,
                    "Internal server error".to_string(),
                    Vec::new(),
                )
            }
            Self::Coded {
                status,
                code,
                message,
                errors,
            } => (status, code, message, errors),
        };

        let mut response = (
            status,
            Json(FluxerErrorBody {
                code: code.as_str(),
                message: &message,
                errors: &errors,
            }),
        )
            .into_response();

        response
            .extensions_mut()
            .insert(ApiErrorMeta { status, code, message });

        response
    }
}

pub async fn adapt_error_dialect(mut req: Request, next: Next) -> Response {
    let dialect = ClientDialect::from_http(req.headers());
    req.extensions_mut().insert(dialect);

    let res = next.run(req).await;

    if dialect == ClientDialect::Discord {
        if let Some(meta) = res.extensions().get::<ApiErrorMeta>() {
            return (
                meta.status,
                Json(DiscordErrorBody {
                    code: meta.code.discord_code(),
                    message: &meta.message,
                }),
            )
                .into_response();
        }
    }

    res
}