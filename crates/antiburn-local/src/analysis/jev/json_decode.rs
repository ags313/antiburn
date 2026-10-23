//! Bounded duplicate-key checks and independent provider usage extraction.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;

use serde::de::{self, DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};

use super::{JevError, JevUsage, MAX_RESPONSE_BYTES};

// Count the root as depth one and bound untyped envelope metadata as well.
const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_NODES: usize = 16_384;

#[derive(Clone, Copy)]
pub enum JevUsageLocation {
    Direct,
    CloudflareEnvelope,
}

/// Records unambiguous usage before rejecting duplicate keys or invalid answers.
/// The first pass retains object keys only, not a second response value tree.
pub fn decode_jev_json_with_usage<T: DeserializeOwned>(
    body: &[u8],
    location: JevUsageLocation,
    record_usage: impl Fn(JevUsage) -> Result<(), JevError>,
) -> Result<T, JevError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(JevError::ResponseTooLarge);
    }
    let mut state = ScanState::default();
    let mut decoder = serde_json::Deserializer::from_slice(body);
    let scanned = ScanSeed {
        state: &mut state,
        depth: 1,
        role: match location {
            JevUsageLocation::Direct => Role::Response,
            JevUsageLocation::CloudflareEnvelope => Role::Envelope,
        },
    }
    .deserialize(&mut decoder)
    .map_err(|_| JevError::ResponseDecode)?;
    decoder.end().map_err(|_| JevError::ResponseDecode)?;
    if let Some(usage) = scanned.usage {
        record_usage(usage)?;
    }
    if state.duplicate {
        return Err(JevError::ResponseDecode);
    }
    serde_json::from_slice(body).map_err(|_| JevError::ResponseDecode)
}

#[derive(Default)]
struct ScanState {
    nodes: usize,
    duplicate: bool,
}

#[derive(Clone, Copy)]
enum Role {
    Envelope,
    Response,
    Usage,
    Token,
    Other,
}

#[derive(Default)]
struct ScannedValue {
    usage: Option<JevUsage>,
    token: Option<u64>,
}

struct ScanSeed<'a> {
    state: &'a mut ScanState,
    depth: usize,
    role: Role,
}

impl<'de> DeserializeSeed<'de> for ScanSeed<'_> {
    type Value = ScannedValue;

    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        if self.depth > MAX_JSON_DEPTH || self.state.nodes >= MAX_JSON_NODES {
            return Err(de::Error::custom("response JSON limit exceeded"));
        }
        self.state.nodes += 1;
        decoder.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for ScanSeed<'_> {
    type Value = ScannedValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded response JSON")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut keys = BTreeSet::new();
        let mut usage = None;
        let mut input_tokens = None;
        let mut output_tokens = None;
        let mut ambiguous_usage = false;
        while let Some(key) = map.next_key_seed(KeySeed)? {
            let duplicate = keys.contains(&key);
            if duplicate {
                self.state.duplicate = true;
                ambiguous_usage |= match self.role {
                    Role::Envelope | Role::Usage => true,
                    Role::Response => key == "usage",
                    _ => false,
                };
            }
            let role = match (self.role, key.as_ref()) {
                (Role::Envelope, "result") => Role::Response,
                (Role::Response, "usage") => Role::Usage,
                (Role::Usage, "input_tokens" | "output_tokens") => Role::Token,
                _ => Role::Other,
            };
            let child = map.next_value_seed(ScanSeed {
                state: self.state,
                depth: self.depth + 1,
                role,
            })?;
            if !duplicate {
                match (self.role, key.as_ref()) {
                    (Role::Envelope, "result") | (Role::Response, "usage") => usage = child.usage,
                    (Role::Usage, "input_tokens") => input_tokens = child.token,
                    (Role::Usage, "output_tokens") => output_tokens = child.token,
                    _ => {}
                }
            }
            keys.insert(key);
        }
        if matches!(self.role, Role::Usage) {
            usage = input_tokens
                .zip(output_tokens)
                .map(|(input_tokens, output_tokens)| JevUsage {
                    input_tokens,
                    output_tokens,
                });
        }
        Ok(ScannedValue {
            usage: if ambiguous_usage { None } else { usage },
            token: None,
        })
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        while sequence
            .next_element_seed(ScanSeed {
                state: self.state,
                depth: self.depth + 1,
                role: Role::Other,
            })?
            .is_some()
        {}
        Ok(ScannedValue::default())
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(ScannedValue {
            token: matches!(self.role, Role::Token).then_some(value),
            usage: None,
        })
    }

    fn visit_i64<E: de::Error>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(ScannedValue::default())
    }

    fn visit_f64<E: de::Error>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(ScannedValue::default())
    }

    fn visit_bool<E: de::Error>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(ScannedValue::default())
    }

    fn visit_str<E: de::Error>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(ScannedValue::default())
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(ScannedValue::default())
    }
}

