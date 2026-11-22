use chrono::{DateTime, SecondsFormat};
use serde::{Deserialize, Deserializer, Serializer};

pub fn millis_to_iso(ms: u64) -> String {
    DateTime::from_timestamp_millis(ms as i64)
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_default()
}

pub fn serialize_millis_iso<S: Serializer>(ms: &u64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&millis_to_iso(*ms))
}

pub fn serialize_millis_iso_opt<S: Serializer>(ms: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
    match ms {
        Some(ms) => s.serialize_str(&millis_to_iso(*ms)),
        None => s.serialize_none(),
    }
}

pub fn serialize_u64_as_string<S: Serializer>(val: &u64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&val.to_string())
}

/// Used with `#[serde(default)]` on PATCH payloads to distinguish omitted fields (`None`)
/// from explicit `null` (`Some(None)`).
pub fn deserialize_double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
pub fn deserialize_string_or_int<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt {
        Str(String),
        Int(i64),
        Uint(u64),
    }

    match StringOrInt::deserialize(deserializer)? {
        StringOrInt::Str(s) => Ok(s),
        StringOrInt::Int(i) => Ok(i.to_string()),
        StringOrInt::Uint(u) => Ok(u.to_string()),
    }
}

pub fn deserialize_template_overwrite_type<'de, D>(
    deserializer: D,
) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum TypeOrStr {
        Int(u32),
        Str(String),
    }

    match Option::<TypeOrStr>::deserialize(deserializer)? {
        Some(TypeOrStr::Int(v)) => Ok(Some(v)),
        Some(TypeOrStr::Str(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "0" | "role" => Ok(Some(0)),
            "1" | "member" => Ok(Some(1)),
            _ => Ok(None),
        },
        None => Ok(None),
    }
}

pub fn deserialize_opt_u64_string_or_int<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrUint {
        Str(String),
        Uint(u64),
    }

    match Option::<StringOrUint>::deserialize(deserializer)? {
        Some(StringOrUint::Str(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(Some(0));
            }
            trimmed
                .parse::<u64>()
                .map(Some)
                .map_err(|_| serde::de::Error::custom("invalid integer string"))
        }
        Some(StringOrUint::Uint(u)) => Ok(Some(u)),
        None => Ok(None),
    }
}

pub fn deserialize_opt_string_or_int<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt {
        Str(String),
        Int(i64),
        Uint(u64),
    }

    match Option::<StringOrInt>::deserialize(deserializer)? {
        Some(StringOrInt::Str(s)) => Ok(Some(s)),
        Some(StringOrInt::Int(i)) => Ok(Some(i.to_string())),
        Some(StringOrInt::Uint(u)) => Ok(Some(u.to_string())),
        None => Ok(None),
    }
}

pub fn parse_timestamp_iso(
    field: &str,
    value: serde_json::Value,
) -> Result<Option<String>, crate::error::AppError> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(raw) => {
            let s = raw.trim().to_string();
            if s.is_empty() {
                Ok(None)
            } else {
                if s.len() > crate::constants::limits::MAX_EXPIRES_AT_LENGTH
                    || chrono::DateTime::parse_from_rfc3339(&s).is_err()
                {
                    return Err(crate::error::AppError::validation(
                        field,
                        crate::error::ValidationCode::InvalidFormat,
                        format!("Invalid {field} timestamp."),
                    ));
                }
                Ok(Some(s))
            }
        }
        serde_json::Value::Number(n) => {
            let ts = n
                .as_i64()
                .and_then(chrono::DateTime::from_timestamp_millis)
                .ok_or_else(|| {
                    crate::error::AppError::validation(
                        field,
                        crate::error::ValidationCode::InvalidFormat,
                        format!("Invalid {field} timestamp."),
                    )
                })?;
            Ok(Some(
                ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            ))
        }
        _ => Err(crate::error::AppError::validation(
            field,
            crate::error::ValidationCode::InvalidFormat,
            format!("Invalid {field} timestamp."),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq)]
    struct DoubleOpt {
        #[serde(default, deserialize_with = "deserialize_double_option")]
        val: Option<Option<String>>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct NonceBox {
        #[serde(default, deserialize_with = "deserialize_opt_string_or_int")]
        nonce: Option<String>,
    }

    #[test]
    fn double_option_distinguishes_omitted_and_null() {
        let omitted: DoubleOpt = serde_json::from_str("{}").unwrap();
        let nulled: DoubleOpt = serde_json::from_str(r#"{"val":null}"#).unwrap();
        let set: DoubleOpt = serde_json::from_str(r#"{"val":"hi"}"#).unwrap();
        assert_eq!(omitted.val, None);
        assert_eq!(nulled.val, Some(None));
        assert_eq!(set.val, Some(Some("hi".to_string())));
    }

    #[test]
    fn string_or_int_parses_all_variants() {
        assert_eq!(
            serde_json::from_str::<NonceBox>(r#"{"nonce":12345}"#)
                .unwrap()
                .nonce
                .as_deref(),
            Some("12345")
        );
        assert_eq!(
            serde_json::from_str::<NonceBox>(r#"{"nonce":"abc"}"#)
                .unwrap()
                .nonce
                .as_deref(),
            Some("abc")
        );
        assert_eq!(
            serde_json::from_str::<NonceBox>(r#"{}"#).unwrap().nonce,
            None
        );
    }

    #[derive(Deserialize)]
    struct PermBox {
        #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
        allow: Option<u64>,
    }

    #[test]
    fn u64_string_or_int_parses_all_variants() {
        assert_eq!(serde_json::from_str::<PermBox>(r#"{"allow":"1024"}"#).unwrap().allow, Some(1024));
        assert_eq!(serde_json::from_str::<PermBox>(r#"{"allow":2048}"#).unwrap().allow, Some(2048));
        assert_eq!(serde_json::from_str::<PermBox>(r#"{"allow":null}"#).unwrap().allow, None);
        assert!(serde_json::from_str::<PermBox>(r#"{"allow":"-5"}"#).is_err());
    }

    #[test]
    fn parse_timestamp_iso_validates_inputs() {
        assert_eq!(
            parse_timestamp_iso("exp", serde_json::Value::Null).unwrap(),
            None
        );
        assert_eq!(
            parse_timestamp_iso("exp", serde_json::json!("  ")).unwrap(),
            None
        );
        assert_eq!(
            parse_timestamp_iso("exp", serde_json::json!("2026-01-01T00:00:00Z")).unwrap(),
            Some("2026-01-01T00:00:00Z".to_string())
        );
        assert!(
            parse_timestamp_iso("exp", serde_json::json!(1_700_000_000_000i64))
                .unwrap()
                .is_some()
        );
        assert!(parse_timestamp_iso("exp", serde_json::json!("not-a-date")).is_err());
        assert!(parse_timestamp_iso("exp", serde_json::json!(true)).is_err());
    }
}
