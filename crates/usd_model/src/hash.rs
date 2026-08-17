//! Fixed-size digests used for semantic/cache identity.

use std::fmt;

use serde::{Deserialize, Serialize};

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// A 256-bit digest used for persistent semantic and cache identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct HashDigest([u8; 32]);

impl HashDigest {
    pub const BYTE_LEN: usize = 32;
    pub const HEX_LEN: usize = Self::BYTE_LEN * 2;

    pub const fn new(bytes: [u8; Self::BYTE_LEN]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; Self::BYTE_LEN] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        let mut output = String::with_capacity(Self::HEX_LEN);
        for byte in self.0 {
            output.push(HEX_DIGITS[(byte >> 4) as usize] as char);
            output.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
        }
        output
    }

    pub fn from_hex(value: &str) -> Result<Self, HashDigestError> {
        if value.len() != Self::HEX_LEN {
            return Err(HashDigestError::InvalidLength {
                expected: Self::HEX_LEN,
                actual: value.len(),
            });
        }

        let bytes = value.as_bytes();
        let mut digest = [0; Self::BYTE_LEN];
        for (index, output) in digest.iter_mut().enumerate() {
            let high =
                decode_hex_digit(bytes[index * 2]).ok_or(HashDigestError::InvalidHexDigit {
                    index: index * 2,
                    byte: bytes[index * 2],
                })?;
            let low =
                decode_hex_digit(bytes[index * 2 + 1]).ok_or(HashDigestError::InvalidHexDigit {
                    index: index * 2 + 1,
                    byte: bytes[index * 2 + 1],
                })?;
            *output = (high << 4) | low;
        }

        Ok(Self(digest))
    }
}

impl From<[u8; HashDigest::BYTE_LEN]> for HashDigest {
    fn from(bytes: [u8; HashDigest::BYTE_LEN]) -> Self {
        Self::new(bytes)
    }
}

impl From<HashDigest> for [u8; HashDigest::BYTE_LEN] {
    fn from(digest: HashDigest) -> Self {
        digest.0
    }
}

impl fmt::Display for HashDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&(*self).to_hex())
    }
}

/// Error returned when parsing a hexadecimal [`HashDigest`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HashDigestError {
    InvalidLength { expected: usize, actual: usize },
    InvalidHexDigit { index: usize, byte: u8 },
}

impl fmt::Display for HashDigestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "expected {expected} hexadecimal characters, got {actual}"
                )
            }
            Self::InvalidHexDigit { index, byte } => {
                write!(
                    formatter,
                    "invalid hexadecimal digit at index {index}: 0x{byte:02x}"
                )
            }
        }
    }
}

impl std::error::Error for HashDigestError {}

fn decode_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_preserves_all_bytes() {
        let original = HashDigest::new([
            0x00, 0x01, 0x0f, 0x10, 0x2a, 0x55, 0x7f, 0x80, 0xa5, 0xff, 0x11, 0x22, 0x33, 0x44,
            0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xf0, 0xfe, 0x12, 0x34, 0x56,
            0x78, 0x9a, 0xbc, 0xde,
        ]);

        let encoded = original.to_hex();
        let decoded = HashDigest::from_hex(&encoded).expect("encoded digest must parse");

        assert_eq!(encoded.len(), HashDigest::HEX_LEN);
        assert_eq!(decoded, original);
        assert_eq!(decoded.to_string(), encoded);
    }

    #[test]
    fn hex_parser_accepts_uppercase() {
        let digest = HashDigest::from_hex(&"AB".repeat(HashDigest::BYTE_LEN))
            .expect("uppercase hexadecimal must parse");

        assert_eq!(digest.as_bytes(), &[0xab; HashDigest::BYTE_LEN]);
    }

    #[test]
    fn hex_parser_reports_invalid_input() {
        assert_eq!(
            HashDigest::from_hex("00"),
            Err(HashDigestError::InvalidLength {
                expected: HashDigest::HEX_LEN,
                actual: 2,
            })
        );

        let mut invalid = "00".repeat(HashDigest::HEX_LEN / 2);
        invalid.replace_range(30..31, "g");
        assert_eq!(
            HashDigest::from_hex(&invalid),
            Err(HashDigestError::InvalidHexDigit {
                index: 30,
                byte: b'g',
            })
        );
    }
}
