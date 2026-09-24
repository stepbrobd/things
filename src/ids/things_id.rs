use std::{fmt, str::FromStr};

use rand::random;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor},
};

/// a Things 3 entity identifier, the 16 bytes behind a canonical base58 id
///
/// the uuid ids of histories from before base58 ids are not read
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ThingsId([u8; 16]);

impl ThingsId {
    pub fn random() -> Self {
        ThingsId(random())
    }

    pub fn from_u128(value: u128) -> Self {
        ThingsId(value.to_be_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.to_string().starts_with(prefix)
    }
}

impl fmt::Display for ThingsId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (buf, len) = base58_encode_fixed(&self.0);
        let encoded = std::str::from_utf8(&buf[..len]).expect("base58 output must be ASCII");
        f.write_str(encoded)
    }
}

impl FromStr for ThingsId {
    type Err = ParseThingsIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.len() > 22 {
            return Err(ParseThingsIdError(s.to_owned()));
        }
        let decoded = base58_decode(s).ok_or_else(|| ParseThingsIdError(s.to_owned()))?;
        if decoded.len() != 16 {
            return Err(ParseThingsIdError(s.to_owned()));
        }
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&decoded);
        Ok(ThingsId(bytes))
    }
}

impl Serialize for ThingsId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ThingsId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ThingsIdVisitor;

        impl Visitor<'_> for ThingsIdVisitor {
            type Value = ThingsId;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a base58 Things ID")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<ThingsId, E> {
                v.parse().map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(ThingsIdVisitor)
    }
}

/// a string that is no [`ThingsId`]
#[derive(Debug)]
pub struct ParseThingsIdError(String);

impl fmt::Display for ParseThingsIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid Things ID: {:?}", self.0)
    }
}

const BASE58_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// 16 bytes as base58 in a stack buffer, with the number of bytes the encoding takes
pub(crate) fn base58_encode_fixed(raw: &[u8; 16]) -> ([u8; 22], usize) {
    let mut digits = [0u8; 22];
    let mut len = 0usize;

    for &byte in raw {
        let mut carry = byte as u32;
        for digit in digits[..len].iter_mut() {
            let value = (*digit as u32) * 256 + carry;
            *digit = (value % 58) as u8;
            carry = value / 58;
        }
        while carry > 0 {
            digits[len] = (carry % 58) as u8;
            len += 1;
            carry /= 58;
        }
    }

    let leading_ones = raw.iter().take_while(|&&b| b == 0).count();
    let total = leading_ones + len;
    debug_assert!(
        total <= 22,
        "base58_encode_fixed: output length {total} > 22"
    );

    let mut out = [0u8; 22];
    for b in out[..leading_ones].iter_mut() {
        *b = BASE58_ALPHABET[0];
    }
    for (i, &d) in digits[..len].iter().rev().enumerate() {
        out[leading_ones + i] = BASE58_ALPHABET[d as usize];
    }

    (out, total.max(1))
}

fn base58_digit(byte: u8) -> Option<u8> {
    BASE58_ALPHABET
        .iter()
        .position(|&c| c == byte)
        .map(|i| i as u8)
}

