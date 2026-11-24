use std::collections::{HashMap, HashSet};

use crate::constants::flags::PermissionFlags;
use crate::constants::guild::{
    DEFAULT_GUILD_AFK_TIMEOUT, DEFAULT_VOICE_BITRATE, DEFAULT_VOICE_CONNECTION_LIMIT,
    SYSTEM_CHANNEL_FLAGS_MASK,
};
use crate::constants::limits::{
    MAX_AFK_TIMEOUT, MAX_CHANNEL_NAME_LENGTH, MAX_CHANNEL_OVERWRITES,
    MAX_CHANNEL_RATE_LIMIT_PER_USER, MAX_CHANNEL_TOPIC_LENGTH, MAX_CHANNELS_PER_GUILD,
    MAX_COLOR_VALUE, MAX_DEFAULT_MESSAGE_NOTIFICATIONS, MAX_EXPLICIT_CONTENT_FILTER,
    MAX_GUILD_ROLES, MAX_ROLE_NAME_LENGTH, MAX_ROLE_UNICODE_EMOJI_LENGTH, MAX_VERIFICATION_LEVEL,
    MAX_VOICE_BITRATE_STANDARD, MAX_VOICE_CONNECTION_LIMIT, MAX_VOICE_USER_LIMIT, MIN_AFK_TIMEOUT,
    MIN_VOICE_BITRATE, MIN_VOICE_CONNECTION_LIMIT,
};
use crate::error::AppError;
use crate::models::channel::ChannelType;
use crate::models::permission::OverwriteType;
use crate::models::rest::guilds::GuildCreationTemplate;
use crate::utils::snowflake::DISCORD_SNOWFLAKE;
use crate::utils::validation::{collapse_channel_whitespace, slugify_channel_name};

pub(super) struct PreparedTemplateRole {
    pub(super) id: String,
    pub(super) position: u32,
    pub(super) name: String,
    pub(super) color: u32,
    pub(super) unicode_emoji: Option<String>,
    pub(super) hoist: bool,
    pub(super) mentionable: bool,
    pub(super) permissions: u64,
}

pub(super) struct PreparedTemplateOverwrite {
    pub(super) target_id: String,
    pub(super) allow: u64,
    pub(super) deny: u64,
}

pub(super) struct PreparedTemplateChannel {
    pub(super) id: String,
    pub(super) kind: u32,
    pub(super) position: u32,
    pub(super) parent_id: Option<String>,
    pub(super) name: String,
    pub(super) topic: Option<String>,
    pub(super) nsfw: bool,
    pub(super) rate_limit_per_user: u32,
    pub(super) bitrate: Option<u32>,
    pub(super) user_limit: Option<u32>,
    pub(super) voice_connection_limit: Option<u32>,
    pub(super) overwrites: Vec<PreparedTemplateOverwrite>,
}

pub(super) struct PreparedTemplate {
    pub(super) verification_level: u8,
    pub(super) default_message_notifications: u8,
    pub(super) explicit_content_filter: u8,
    pub(super) afk_timeout: u32,
    pub(super) system_channel_flags: u32,
    pub(super) system_channel_id: String,
    pub(super) everyone_permissions: u64,
    pub(super) everyone_color: u32,
    pub(super) roles: Vec<PreparedTemplateRole>,
    pub(super) channels: Vec<PreparedTemplateChannel>,
}

