pub const DEFAULT_AFK_TIMEOUT: u32 = 600;
pub const DEFAULT_DISCRIMINATOR: &str = "0000";
pub const DEFAULT_LOCALE: &str = "en-US";
pub const DEFAULT_THEME: &str = "dark";
pub const DEFAULT_STATUS: &str = "online";
pub const VALID_THEMES: [&str; 5] = ["dark", "dark_legacy", "coal", "light", "system"];
pub const VALID_STATUSES: [&str; 4] = ["online", "idle", "dnd", "invisible"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceStatus {
    Online,
    Idle,
    Dnd,
    Invisible,
    Offline,
}

impl PresenceStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "online" => Some(Self::Online),
            "idle" => Some(Self::Idle),
            "dnd" => Some(Self::Dnd),
            "invisible" => Some(Self::Invisible),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Idle => "idle",
            Self::Dnd => "dnd",
            Self::Invisible => "invisible",
            Self::Offline => "offline",
        }
    }
}