fn base58_decode(input: &str) -> Option<Vec<u8>> {
    if input.is_empty() {
        return Some(Vec::new());
    }

    let bytes = input.as_bytes();
    let mut leading_ones = 0usize;
    for b in bytes {
        if *b == b'1' {
            leading_ones += 1;
        } else {
            break;
        }
    }

    let mut decoded: Vec<u8> = Vec::new();
    for &ch in bytes.iter().skip(leading_ones) {
        let mut carry = base58_digit(ch)? as u32;
        for byte in &mut decoded {
            let value = (*byte as u32 * 58) + carry;
            *byte = (value & 0xff) as u8;
            carry = value >> 8;
        }
        while carry > 0 {
            decoded.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }

    let mut out = Vec::with_capacity(leading_ones + decoded.len());
    out.extend(std::iter::repeat_n(0u8, leading_ones));
    for byte in decoded.iter().rev() {
        out.push(*byte);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const COMPACT: &str = "A7h5eCi24RvAWKC3Hv3muf";

    #[test]
    fn parse_compact_preserved() {
        let id: ThingsId = COMPACT.parse().unwrap();
        assert_eq!(id.to_string(), COMPACT);
    }

    #[test]
    fn empty_string_is_error() {
        let err = "".parse::<ThingsId>();
        assert!(err.is_err());
    }

    #[test]
    fn display_roundtrip() {
        let id: ThingsId = COMPACT.parse().unwrap();
        let displayed = id.to_string();
        let reparsed: ThingsId = displayed.parse().unwrap();
        assert_eq!(id, reparsed);
    }

    #[test]
    fn random_is_unique() {
        let ids: HashSet<String> = (0..20).map(|_| ThingsId::random().to_string()).collect();
        assert_eq!(ids.len(), 20, "random IDs should be unique");
    }

    #[test]
    fn random_is_compact_length() {
        let id = ThingsId::random();
        let len = id.to_string().len();
        assert!((1..=22).contains(&len), "compact ID length must be 1..=22");
    }

    #[test]
    fn serde_roundtrip_compact() {
        let id: ThingsId = COMPACT.parse().unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let back: ThingsId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn rejects_invalid_compact_id() {
        assert!("not-a-things-id".parse::<ThingsId>().is_err());
        assert!("0OIl".parse::<ThingsId>().is_err());
        // ids from before base58 ids are not read
        for legacy in [
            "3C6BBD49-8D11-4FFF-8B0E-B8F33FA9C00A",
            "ACTIONGROUP-3C6BBD49-8D11-4FFF-8B0E-B8F33FA9C00A",
            "3C6BBD49-8D11-4FFF-8B0E-B8F33FA9C00A-20240131",
        ] {
            assert!(legacy.parse::<ThingsId>().is_err(), "{legacy}");
        }
        assert!(
            "123456789ABCDEFGHJKLMNPQRSTUVWXYZ"
                .parse::<ThingsId>()
                .is_err()
        );
    }

    #[test]
    fn base58_roundtrip_for_internal_bytes() {
        let samples = [
            [0u8; 16],
            [255u8; 16],
            [0, 0, 7, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
        ];
        for sample in samples {
            let (buf, len) = base58_encode_fixed(&sample);
            let encoded = std::str::from_utf8(&buf[..len]).unwrap().to_owned();
            let decoded = base58_decode(&encoded).unwrap();
            assert_eq!(decoded, sample);
        }
    }

    #[test]
    fn base58_encode_fixed_matches_display_encoding() {
        let mut samples: Vec<ThingsId> = vec![
            ThingsId([0u8; 16]),
            ThingsId([255u8; 16]),
            COMPACT.parse().unwrap(),
        ];
        for _ in 0..20 {
            samples.push(ThingsId::random());
        }

        for id in &samples {
            let (buf, len) = base58_encode_fixed(id.as_bytes());
            let fixed = std::str::from_utf8(&buf[..len]).unwrap().to_owned();
            let expected = id.to_string();
            assert_eq!(fixed, expected, "mismatch for {:?}", id.as_bytes());
        }
    }

    #[test]
    fn base58_encode_fixed_preserves_sort_order() {
        let ids: Vec<ThingsId> = (0..50).map(|_| ThingsId::random()).collect();
        let mut by_fixed: Vec<String> = ids
            .iter()
            .map(|id| {
                let (buf, len) = base58_encode_fixed(id.as_bytes());
                std::str::from_utf8(&buf[..len]).unwrap().to_owned()
            })
            .collect();
        by_fixed.sort();

        let mut by_string: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
        by_string.sort();

        assert_eq!(
            by_fixed, by_string,
            "base58_encode_fixed sort order != to_string sort order"
        );
    }
}
