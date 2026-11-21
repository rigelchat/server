// This file is a Rust port/adaptation of the JavaScript implementation
// from the Sapphire Utilities project:
// https://github.com/sapphiredev/utilities/tree/main/packages/snowflake
//
// Original implementation licensed under MIT.

#![allow(dead_code)]

use std::cmp::Ordering;
use std::sync::atomic::{AtomicU16, Ordering as AtomicOrdering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAXIMUM_WORKER_ID: u64 = 0b11111;
pub const MAXIMUM_PROCESS_ID: u64 = 0b11111;
pub const MAXIMUM_INCREMENT: u16 = 0b111111111111;

pub static DISCORD_SNOWFLAKE: Snowflake = Snowflake::new(1420070400000); // 2015-01-01T00:00:00.000Z
pub static TWITTER_SNOWFLAKE: Snowflake = Snowflake::new(1288834974657); // 2010-11-04T01:42:54.657Z

#[derive(Debug)]
pub struct Snowflake {
    epoch: u64,
    process_id: u64,
    worker_id: u64,
    increment: AtomicU16,
}

#[derive(Default, Debug, Clone)]
pub struct GenerateOptions {
    pub increment: Option<u16>,
    pub timestamp: Option<u64>,
    pub worker_id: Option<u64>,
    pub process_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeconstructedSnowflake {
    pub id: u64,
    pub timestamp: u64,
    pub worker_id: u64,
    pub process_id: u64,
    pub increment: u64,
    pub epoch: u64,
}

impl Snowflake {
    pub const fn new(epoch: u64) -> Self {
        Self {
            epoch,
            process_id: 1,
            worker_id: 0,
            increment: AtomicU16::new(0),
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn process_id(&self) -> u64 {
        self.process_id
    }

    pub fn set_process_id(&mut self, value: u64) {
        self.process_id = value & MAXIMUM_PROCESS_ID;
    }

    pub fn worker_id(&self) -> u64 {
        self.worker_id
    }

    pub fn set_worker_id(&mut self, value: u64) {
        self.worker_id = value & MAXIMUM_WORKER_ID;
    }

    pub fn generate(&self, options: Option<GenerateOptions>) -> u64 {
        let opts = options.unwrap_or_default();

        let timestamp = opts.timestamp.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(self.epoch)
        });

        let worker_id = opts.worker_id.unwrap_or(self.worker_id) & MAXIMUM_WORKER_ID;
        let process_id = opts.process_id.unwrap_or(self.process_id) & MAXIMUM_PROCESS_ID;

        let increment = opts.increment.unwrap_or_else(|| {
            let current = self.increment.fetch_add(1, AtomicOrdering::Relaxed);
            current & MAXIMUM_INCREMENT
        }) as u64;

        (timestamp.saturating_sub(self.epoch) << 22)
            | (worker_id << 17)
            | (process_id << 12)
            | increment
    }

    pub fn deconstruct(&self, id: u64) -> DeconstructedSnowflake {
        DeconstructedSnowflake {
            id,
            timestamp: (id >> 22) + self.epoch,
            worker_id: (id >> 17) & MAXIMUM_WORKER_ID,
            process_id: (id >> 12) & MAXIMUM_PROCESS_ID,
            increment: id & (MAXIMUM_INCREMENT as u64),
            epoch: self.epoch,
        }
    }

    pub fn decode(&self, id: u64) -> DeconstructedSnowflake {
        self.deconstruct(id)
    }

    pub fn timestamp_from(&self, id: u64) -> u64 {
        (id >> 22) + self.epoch
    }

    pub fn compare_strings(a: &str, b: &str) -> Ordering {
        if a.len() != b.len() {
            a.len().cmp(&b.len())
        } else {
            a.cmp(b)
        }
    }
}

pub fn validate_snowflake(field: &str, value: &str) -> Result<(), crate::error::AppError> {
    if value.is_empty()
        || value.len() > crate::constants::limits::MAX_SNOWFLAKE_LENGTH
        || !value.bytes().all(|b| b.is_ascii_digit())
        || value.parse::<u64>().is_err()
    {
        return Err(crate::error::AppError::validation(
            field,
            crate::error::ValidationCode::InvalidFormat,
            format!("Invalid snowflake for '{field}'."),
        ));
    }
    Ok(())
}

pub fn validate_snowflake_opt(
    field: &str,
    value: Option<&str>,
) -> Result<(), crate::error::AppError> {
    if let Some(s) = value {
        validate_snowflake(field, s)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_deconstruct_roundtrip() {
        let sf = Snowflake::new(1420070400000);
        let id = sf.generate(Some(GenerateOptions {
            timestamp: Some(1700000000000),
            worker_id: Some(3),
            process_id: Some(7),
            increment: Some(42),
        }));
        let decoded = sf.deconstruct(id);
        assert_eq!(decoded.timestamp, 1700000000000);
        assert_eq!(decoded.worker_id, 3);
        assert_eq!(decoded.process_id, 7);
        assert_eq!(decoded.increment, 42);
    }

    #[test]
    fn validate_snowflake_checks_bounds_and_digits() {
        assert!(validate_snowflake("after", "123456789012345678").is_ok());
        assert!(validate_snowflake("after", "").is_err());
        assert!(validate_snowflake("after", "abc").is_err());
        assert!(validate_snowflake("after", "99999999999999999999").is_err());
        assert!(validate_snowflake("after", "123456789012345678901").is_err());
    }
}
