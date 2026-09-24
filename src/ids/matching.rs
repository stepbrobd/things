use std::collections::HashMap;

use crate::ids::{ThingsId, things_id::base58_encode_fixed};

pub fn lcp_len(a: &str, b: &str) -> usize {
    lcp_len_bytes(a.as_bytes(), b.as_bytes())
}

/// the shortest unique prefix of every id
///
/// each id is encoded once into a stack buffer
/// the sort and the common prefix scan run on byte slices
/// only the result allocates a string per entry
/// duplicate ids in the input collapse into one entry
pub fn shortest_unique_prefixes(ids: &[ThingsId]) -> HashMap<ThingsId, String> {
    if ids.is_empty() {
        return HashMap::new();
    }

    let mut pairs: Vec<(ThingsId, [u8; 22], usize)> = ids
        .iter()
        .map(|id| {
            let (buf, len) = base58_encode_fixed(id.as_bytes());
            (id.clone(), buf, len)
        })
        .collect();
    pairs.sort_unstable_by(|a, b| a.1[..a.2].cmp(&b.1[..b.2]));

    let n = pairs.len();
    let mut result = HashMap::with_capacity(n);
    for i in 0..n {
        let enc = &pairs[i].1[..pairs[i].2];
        let left = if i > 0 {
            lcp_len_bytes(enc, &pairs[i - 1].1[..pairs[i - 1].2])
        } else {
            0
        };
        let right = if i + 1 < n {
            lcp_len_bytes(enc, &pairs[i + 1].1[..pairs[i + 1].2])
        } else {
            0
        };
        // one character past the shared prefix
        // clamped to the full encoding
        // when one encoding is a prefix of another, the whole string is the shortest unique prefix
        let need = (left.max(right) + 1).min(pairs[i].2);
        let prefix = std::str::from_utf8(&enc[..need])
            .expect("base58 output must be ASCII")
            .to_owned();
        result.insert(pairs[i].0.clone(), prefix);
    }

    result
}

/// the width that shows every id of a group unique, the longest of their shortest unique prefixes
pub fn longest_shortest_unique_prefix_len(ids: &[ThingsId]) -> usize {
    if ids.is_empty() {
        return 0;
    }

    shortest_unique_prefixes(ids)
        .values()
        .map(|s| s.len())
        .max()
        .unwrap_or(1)
}

#[inline]
fn lcp_len_bytes(a: &[u8], b: &[u8]) -> usize {
    let max = a.len().min(b.len());
    for i in 0..max {
        if a[i] != b[i] {
            return i;
        }
    }
    max
}

