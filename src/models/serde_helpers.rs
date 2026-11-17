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
