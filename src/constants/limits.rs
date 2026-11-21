pub const MAX_USERNAME_LENGTH: usize = 32;
pub const MAX_GLOBAL_NAME_LENGTH: usize = 32;
pub const MAX_EMAIL_LENGTH: usize = 254;
pub const MAX_PASSWORD_LENGTH: usize = 256;
pub const MAX_BIO_LENGTH: usize = 320;
pub const MAX_PRONOUNS_LENGTH: usize = 40;
pub const MAX_ASSET_HASH_LENGTH: usize = 255;
pub const MAX_COLOR_VALUE: u32 = 0x00FF_FFFF;

pub const MIN_AFK_TIMEOUT: u32 = 60;
pub const MAX_LOCALE_LENGTH: usize = 16;
pub const MAX_GRADIENT_PRESET_LENGTH: usize = 32;
pub const MAX_SYNCED_PREFS_BYTES: usize = 16_384;
pub const MAX_EXTRA_SETTINGS_BYTES: usize = 60_000;
pub const MAX_CUSTOM_STATUS_TEXT_LENGTH: usize = 128;
pub const MAX_EMOJI_NAME_LENGTH: usize = 64;
pub const MAX_EXPIRES_AT_LENGTH: usize = 64;
pub const MAX_SETTINGS_LIST_ITEMS: usize = 200;
pub const MAX_GUILD_FOLDERS: usize = 100;
pub const MAX_FOLDER_NAME_LENGTH: usize = 100;
pub const MAX_FOLDER_ICON_LENGTH: usize = 64;

pub const MAX_GUILD_NAME_LENGTH: usize = 100;
pub const MAX_GUILDS_PER_USER: i64 = 100;
pub const MAX_GUILDS_PAGE_LIMIT: u32 = 200;
pub const GUILD_LIST_PERMISSION_LIMIT: u32 = 100;
pub const DEFAULT_MEMBERS_PAGE_LIMIT: u32 = 100;
pub const MAX_MEMBERS_PAGE_LIMIT: u32 = 1000;
pub const MAX_CHANNELS_PER_GUILD: u64 = 500;
pub const MAX_LAZY_SUBSCRIBED_MEMBERS: usize = 1000;

pub const DEFAULT_DISCOVERY_LIMIT: u64 = 24;
pub const MAX_DISCOVERY_LIMIT: u64 = 48;

pub const MAX_MESSAGE_LENGTH: usize = 2000;
pub const DEFAULT_MESSAGES_PAGE_LIMIT: u32 = 50;
pub const MAX_MESSAGES_PAGE_LIMIT: u32 = 100;

pub const MAX_INVITES_PER_GUILD: u64 = 1000;
pub const MAX_INVITE_USES: u32 = 100;
pub const MAX_INVITE_AGE_SECONDS: u32 = 604_800;

pub const MAX_ATTACHMENT_SIZE_BYTES: u64 = 26_214_400;