struct KeySeed;

impl<'de> DeserializeSeed<'de> for KeySeed {
    type Value = Cow<'de, str>;

    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        decoder.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for KeySeed {
    type Value = Cow<'de, str>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object key")
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(Cow::Borrowed(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Cow::Owned(value.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn decode(
        body: &str,
        location: JevUsageLocation,
        expected: Option<JevUsage>,
    ) -> Result<Value, JevError> {
        let calls = std::cell::Cell::new(0);
        let result = decode_jev_json_with_usage(body.as_bytes(), location, |usage| {
            calls.set(calls.get() + 1);
            assert_eq!(Some(usage), expected);
            Ok(())
        });
        assert_eq!(calls.get(), usize::from(expected.is_some()));
        result
    }

    const USAGE: &str = r#"{"input_tokens":12,"output_tokens":1}"#;
    const KNOWN: Option<JevUsage> = Some(JevUsage {
        input_tokens: 12,
        output_tokens: 1,
    });

    #[test]
    fn duplicates_at_every_level_fail_without_losing_independent_usage() {
        for answer in [
            r#"{"q":{},"q":{}}"#,
            r#"{"q":{"type":"noul","noul":0.2,"noul":0.9}}"#,
            r#"{"q":{"probabilities":{"0":0.2,"0":0.8}}}"#,
            r#"{"q":{"legend":{"0":"low","0":"high"}}}"#,
            r#"{"q":{"nested":[{"key":1,"key":2}]}}"#,
            r#"{"q":{"probabilities":{"yes":0.2,"\u0079es":0.8}}}"#,
        ] {
            let direct = format!(r#"{{"model":"test","answers":{answer},"usage":{USAGE}}}"#);
            for (body, location) in [
                (direct.clone(), JevUsageLocation::Direct),
                (
                    format!(r#"{{"success":true,"result":{direct}}}"#),
                    JevUsageLocation::CloudflareEnvelope,
                ),
            ] {
                assert_eq!(
                    decode(&body, location, KNOWN),
                    Err(JevError::ResponseDecode)
                );
            }
        }
        let duplicate_answers = format!(r#"{{"answers":{{}},"answers":{{}},"usage":{USAGE}}}"#);
        assert_eq!(
            decode(&duplicate_answers, JevUsageLocation::Direct, KNOWN),
            Err(JevError::ResponseDecode)
        );
    }

    #[test]
    fn ambiguous_usage_and_envelope_members_never_select_a_value() {
        for usage in [
            r#"{"input_tokens":12,"input_tokens":99,"output_tokens":1}"#,
            r#"{"input_tokens":12,"output_tokens":1,"output_tokens":99}"#,
            r#"{"input_tokens":12,"\u0069nput_tokens":99,"output_tokens":1}"#,
        ] {
            let body = format!(r#"{{"usage":{usage}}}"#);
            assert_eq!(
                decode(&body, JevUsageLocation::Direct, None),
                Err(JevError::ResponseDecode)
            );
        }
        let result = format!(r#"{{"usage":{USAGE}}}"#);
        let duplicate_usage = format!(r#"{{"usage":{USAGE},"usage":{USAGE}}}"#);
        for (body, location) in [
            (duplicate_usage.clone(), JevUsageLocation::Direct),
            (
                format!(r#"{{"success":true,"result":{duplicate_usage}}}"#),
                JevUsageLocation::CloudflareEnvelope,
            ),
            (
                format!(r#"{{"success":true,"result":{result},"result":{result}}}"#),
                JevUsageLocation::CloudflareEnvelope,
            ),
            (
                format!(r#"{{"success":true,"success":false,"result":{result}}}"#),
                JevUsageLocation::CloudflareEnvelope,
            ),
            (
                format!(r#"{{"success":true,"errors":[],"errors":[],"result":{result}}}"#),
                JevUsageLocation::CloudflareEnvelope,
            ),
        ] {
            assert_eq!(decode(&body, location, None), Err(JevError::ResponseDecode));
        }
    }

    #[test]
    fn usage_requires_unsigned_integer_tokens_at_the_selected_path() {
        for usage in [
            r#"{"input_tokens":-1,"output_tokens":1}"#,
            r#"{"input_tokens":12.0,"output_tokens":1}"#,
            r#"{"input_tokens":"12","output_tokens":1}"#,
            r#"{"input_tokens":12}"#,
            r#"{"input_tokens":18446744073709551616,"output_tokens":1}"#,
            "null",
        ] {
            let body = format!(r#"{{"usage":{usage}}}"#);
            assert!(decode(&body, JevUsageLocation::Direct, None).is_ok());
        }
        assert!(
            decode(
                &format!(r#"{{"result":{{"usage":{USAGE}}}}}"#),
                JevUsageLocation::Direct,
                None
            )
            .is_ok()
        );
        assert!(
            decode(
                &format!(r#"{{"usage":{USAGE}}}"#),
                JevUsageLocation::CloudflareEnvelope,
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn usage_storage_failure_precedes_duplicate_and_typed_answer_errors() {
        let body = format!(r#"{{"answers":{{"q":{{}},"q":{{}}}},"usage":{USAGE}}}"#);
        let result: Result<super::super::JevResponse, _> =
            decode_jev_json_with_usage(body.as_bytes(), JevUsageLocation::Direct, |_| {
                Err(JevError::ProgressStorageFailure)
            });
        assert_eq!(result, Err(JevError::ProgressStorageFailure));
    }

    #[test]
    fn byte_depth_and_node_limits_apply_before_typed_allocation() {
        let body = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_DEPTH - 1),
            "]".repeat(MAX_JSON_DEPTH - 1)
        );
        assert!(decode(&body, JevUsageLocation::Direct, None).is_ok());
        let body = format!("[{body}]");
        assert_eq!(
            decode(&body, JevUsageLocation::Direct, None),
            Err(JevError::ResponseDecode)
        );

        let body = format!("[{}]", vec!["0"; MAX_JSON_NODES - 1].join(","));
        assert!(body.len() <= MAX_RESPONSE_BYTES);
        assert!(decode(&body, JevUsageLocation::Direct, None).is_ok());
        let body = body.replacen('[', "[0,", 1);
        assert!(body.len() <= MAX_RESPONSE_BYTES);
        assert_eq!(
            decode(&body, JevUsageLocation::Direct, None),
            Err(JevError::ResponseDecode)
        );

        let body = " ".repeat(MAX_RESPONSE_BYTES + 1);
        assert_eq!(
            decode(&body, JevUsageLocation::Direct, None),
            Err(JevError::ResponseTooLarge)
        );
        assert_eq!(
            decode("{}{}", JevUsageLocation::Direct, None),
            Err(JevError::ResponseDecode)
        );
    }
}
