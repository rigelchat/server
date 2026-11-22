use serde::Serialize;

#[derive(Serialize, Clone, Copy)]
#[serde(into = "u8")]
pub enum VerificationLevel {
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    VeryHigh = 4,
}

impl From<u8> for VerificationLevel {
    fn from(level: u8) -> Self {
        match level {
            1 => Self::Low,
            2 => Self::Medium,
            3 => Self::High,
            4 => Self::VeryHigh,
            _ => Self::None,
        }
    }
}

impl From<VerificationLevel> for u8 {
    fn from(level: VerificationLevel) -> u8 {
        level as u8
    }
}

#[derive(Serialize, Clone, Copy)]
#[serde(into = "u8")]
pub enum DiscoveryCategory {
    Gaming = 0,
    Music = 1,
    Entertainment = 2,
    Education = 3,
    ScienceAndTechnology = 4,
    ContentCreator = 5,
    AnimeAndManga = 6,
    MoviesAndTV = 7,
    Other = 8,
}

impl DiscoveryCategory {
    pub const ALL: [Self; 9] = [
        Self::Gaming,
        Self::Music,
        Self::Entertainment,
        Self::Education,
        Self::ScienceAndTechnology,
        Self::ContentCreator,
        Self::AnimeAndManga,
        Self::MoviesAndTV,
        Self::Other,
    ];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Gaming => "Gaming",
            Self::Music => "Music",
            Self::Entertainment => "Entertainment",
            Self::Education => "Education",
            Self::ScienceAndTechnology => "Science & Technology",
            Self::ContentCreator => "Content Creator",
            Self::AnimeAndManga => "Anime & Manga",
            Self::MoviesAndTV => "Movies & TV",
            Self::Other => "Other",
        }
    }
}

impl From<u8> for DiscoveryCategory {
    fn from(category: u8) -> Self {
        match category {
            0 => Self::Gaming,
            1 => Self::Music,
            2 => Self::Entertainment,
            3 => Self::Education,
            4 => Self::ScienceAndTechnology,
            5 => Self::ContentCreator,
            6 => Self::AnimeAndManga,
            7 => Self::MoviesAndTV,
            _ => Self::Other,
        }
    }
}

impl From<DiscoveryCategory> for u8 {
    fn from(category: DiscoveryCategory) -> u8 {
        category as u8
    }
}

pub const DEFAULT_VOICE_BITRATE: u32 = 64_000;
pub const DEFAULT_VOICE_CONNECTION_LIMIT: u32 = 5;
pub const DEFAULT_PRIMARY_LANGUAGE: &str = "en-US";
pub const FEATURE_DISCOVERABLE: &str = "DISCOVERABLE";
pub const MAX_GUILD_DESCRIPTION_LENGTH: usize = 300;
pub const MIN_AFK_TIMEOUT: u32 = 60;
pub const MAX_AFK_TIMEOUT: u32 = 3600;
pub const MAX_VERIFICATION_LEVEL: u8 = 3;
pub const MAX_EXPLICIT_CONTENT_FILTER: u8 = 2;
pub const MAX_DEFAULT_MESSAGE_NOTIFICATIONS: u8 = 1;
pub const SYSTEM_CHANNEL_FLAGS_MASK: u32 = 1 << 0;
pub const MIN_VOICE_BITRATE: u32 = 8_000;
pub const MAX_VOICE_BITRATE_STANDARD: u32 = 96_000;
pub const MAX_VOICE_BITRATE_CEILING: u32 = 384_000;
pub const MAX_VOICE_USER_LIMIT: u32 = 99;
pub const MIN_VOICE_CONNECTION_LIMIT: u32 = 1;
pub const MAX_VOICE_CONNECTION_LIMIT: u32 = 100;