use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::date::CivilDate;
use crate::error::{KernelError, Result};
use crate::hash::ContentHash;

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An opaque, type-prefixed, 128-bit identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId(String);

impl ObjectId {
    pub fn derive(kind: &str, seed: &[u8]) -> Result<Self> {
        validate_kind(kind)?;
        let mut material = Vec::with_capacity(kind.len() + seed.len() + 1);
        material.extend_from_slice(kind.as_bytes());
        material.push(0);
        material.extend_from_slice(seed);
        let digest = ContentHash::digest(&material);
        Ok(Self(format!("{kind}_{}", digest.short_hex(16))))
    }

    pub fn mint(kind: &str) -> Result<Self> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| KernelError::InvalidId("system clock precedes Unix epoch".into()))?;
        let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut seed = Vec::with_capacity(32);
        seed.extend_from_slice(&now.as_nanos().to_be_bytes());
        seed.extend_from_slice(&u64::from(std::process::id()).to_be_bytes());
        seed.extend_from_slice(&counter.to_be_bytes());
        Self::derive(kind, &seed)
    }

    #[must_use]
    pub fn kind(&self) -> &str {
        self.0.split_once('_').map_or("", |(kind, _)| kind)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ObjectId {
    type Err = KernelError;

    fn from_str(value: &str) -> Result<Self> {
        let (kind, opaque) = value
            .split_once('_')
            .ok_or_else(|| KernelError::InvalidId("missing type separator".into()))?;
        validate_kind(kind)?;
        if opaque.len() != 32
            || !opaque
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(KernelError::InvalidId(
                "opaque component must be 32 lowercase hexadecimal digits".into(),
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

fn validate_kind(kind: &str) -> Result<()> {
    if kind.is_empty()
        || kind.len() > 24
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(KernelError::InvalidId(
            "kind must be 1-24 lowercase ASCII letters, digits, or hyphens".into(),
        ));
    }
    Ok(())
}

/// Structured occurrence identity. The opaque key is derived, never parsed to recover structure.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OccurrenceId {
    pub series_id: ObjectId,
    pub scheduled_for: CivilDate,
    pub sequence: u32,
}

impl OccurrenceId {
    #[must_use]
    pub fn opaque_key(&self) -> ObjectId {
        let seed = format!(
            "{}\0{}\0{}",
            self.series_id, self.scheduled_for, self.sequence
        );
        ObjectId::derive("occ", seed.as_bytes()).expect("constant occurrence kind is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_ids_are_stable_and_type_scoped() {
        let first = ObjectId::derive("task", b"same seed").unwrap();
        assert_eq!(first, ObjectId::derive("task", b"same seed").unwrap());
        assert_ne!(first, ObjectId::derive("aim", b"same seed").unwrap());
    }

    #[test]
    fn opaque_ids_reject_path_material() {
        assert!("task_../../etc/passwd".parse::<ObjectId>().is_err());
        assert!(ObjectId::derive("../task", b"seed").is_err());
    }

    #[test]
    fn occurrence_keys_include_structure() {
        let base = ObjectId::derive("routine", b"walk").unwrap();
        let first = OccurrenceId {
            series_id: base.clone(),
            scheduled_for: CivilDate::new(2026, 8, 1).unwrap(),
            sequence: 1,
        };
        let second = OccurrenceId {
            sequence: 2,
            ..first.clone()
        };
        assert_ne!(first.opaque_key(), second.opaque_key());
        assert_eq!(first.series_id, base);
    }
}
