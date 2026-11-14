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