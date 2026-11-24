use crate::constants::limits::{MAX_ASSET_HASH_LENGTH, MAX_COLOR_VALUE, MAX_EMAIL_LENGTH};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::ChannelType;

pub fn validate_asset_opt(field: &str, value: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.len() > MAX_ASSET_HASH_LENGTH
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::validation(
            field,
            ValidationCode::InvalidFormat,
            format!("Invalid {field} identifier."),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

pub fn validate_color_opt(field: &str, color: Option<u32>) -> Result<(), AppError> {
    if let Some(c) = color
        && c > MAX_COLOR_VALUE
    {
        return Err(AppError::validation(
            field,
            ValidationCode::ColorValueTooHigh,
            "Color value must not exceed 0xffffff.",
        ));
    }
    Ok(())
}

pub fn reject_control_chars(field: &str, value: &str) -> Result<(), AppError> {
    if value.chars().any(char::is_control) {
        return Err(AppError::validation(
            field,
            ValidationCode::InvalidFormat,
            format!("{field} cannot contain control characters."),
        ));
    }
    Ok(())
}

pub fn is_valid_email(raw: &str) -> bool {
    if raw.is_empty()
        || raw.len() > MAX_EMAIL_LENGTH
        || raw.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    let Some((local, domain)) = raw.split_once('@') else {
        return false;
    };
    if local.is_empty() || local.len() > 64 || domain.is_empty() || domain.contains('@') {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    domain.split('.').all(|label| !label.is_empty())
}

pub fn normalize_optional_text(
    field: &str,
    input: Option<&str>,
    max_chars: usize,
) -> Result<Option<String>, AppError> {
    let value = input.map(str::trim).filter(|s| !s.is_empty());
    if let Some(text) = value
        && text.chars().count() > max_chars
    {
        return Err(AppError::validation(
            field,
            ValidationCode::ContentExceedsMaxLength,
            format!("Text must not exceed {max_chars} characters."),
        ));
    }
    Ok(value.map(str::to_string))
}

pub fn collapse_channel_whitespace(raw: &str) -> String {
    let mut collapsed = String::with_capacity(raw.len());
    let mut prev_space = true;
    for c in raw.chars() {
        if c.is_control() {
            continue;
        }
        if c.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
                prev_space = true;
            }
        } else {
            collapsed.push(c);
            prev_space = false;
        }
    }
    collapsed
}

pub fn slugify_channel_name(trimmed: &str, kind: u32) -> String {
    if kind == ChannelType::GuildText as u32 || kind == ChannelType::GuildAnnouncement as u32 {
        trimmed
            .to_lowercase()
            .chars()
            .filter_map(|c| {
                if c == ' ' {
                    Some('-')
                } else if !c.is_ascii_punctuation() || c == '-' || c == '.' || c == '_' {
                    Some(c)
                } else {
                    None
                }
            })
            .collect()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_assets_colors_and_control_chars() {
        assert_eq!(
            validate_asset_opt("avatar", Some("  abc_123-XYZ ")).unwrap(),
            Some("abc_123-XYZ".to_string())
        );
        assert_eq!(validate_asset_opt("avatar", Some("   ")).unwrap(), None);
        assert!(validate_asset_opt("avatar", Some("../etc/passwd")).is_err());

        assert!(validate_color_opt("accent_color", Some(MAX_COLOR_VALUE)).is_ok());
        assert!(validate_color_opt("accent_color", Some(MAX_COLOR_VALUE + 1)).is_err());

        assert!(reject_control_chars("name", "Alice").is_ok());
        assert!(reject_control_chars("name", "Alice\nBob").is_err());
        assert!(reject_control_chars("name", "Alice\0").is_err());
    }

    #[test]
    fn validates_email_format() {
        assert!(is_valid_email("alice@example.com"));
        assert!(is_valid_email("a+b@sub.example.co"));
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("alice@"));
        assert!(!is_valid_email("alice@localhost"));
        assert!(!is_valid_email("alice@.com"));
        assert!(!is_valid_email("alice@a..b"));
        assert!(!is_valid_email("alice b@example.com"));
        assert!(!is_valid_email("alice@b\n.com"));
        assert!(!is_valid_email("a@b@c.com"));
    }

    #[test]
    fn normalizes_optional_text_by_char_count() {
        assert_eq!(
            normalize_optional_text("bio", Some("   "), 10).unwrap(),
            None
        );
        assert_eq!(
            normalize_optional_text("bio", Some("  ééé  "), 3).unwrap(),
            Some("ééé".to_string())
        );
        assert!(normalize_optional_text("bio", Some("éééé"), 3).is_err());
    }
}