/// every id whose base58 form starts with `prefix`, in the order given
///
/// ids sorted by their bytes stop being contiguous by textual prefix once their encoded lengths differ
/// the whole list is therefore scanned rather than stopped at the first miss after a match
pub fn prefix_matches<'a>(sorted_ids: &'a [ThingsId], prefix: &str) -> Vec<&'a ThingsId> {
    let prefix = prefix.as_bytes();
    if prefix.is_empty() {
        return sorted_ids.iter().collect();
    }

    sorted_ids
        .iter()
        .filter(|id| {
            let (buf, len) = base58_encode_fixed(id.as_bytes());
            buf[..len].starts_with(prefix)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortest_unique_prefixes_are_actually_unique() {
        // every prefix of a random set matches exactly one of its ids
        let ids: Vec<ThingsId> = (0..200).map(|_| ThingsId::random()).collect();
        let prefixes = shortest_unique_prefixes(&ids);

        assert_eq!(prefixes.len(), ids.len());

        for (id, prefix) in &prefixes {
            let matches: Vec<&ThingsId> = ids
                .iter()
                .filter(|other| other.to_string().starts_with(prefix.as_str()))
                .collect();
            assert_eq!(
                matches.len(),
                1,
                "prefix {:?} for {:?} matched {} IDs: {:?}",
                prefix,
                id.to_string(),
                matches.len(),
                matches.iter().map(|m| m.to_string()).collect::<Vec<_>>()
            );
            assert_eq!(matches[0], id);
        }
    }

    #[test]
    fn shortest_unique_prefixes_are_minimal() {
        // every prefix is minimal
        // one character less matches more than one id
        let ids: Vec<ThingsId> = (0..200).map(|_| ThingsId::random()).collect();
        let prefixes = shortest_unique_prefixes(&ids);

        for prefix in prefixes.values() {
            if prefix.len() <= 1 {
                continue; // can't shorten a 1-char prefix
            }
            let shorter = &prefix[..prefix.len() - 1];
            let matches: Vec<&ThingsId> = ids
                .iter()
                .filter(|other| other.to_string().starts_with(shorter))
                .collect();
            assert!(
                matches.len() > 1,
                "prefix {:?} is not minimal, {:?} is one char shorter and still matches 1 ID alone",
                prefix,
                shorter
            );
        }
    }

    #[test]
    fn every_id_with_the_prefix_is_found_whatever_its_encoded_length() {
        // byte order puts both 21 character ids before the 22 character one
        // that order separates the two ids starting with A by the one starting with z
        let short_a = "A11111111111111111111";
        let short_z = "z11111111111111111111";
        let long_a = "A111111111111111111111";
        let mut sorted: Vec<ThingsId> = [short_a, short_z, long_a]
            .iter()
            .map(|id| id.parse().expect("a valid id"))
            .collect();
        sorted.sort();
        assert_eq!(
            sorted.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec![short_a, short_z, long_a]
        );

        let matches = prefix_matches(&sorted, "A");
        assert_eq!(
            matches.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec![short_a, long_a]
        );
        assert_eq!(prefix_matches(&sorted, "z").len(), 1);
        assert_eq!(prefix_matches(&sorted, "A1111111111111111111111").len(), 0);
    }

    #[test]
    fn single_id() {
        let ids = vec![ThingsId::random()];
        let prefixes = shortest_unique_prefixes(&ids);
        assert_eq!(prefixes.len(), 1);
        let prefix = prefixes.values().next().unwrap();
        assert_eq!(prefix.len(), 1, "single ID should have 1-char prefix");
    }

    #[test]
    fn empty_input() {
        let prefixes = shortest_unique_prefixes(&[]);
        assert!(prefixes.is_empty());
    }

    #[test]
    fn longest_shortest_prefix_len_drives_fixed_width_unique_rendering() {
        fn find_case_ids() -> Vec<ThingsId> {
            let mut a: Option<ThingsId> = None;
            let mut b_left: Option<ThingsId> = None;
            let mut b_right: Option<ThingsId> = None;

            for _ in 0..1_000_000 {
                let id = ThingsId::random();
                let (buf, len) = base58_encode_fixed(id.as_bytes());
                let s = std::str::from_utf8(&buf[..len]).unwrap().to_owned();

                if s.starts_with('A') {
                    if a.is_none() {
                        a = Some(id);
                    }
                    continue;
                }

                if s.starts_with('B') {
                    let second = s.chars().nth(1).unwrap_or('1');
                    if let Some(left) = b_left.as_ref() {
                        let left_second = left.to_string().chars().nth(1).unwrap_or('1');
                        if second != left_second {
                            b_right = Some(id);
                        }
                    } else {
                        b_left = Some(id);
                    }
                }

                if a.is_some() && b_left.is_some() && b_right.is_some() {
                    break;
                }
            }

            vec![
                a.expect("must find an A* id"),
                b_left.expect("must find first B* id"),
                b_right.expect("must find second B* id with different second char"),
            ]
        }

        let ids = find_case_ids();
        let width = longest_shortest_unique_prefix_len(&ids);
        assert_eq!(width, 2);

        for id in &ids {
            let prefix = id.to_string().chars().take(width).collect::<String>();
            let matches: Vec<&ThingsId> = ids
                .iter()
                .filter(|other| other.to_string().starts_with(prefix.as_str()))
                .collect();
            assert_eq!(
                matches.len(),
                1,
                "prefix {prefix:?} should identify exactly one id"
            );
            assert_eq!(matches[0], id);
        }
    }
}
