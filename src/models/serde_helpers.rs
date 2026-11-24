use std::fmt;

use chrono::{DateTime, SecondsFormat};
use serde::{
    Deserialize, Deserializer, Serializer,
    de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor},
};

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

pub fn deserialize_template_overwrite_type<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimestampInput {
    Empty,
    Iso(String),
    Invalid,
}

impl TimestampInput {
    pub fn into_opt_iso(self, field: &str) -> Result<Option<String>, crate::error::AppError> {
        match self {
            Self::Empty => Ok(None),
            Self::Iso(s) => Ok(Some(s)),
            Self::Invalid => Err(crate::error::AppError::validation(
                field,
                crate::error::ValidationCode::InvalidIsoTimestamp,
                format!("Invalid {field} timestamp."),
            )),
        }
    }
}

impl<'de> Deserialize<'de> for TimestampInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TimestampVisitor;

        impl<'de> Visitor<'de> for TimestampVisitor {
            type Value = TimestampInput;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an RFC3339 timestamp string, millisecond integer, or null")
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(TimestampInput::Empty)
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(TimestampInput::Empty)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let trimmed = v.trim();
                if trimmed.is_empty() {
                    Ok(TimestampInput::Empty)
                } else if trimmed.len() <= crate::constants::limits::MAX_EXPIRES_AT_LENGTH
                    && DateTime::parse_from_rfc3339(trimmed).is_ok()
                {
                    Ok(TimestampInput::Iso(trimmed.to_string()))
                } else {
                    Ok(TimestampInput::Invalid)
                }
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(match DateTime::from_timestamp_millis(v) {
                    Some(ts) => {
                        TimestampInput::Iso(ts.to_rfc3339_opts(SecondsFormat::Millis, true))
                    }
                    None => TimestampInput::Invalid,
                })
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(i64::try_from(v)
                    .ok()
                    .and_then(DateTime::from_timestamp_millis)
                    .map_or(TimestampInput::Invalid, |ts| {
                        TimestampInput::Iso(ts.to_rfc3339_opts(SecondsFormat::Millis, true))
                    }))
            }

            fn visit_f64<E: de::Error>(self, _v: f64) -> Result<Self::Value, E> {
                Ok(TimestampInput::Invalid)
            }

            fn visit_bool<E: de::Error>(self, _v: bool) -> Result<Self::Value, E> {
                Ok(TimestampInput::Invalid)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(TimestampInput::Invalid)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(TimestampInput::Invalid)
            }
        }

        deserializer.deserialize_any(TimestampVisitor)
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
        assert_eq!(
            serde_json::from_str::<PermBox>(r#"{"allow":"1024"}"#)
                .unwrap()
                .allow,
            Some(1024)
        );
        assert_eq!(
            serde_json::from_str::<PermBox>(r#"{"allow":2048}"#)
                .unwrap()
                .allow,
            Some(2048)
        );
        assert_eq!(
            serde_json::from_str::<PermBox>(r#"{"allow":null}"#)
                .unwrap()
                .allow,
            None
        );
        assert!(serde_json::from_str::<PermBox>(r#"{"allow":"-5"}"#).is_err());
    }

    #[test]
    fn timestamp_input_validates_all_json_variants() {
        assert_eq!(
            serde_json::from_str::<Option<TimestampInput>>("null").unwrap(),
            None
        );
        assert_eq!(
            serde_json::from_str::<TimestampInput>(r#""  ""#)
                .unwrap()
                .into_opt_iso("exp")
                .unwrap(),
            None
        );
        assert_eq!(
            serde_json::from_str::<TimestampInput>(r#""2026-01-01T00:00:00Z""#)
                .unwrap()
                .into_opt_iso("exp")
                .unwrap(),
            Some("2026-01-01T00:00:00Z".to_string())
        );
        assert!(
            serde_json::from_str::<TimestampInput>("1700000000000")
                .unwrap()
                .into_opt_iso("exp")
                .unwrap()
                .is_some()
        );
        assert!(
            serde_json::from_str::<TimestampInput>(r#""not-a-date""#)
                .unwrap()
                .into_opt_iso("exp")
                .is_err()
        );
        assert!(
            serde_json::from_str::<TimestampInput>("true")
                .unwrap()
                .into_opt_iso("exp")
                .is_err()
        );
        assert!(
            serde_json::from_str::<TimestampInput>("[1,2]")
                .unwrap()
                .into_opt_iso("exp")
                .is_err()
        );
    }
}
