use std::fmt;
use std::str::FromStr;

use sha2::{Digest, Sha256};

use crate::error::{KernelError, Result};

/// A SHA-256 digest used for canonical object bytes and evidence binding.
///
/// The textual form is deliberately tagged so a hash cannot be confused with
/// an untyped identifier or a digest produced by another algorithm.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentHash([u8; 32]);

impl AsRef<[u8]> for ContentHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; 32]> for ContentHash {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl ContentHash {
    #[must_use]
    pub fn digest(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn short_hex(&self, bytes: usize) -> String {
        encode_hex(&self.0[..bytes.min(self.0.len())])
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "sha256:{}", encode_hex(&self.0))
    }
}

impl FromStr for ContentHash {
    type Err = KernelError;

    fn from_str(value: &str) -> Result<Self> {
        let hex = value
            .strip_prefix("sha256:")
            .ok_or_else(|| KernelError::Integrity("content hash must start with sha256:".into()))?;
        if hex.len() != 64 {
            return Err(KernelError::Integrity(
                "content hash must contain 64 hexadecimal digits".into(),
            ));
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
        }
        Ok(Self(bytes))
    }
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(KernelError::Integrity(
            "content hash contains non-lowercase-hex data".into(),
        )),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            ContentHash::digest(b"abc").to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            ContentHash::digest(b"").to_string(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hashes_round_trip_and_reject_malformed_input() {
        let hash = ContentHash::digest(b"disk truth");
        assert_eq!(hash.to_string().parse::<ContentHash>().unwrap(), hash);
        assert!("sha256:ABC".parse::<ContentHash>().is_err());
        assert!("md5:00000000000000000000000000000000"
            .parse::<ContentHash>()
            .is_err());
    }
}
