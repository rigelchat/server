use std::fmt;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorCode {
    BadRequest,
    InvalidFormBody,
    Unauthorized,
    MissingAuthorization,
    InvalidAuthToken,
    MissingPermissions,
    MissingAccess,
    CommunicationDisabled,
    AccessDenied,
    NotFound,
    UnknownUser,
    UnknownGuild,
    UnknownChannel,
    UnknownMessage,
    UnknownInvite,
    UnknownMember,
    UnknownForumTag,
    UnknownEmoji,
    UnknownThreadMember,
    InvalidChannelType,
    CannotSendEmptyMessage,
    CannotSendMessagesInNonTextChannel,
    CannotEditOtherUserMessage,
    UserBannedFromGuild,
    Conflict,
    RegistrationClosed,
    MaxInvites,
    MaxGuilds,
    MaxGuildChannels,
    MaxCategoryChannels,
    ChannelTypeConversionNotSupported,
    UnknownRole,
    MaxGuildRoles,
    GuildTemplateInvalid,
    MaxForumTags,
    ForumTagNamesMustBeUnique,
    NoTagsAvailableToNonModerators,
    HideMediaDownloadOptionMediaOnly,
    ForumTagRequired,
    MaxPinnedThreadsInForum,
    MaxActiveThreads,
    ThreadArchived,
    ThreadLocked,
    ThreadAlreadyCreatedForMessage,
    CannotModifySystemWebhook,
    MaxThreadMembers,
    InvalidThreadNotificationSettings,
    SlowmodeRateLimited,
    ResourceLocked,
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
            Self::MissingAccess => "MISSING_ACCESS",
            Self::CommunicationDisabled => "COMMUNICATION_DISABLED",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::NotFound => "NOT_FOUND",
            Self::UnknownUser => "UNKNOWN_USER",
            Self::UnknownGuild => "UNKNOWN_GUILD",
            Self::UnknownChannel => "UNKNOWN_CHANNEL",
            Self::UnknownMessage => "UNKNOWN_MESSAGE",
            Self::UnknownInvite => "UNKNOWN_INVITE",
            Self::UnknownMember => "UNKNOWN_MEMBER",
            Self::UnknownForumTag => "UNKNOWN_FORUM_TAG",
            Self::UnknownEmoji => "UNKNOWN_EMOJI",
            Self::UnknownThreadMember => "UNKNOWN_THREAD_MEMBER",
            Self::InvalidChannelType => "INVALID_CHANNEL_TYPE",
            Self::CannotSendEmptyMessage => "CANNOT_SEND_EMPTY_MESSAGE",
            Self::CannotSendMessagesInNonTextChannel => "CANNOT_SEND_MESSAGES_IN_NON_TEXT_CHANNEL",
            Self::CannotEditOtherUserMessage => "CANNOT_EDIT_OTHER_USER_MESSAGE",
            Self::UserBannedFromGuild => "USER_BANNED_FROM_GUILD",
            Self::Conflict => "CONFLICT",
            Self::RegistrationClosed => "REGISTRATION_CLOSED",
            Self::MaxInvites => "MAX_INVITES",
            Self::MaxGuilds => "MAX_GUILDS",
            Self::MaxGuildChannels => "MAX_GUILD_CHANNELS",
            Self::MaxCategoryChannels => "MAX_CATEGORY_CHANNELS",
            Self::ChannelTypeConversionNotSupported => "CHANNEL_TYPE_CONVERSION_NOT_SUPPORTED",
            Self::UnknownRole => "UNKNOWN_ROLE",
            Self::MaxGuildRoles => "MAX_GUILD_ROLES",
            Self::GuildTemplateInvalid => "GUILD_TEMPLATE_INVALID",
            Self::MaxForumTags => "MAX_FORUM_TAGS",
            Self::ForumTagNamesMustBeUnique => "FORUM_TAG_NAMES_MUST_BE_UNIQUE",
            Self::NoTagsAvailableToNonModerators => "NO_TAGS_AVAILABLE_TO_NON_MODERATORS",
            Self::HideMediaDownloadOptionMediaOnly => "HIDE_MEDIA_DOWNLOAD_OPTION_MEDIA_ONLY",
            Self::ForumTagRequired => "FORUM_TAG_REQUIRED",
            Self::MaxPinnedThreadsInForum => "MAX_PINNED_THREADS_IN_FORUM",
            Self::MaxActiveThreads => "MAX_ACTIVE_THREADS",
            Self::ThreadArchived => "THREAD_ARCHIVED",
            Self::ThreadLocked => "THREAD_LOCKED",
            Self::ThreadAlreadyCreatedForMessage => "THREAD_ALREADY_CREATED_FOR_MESSAGE",
            Self::CannotModifySystemWebhook => "CANNOT_MODIFY_SYSTEM_WEBHOOK",
            Self::MaxThreadMembers => "MAX_THREAD_MEMBERS",
            Self::InvalidThreadNotificationSettings => "INVALID_THREAD_NOTIFICATION_SETTINGS",
            Self::SlowmodeRateLimited => "SLOWMODE_RATE_LIMITED",
            Self::ResourceLocked => "RESOURCE_LOCKED",
            Self::InternalServerError => "INTERNAL_SERVER_ERROR",
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
    InvalidIsoTimestamp,
    CannotLeaveGuildAsOwner,
    ValueMustBeIntegerInRange,
    SystemChannelMustBeInGuild,
    SystemChannelMustBeText,
    AfkChannelMustBeInGuild,
    AfkChannelMustBeVoice,
    DiscoverableGuildVerificationLevelTooLow,
    NameEmptyAfterNormalization,
    InvalidParentChannel,
    ParentChannelNotInGuild,
    ParentMustBeCategory,
    CategoriesCannotHaveParents,
    InvalidUrlFormat,
    UrlLengthInvalid,
    ChannelNotFound,
    CannotPositionChannelRelativeToItself,
    PrecedingChannelMustShareParent,
    InvalidRoleId,
    CannotReorderEveryoneRole,
    RoleIconAndEmojiExclusive,
    RoleIconSourceAndIconExclusive,
    CannotSetIconOnEveryoneRole,
    NotAValidUnicodeEmoji,
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
            Self::InvalidIsoTimestamp => "INVALID_ISO_TIMESTAMP",
            Self::CannotLeaveGuildAsOwner => "CANNOT_LEAVE_GUILD_AS_OWNER",
            Self::ValueMustBeIntegerInRange => "VALUE_MUST_BE_INTEGER_IN_RANGE",
            Self::SystemChannelMustBeInGuild => "SYSTEM_CHANNEL_MUST_BE_IN_GUILD",
            Self::SystemChannelMustBeText => "SYSTEM_CHANNEL_MUST_BE_TEXT",
            Self::AfkChannelMustBeInGuild => "AFK_CHANNEL_MUST_BE_IN_GUILD",
            Self::AfkChannelMustBeVoice => "AFK_CHANNEL_MUST_BE_VOICE",
            Self::DiscoverableGuildVerificationLevelTooLow => {
                "DISCOVERABLE_GUILD_VERIFICATION_LEVEL_TOO_LOW"
            }
            Self::NameEmptyAfterNormalization => "NAME_EMPTY_AFTER_NORMALIZATION",
            Self::InvalidParentChannel => "INVALID_PARENT_CHANNEL",
            Self::ParentChannelNotInGuild => "PARENT_CHANNEL_NOT_IN_GUILD",
            Self::ParentMustBeCategory => "PARENT_MUST_BE_CATEGORY",
            Self::CategoriesCannotHaveParents => "CATEGORIES_CANNOT_HAVE_PARENTS",
            Self::InvalidUrlFormat => "INVALID_URL_FORMAT",
            Self::UrlLengthInvalid => "URL_LENGTH_INVALID",
            Self::ChannelNotFound => "CHANNEL_NOT_FOUND",
            Self::CannotPositionChannelRelativeToItself => {
                "CANNOT_POSITION_CHANNEL_RELATIVE_TO_ITSELF"
            }
            Self::PrecedingChannelMustShareParent => "PRECEDING_CHANNEL_MUST_SHARE_PARENT",
            Self::InvalidRoleId => "INVALID_ROLE_ID",
            Self::CannotReorderEveryoneRole => "CANNOT_REORDER_EVERYONE_ROLE",
            Self::RoleIconAndEmojiExclusive => "ROLE_ICON_AND_EMOJI_EXCLUSIVE",
            Self::RoleIconSourceAndIconExclusive => "ROLE_ICON_SOURCE_AND_ICON_EXCLUSIVE",
            Self::CannotSetIconOnEveryoneRole => "CANNOT_SET_ICON_ON_EVERYONE_ROLE",
            Self::NotAValidUnicodeEmoji => "NOT_A_VALID_UNICODE_EMOJI",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationError {
    pub path: String,
    pub code: &'static str,
    pub message: String,
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

    pub fn unknown_role() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownRole,
            "Unknown role",
        )
    }

    pub fn guild_template_invalid(message: impl Into<String>) -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::GuildTemplateInvalid,
            message,
        )
    }

    pub fn max_guild_roles() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxGuildRoles,
            "Maximum number of roles reached for this community.",
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

    pub fn cannot_modify_system_message(message: impl Into<String>) -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::CannotModifySystemWebhook,
            message,
        )
    }

    pub fn max_guild_channels() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxGuildChannels,
            "Maximum number of channels reached for this community.",
        )
    }

    pub fn max_category_channels() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxCategoryChannels,
            "Maximum number of channels reached for this category.",
        )
    }

    pub fn channel_type_conversion_not_supported() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ChannelTypeConversionNotSupported,
            "Channel type conversion is not supported.",
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

    pub fn not_found() -> Self {
        Self::coded(StatusCode::NOT_FOUND, ApiErrorCode::NotFound, "Not found")
    }

    pub fn missing_access() -> Self {
        Self::coded(
            StatusCode::FORBIDDEN,
            ApiErrorCode::MissingAccess,
            "Missing access",
        )
    }

    pub fn access_denied() -> Self {
        Self::coded(
            StatusCode::FORBIDDEN,
            ApiErrorCode::AccessDenied,
            "Access denied",
        )
    }

    pub fn unknown_forum_tag() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownForumTag,
            "Unknown forum tag",
        )
    }

    pub fn unknown_emoji() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownEmoji,
            "Unknown emoji",
        )
    }

    pub fn unknown_thread_member() -> Self {
        Self::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownThreadMember,
            "Unknown thread member",
        )
    }

    pub fn max_forum_tags() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxForumTags,
            "Maximum number of forum tags reached for this channel.",
        )
    }

    pub fn forum_tag_names_must_be_unique() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ForumTagNamesMustBeUnique,
            "Forum tag names must be unique.",
        )
    }

    pub fn no_tags_available_to_non_moderators() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::NoTagsAvailableToNonModerators,
            "No unmoderated tags would remain available.",
        )
    }

    pub fn hide_media_download_option_media_only() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::HideMediaDownloadOptionMediaOnly,
            "HIDE_MEDIA_DOWNLOAD_OPTIONS can only be set on media channels.",
        )
    }

    pub fn forum_tag_required() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ForumTagRequired,
            "A tag is required to post in this channel.",
        )
    }

    pub fn max_pinned_threads_in_forum() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxPinnedThreadsInForum,
            "Maximum number of pinned posts reached for this channel.",
        )
    }

    pub fn max_active_threads() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxActiveThreads,
            "Maximum number of active threads reached for this community.",
        )
    }

    pub fn thread_archived() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ThreadArchived,
            "This thread is archived.",
        )
    }

    pub fn thread_locked() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ThreadLocked,
            "This thread is locked.",
        )
    }

    pub fn thread_already_created_for_message() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ThreadAlreadyCreatedForMessage,
            "A thread has already been created for this message.",
        )
    }

    pub fn max_thread_members() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::MaxThreadMembers,
            "Maximum number of members reached for this thread.",
        )
    }

    pub fn invalid_thread_notification_settings() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::InvalidThreadNotificationSettings,
            "Invalid thread notification settings.",
        )
    }

    pub fn slowmode_rate_limited() -> Self {
        Self::coded(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::SlowmodeRateLimited,
            "You are sending messages or creating posts too quickly.",
        )
    }

    pub fn resource_locked() -> Self {
        Self::coded(
            StatusCode::TOO_MANY_REQUESTS,
            ApiErrorCode::ResourceLocked,
            "Another tag change is currently running.",
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

        (
            status,
            Json(FluxerErrorBody {
                code: code.as_str(),
                message: &message,
                errors: &errors,
            }),
        )
            .into_response()
    }
}