pub(super) fn prepare_template(
    guild_id: &str,
    tpl: GuildCreationTemplate,
) -> Result<PreparedTemplate, AppError> {
    let raw_roles = tpl.roles.unwrap_or_default();
    let raw_channels = tpl.channels.unwrap_or_default();

    if raw_roles.len() > MAX_GUILD_ROLES as usize {
        return Err(AppError::guild_template_invalid(
            "Guild template exceeds maximum number of roles.",
        ));
    }
    if raw_channels.len() > MAX_CHANNELS_PER_GUILD as usize {
        return Err(AppError::guild_template_invalid(
            "Guild template exceeds maximum number of channels.",
        ));
    }

    let mut seen_role_ids = HashSet::with_capacity(raw_roles.len());
    for role in &raw_roles {
        let tid = role.id.trim();
        if tid.is_empty() || !seen_role_ids.insert(tid.to_string()) {
            return Err(AppError::guild_template_invalid(
                "Template role IDs must be non-empty and unique.",
            ));
        }
    }

    let mut seen_channel_ids = HashSet::with_capacity(raw_channels.len());
    for ch in &raw_channels {
        let tid = ch.id.trim();
        if tid.is_empty() || !seen_channel_ids.insert(tid.to_string()) {
            return Err(AppError::guild_template_invalid(
                "Template channel IDs must be non-empty and unique.",
            ));
        }
    }

    let everyone_idx = raw_roles
        .iter()
        .position(|r| r.name.as_deref().map(str::trim) == Some("@everyone"))
        .or_else(|| raw_roles.iter().position(|r| r.id.trim() == "0"));

    let mut everyone_permissions = PermissionFlags::DEFAULT_EVERYONE.bits();
    let mut everyone_color = 0u32;
    let mut role_id_map: HashMap<String, String> = HashMap::with_capacity(raw_roles.len() + 1);
    role_id_map.insert("0".to_string(), guild_id.to_string());

    if let Some(idx) = everyone_idx {
        let ev = &raw_roles[idx];
        if let Some(bits) = ev.permissions_new.or(ev.permissions) {
            everyone_permissions = PermissionFlags::from_bits_truncate(bits).bits();
        }
        everyone_color = ev.color.unwrap_or(0).min(MAX_COLOR_VALUE);
        role_id_map.insert(ev.id.trim().to_string(), guild_id.to_string());
    }

    let mut roles = Vec::with_capacity(raw_roles.len());
    let mut next_role_pos = 1u32;
    for (idx, r) in raw_roles.into_iter().enumerate() {
        if Some(idx) == everyone_idx {
            continue;
        }
        let new_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        role_id_map.insert(r.id.trim().to_string(), new_id.clone());

        let name: String = r
            .name
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .chars()
            .take(MAX_ROLE_NAME_LENGTH)
            .collect();
        let permissions = r
            .permissions_new
            .or(r.permissions)
            .map(|bits| PermissionFlags::from_bits_truncate(bits).bits())
            .unwrap_or(0);

        roles.push(PreparedTemplateRole {
            id: new_id,
            position: next_role_pos,
            name,
            color: r.color.unwrap_or(0).min(MAX_COLOR_VALUE),
            unicode_emoji: sanitize_template_unicode_emoji(r.unicode_emoji.as_deref()),
            hoist: r.hoist.unwrap_or(false),
            mentionable: r.mentionable.unwrap_or(false),
            permissions,
        });
        next_role_pos += 1;
    }

    let mut category_id_map: HashMap<String, String> = HashMap::new();
    let mut kept_channels = Vec::with_capacity(raw_channels.len());
    for ch in raw_channels {
        let mapped_kind = match ch.kind {
            0 => ChannelType::GuildText as u32,
            2 | 13 => ChannelType::GuildVoice as u32,
            4 => ChannelType::GuildCategory as u32,
            5 => ChannelType::GuildAnnouncement as u32,
            _ => continue,
        };
        let new_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        if mapped_kind == ChannelType::GuildCategory as u32 {
            category_id_map.insert(ch.id.trim().to_string(), new_id.clone());
        }
        kept_channels.push((new_id, mapped_kind, ch));
    }

    let mut categories = Vec::new();
    let mut children = Vec::new();
    let mut template_ch_id_map: HashMap<String, (String, u32)> = HashMap::new();
    let mut first_text_channel: Option<(u32, usize, String)> = None;

    for (order_idx, (new_id, mapped_kind, ch)) in kept_channels.into_iter().enumerate() {
        let raw_overwrites = ch.permission_overwrites.unwrap_or_default();
        if raw_overwrites.len() > MAX_CHANNEL_OVERWRITES {
            return Err(AppError::guild_template_invalid(
                "Channel template exceeds maximum number of permission overwrites.",
            ));
        }

        let tpl_id = ch.id.trim().to_string();
        let position = ch.position.unwrap_or(0).clamp(0, u32::MAX as i64) as u32;
        let parent_id = if mapped_kind == ChannelType::GuildCategory as u32 {
            None
        } else {
            ch.parent_id
                .as_deref()
                .map(str::trim)
                .and_then(|pid| category_id_map.get(pid).cloned())
        };

        let name = normalize_template_channel_name(ch.name.as_deref(), mapped_kind);
        let topic = ch
            .topic
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(MAX_CHANNEL_TOPIC_LENGTH).collect());
        let is_voice = mapped_kind == ChannelType::GuildVoice as u32;
        let bitrate = is_voice.then(|| {
            ch.bitrate
                .unwrap_or(DEFAULT_VOICE_BITRATE)
                .clamp(MIN_VOICE_BITRATE, MAX_VOICE_BITRATE_STANDARD)
        });
        let user_limit = is_voice.then(|| ch.user_limit.unwrap_or(0).min(MAX_VOICE_USER_LIMIT));
        let voice_connection_limit = is_voice.then(|| {
            ch.voice_connection_limit
                .unwrap_or(DEFAULT_VOICE_CONNECTION_LIMIT)
                .clamp(MIN_VOICE_CONNECTION_LIMIT, MAX_VOICE_CONNECTION_LIMIT)
        });
        let rate_limit_per_user = ch
            .rate_limit_per_user
            .unwrap_or(0)
            .min(MAX_CHANNEL_RATE_LIMIT_PER_USER);

        let mut overwrites: Vec<PreparedTemplateOverwrite> = Vec::new();
        let mut ow_index: HashMap<String, usize> = HashMap::new();
        for ow in raw_overwrites {
            if ow.kind != Some(OverwriteType::Role as u32) {
                continue;
            }
            let Some(target_id) = role_id_map.get(ow.id.trim()).cloned() else {
                continue;
            };
            let prepared_ow = PreparedTemplateOverwrite {
                target_id: target_id.clone(),
                allow: PermissionFlags::from_bits_truncate(ow.allow.unwrap_or(0)).bits(),
                deny: PermissionFlags::from_bits_truncate(ow.deny.unwrap_or(0)).bits(),
            };
            if let Some(&existing_idx) = ow_index.get(&target_id) {
                overwrites[existing_idx] = prepared_ow;
            } else {
                ow_index.insert(target_id, overwrites.len());
                overwrites.push(prepared_ow);
            }
        }

        if mapped_kind == ChannelType::GuildText as u32
            || mapped_kind == ChannelType::GuildAnnouncement as u32
        {
            template_ch_id_map.insert(tpl_id, (new_id.clone(), mapped_kind));
            let replace_first = match &first_text_channel {
                None => true,
                Some((best_pos, best_ord, _)) => (position, order_idx) < (*best_pos, *best_ord),
            };
            if replace_first {
                first_text_channel = Some((position, order_idx, new_id.clone()));
            }
        }

        let prepared_ch = PreparedTemplateChannel {
            id: new_id,
            kind: mapped_kind,
            position,
            parent_id,
            name,
            topic,
            nsfw: ch.nsfw.unwrap_or(false),
            rate_limit_per_user,
            bitrate,
            user_limit,
            voice_connection_limit,
            overwrites,
        };

        if mapped_kind == ChannelType::GuildCategory as u32 {
            categories.push(prepared_ch);
        } else {
            children.push(prepared_ch);
        }
    }

    let mut channels = categories;
    channels.extend(children);

    let requested_sys = tpl
        .system_channel_id
        .as_deref()
        .map(str::trim)
        .and_then(|tid| template_ch_id_map.get(tid))
        .map(|(new_id, _)| new_id.clone());

    let system_channel_id = if let Some(id) = requested_sys {
        id
    } else if let Some((_, _, first_id)) = first_text_channel {
        first_id
    } else {
        if channels.len() >= MAX_CHANNELS_PER_GUILD as usize {
            return Err(AppError::guild_template_invalid(
                "Guild template exceeds maximum number of channels.",
            ));
        }
        let fallback_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        channels.push(PreparedTemplateChannel {
            id: fallback_id.clone(),
            kind: ChannelType::GuildText as u32,
            position: 0,
            parent_id: None,
            name: "general".to_string(),
            topic: None,
            nsfw: false,
            rate_limit_per_user: 0,
            bitrate: None,
            user_limit: None,
            voice_connection_limit: None,
            overwrites: Vec::new(),
        });
        fallback_id
    };

    Ok(PreparedTemplate {
        verification_level: tpl
            .verification_level
            .unwrap_or(0)
            .clamp(0, MAX_VERIFICATION_LEVEL as i64) as u8,
        default_message_notifications: tpl
            .default_message_notifications
            .unwrap_or(0)
            .clamp(0, MAX_DEFAULT_MESSAGE_NOTIFICATIONS as i64)
            as u8,
        explicit_content_filter: tpl
            .explicit_content_filter
            .unwrap_or(0)
            .clamp(0, MAX_EXPLICIT_CONTENT_FILTER as i64) as u8,
        afk_timeout: tpl
            .afk_timeout
            .unwrap_or(DEFAULT_GUILD_AFK_TIMEOUT as i64)
            .clamp(MIN_AFK_TIMEOUT as i64, MAX_AFK_TIMEOUT as i64) as u32,
        system_channel_flags: tpl.system_channel_flags.unwrap_or(0) & SYSTEM_CHANNEL_FLAGS_MASK,
        system_channel_id,
        everyone_permissions,
        everyone_color,
        roles,
        channels,
    })
}

fn sanitize_template_unicode_emoji(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_ROLE_UNICODE_EMOJI_LENGTH
        || trimmed
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn normalize_template_channel_name(raw: Option<&str>, kind: u32) -> String {
    let collapsed = collapse_channel_whitespace(raw.unwrap_or(""));
    let trimmed = collapsed.trim();
    let slug = slugify_channel_name(trimmed, kind);
    let normalized = if slug.is_empty() {
        trimmed.to_string()
    } else {
        slug
    };

    if normalized.is_empty() {
        "channel".to_string()
    } else {
        normalized.chars().take(MAX_CHANNEL_NAME_LENGTH).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::rest::guilds::CreateGuildRequest;

    #[test]
    fn prepare_template_maps_roles_categories_overwrites_and_fallbacks() {
        let json = r#"{
            "name": "Ignored",
            "verification_level": 9,
            "afk_timeout": 10,
            "system_channel_id": "99",
            "roles": [
                { "id": 0, "name": "@everyone", "permissions": "1024", "permissions_new": "2048" },
                { "id": 10, "name": "Mod", "color": 255, "hoist": true, "unicode_emoji": "🛡️" },
                { "id": "11", "name": "BadEmoji", "unicode_emoji": "not_emoji" }
            ],
            "channels": [
                { "id": 20, "type": 0, "name": "Welcome Chat!", "position": 0, "parent_id": 30,
                  "permission_overwrites": [
                    { "id": 0, "type": "role", "allow": "0", "deny": "1024" },
                    { "id": 10, "type": 0, "allow": "1024", "deny": "0" },
                    { "id": 999, "type": 0, "allow": "1024", "deny": "0" },
                    { "id": 10, "type": 1, "allow": "8", "deny": "0" }
                  ]
                },
                { "id": 30, "type": 4, "name": "Info", "position": 0 },
                { "id": 40, "type": 13, "name": "Stage", "position": 1, "bitrate": 384000 },
                { "id": 50, "type": 15, "name": "ForumSkipped", "position": 2 }
            ]
        }"#;

        let req: CreateGuildRequest =
            serde_json::from_str(&format!(r#"{{"name":"My Guild","template":{json}}}"#)).unwrap();
        let prepared = prepare_template("1000", req.template.unwrap()).unwrap();

        assert_eq!(prepared.verification_level, 3);
        assert_eq!(prepared.afk_timeout, 60);
        assert_eq!(prepared.everyone_permissions, 2048);
        assert_eq!(prepared.roles.len(), 2);
        assert_eq!(prepared.roles[0].name, "Mod");
        assert_eq!(prepared.roles[0].position, 1);
        assert_eq!(prepared.roles[0].unicode_emoji.as_deref(), Some("🛡️"));
        assert_eq!(prepared.roles[1].unicode_emoji, None);

        assert_eq!(prepared.channels.len(), 3);
        assert_eq!(prepared.channels[0].kind, ChannelType::GuildCategory as u32);
        assert_eq!(prepared.channels[1].name, "welcome-chat");
        assert_eq!(
            prepared.channels[1].parent_id.as_deref(),
            Some(prepared.channels[0].id.as_str())
        );
        assert_eq!(prepared.channels[1].overwrites.len(), 2);
        assert_eq!(prepared.channels[1].overwrites[0].target_id, "1000");
        assert_eq!(prepared.channels[2].kind, ChannelType::GuildVoice as u32);
        assert_eq!(
            prepared.channels[2].bitrate,
            Some(MAX_VOICE_BITRATE_STANDARD)
        );
        assert_eq!(prepared.system_channel_id, prepared.channels[1].id);
    }
}
