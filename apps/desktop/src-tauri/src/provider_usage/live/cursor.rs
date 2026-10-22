//! Cursor `GetFilteredUsageEvents` pages, read strictly.
//!
//! This module holds only the page parser. It does no I/O.
//!
//! # What this parses
//!
//! The JSON body from
//! `POST https://api2.cursor.sh/aiserver.v1.DashboardService/GetFilteredUsageEvents`.
//! Call [`parse_usage_page`] on that body. A test fixture does not need to
//! look like an HTTP response.
//!
//! # What the payload looks like
//!
//! A page carries `totalUsageEventsCount` and `usageEventsDisplay[]`. Each
//! event keeps only: `timestamp` (epoch ms, often a string), `model`,
//! `kind`, optional `tokenUsage` (`inputTokens`, `outputTokens`,
//! `cacheReadTokens`, `totalCents`), `chargedCents`, `conversationId`,
//! `isHeadless`, and `usageEventId`. The parser ignores unknown fields.
//!
//! # Legacy request events
//!
//! Old request-based events (kind `USAGE_EVENT_KIND_CUSTOM_SUBSCRIPTION`,
//! seen from 2024 to early 2025) have no `usageEventId` and no
//! `tokenUsage`. They hold no tokens or recorded cost, so the parser skips
//! them and counts them in [`CursorUsagePage::skipped`]. An event that has
//! `tokenUsage` but no `usageEventId` still rejects the page.
//!
//! # Connect JSON defaults
//!
//! Connect JSON may omit zero, false, and empty values. A missing
//! `isHeadless` means false. A missing `chargedCents` or
//! `totalUsageEventsCount` means 0. A missing `usageEventsDisplay` means an
//! empty list. A missing `model`, `kind`, or `conversationId` means an empty
//! string. A present field with the wrong type still rejects the page.
//!
//! # Fail closed
//!
//! A present field with the wrong JSON type, a non-finite number, or a
//! negative token or cost value rejects the whole page. A missing
//! `tokenUsage` object is not an error: tokens stay zero and recorded cost
//! stays absent. Cursor's `totalCents` becomes micros of a dollar
//! (`cents × 10_000`) for the recorded cost.

use serde_json::Value;

use super::model::{ProviderUsageError, SchemaReason};

/// One cent of a dollar, in micros.
const MICROS_PER_CENT: f64 = 10_000.0;

/// What a well-formed usage-event page yielded.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorUsagePage {
    pub total_usage_events_count: u64,
    pub events: Vec<CursorUsageEvent>,
    /// Legacy request events on this page that the parser skipped. They
    /// count toward `total_usage_events_count`.
    pub skipped: usize,
}

/// One usage event from `usageEventsDisplay`.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorUsageEvent {
    pub timestamp_ms: u64,
    pub model: String,
    pub kind: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    /// From `tokenUsage.totalCents`, when that object and field are present.
    pub recorded_cost_micros: Option<u64>,
    pub charged_cents: f64,
    pub conversation_id: String,
    pub is_headless: bool,
    pub usage_event_id: String,
}

/// Parse a `GetFilteredUsageEvents` page into events and the stated total.
///
/// Reject the whole page when a present field is malformed. A partial page
/// would store wrong tokens or cost.
pub fn parse_usage_page(input: &str) -> Result<CursorUsagePage, ProviderUsageError> {
    let value: Value = serde_json::from_str(input)
        .map_err(|_| ProviderUsageError::Schema(SchemaReason::InvalidJson))?;
    let envelope = value
        .as_object()
        .ok_or(ProviderUsageError::Schema(SchemaReason::MissingEnvelope))?;

    let total_usage_events_count =
        optional_u64(envelope.get("totalUsageEventsCount"))?.unwrap_or(0);
    let mut events = Vec::new();
    let mut skipped = 0;
    match envelope.get("usageEventsDisplay") {
        None => {}
        Some(value) if value.is_null() => {}
        Some(value) => {
            let items = value
                .as_array()
                .ok_or(ProviderUsageError::Schema(SchemaReason::InvalidValue))?;
            events.reserve(items.len());
            for event in items {
                match parse_event(event)? {
                    Some(event) => events.push(event),
                    None => skipped += 1,
                }
            }
        }
    }

    Ok(CursorUsagePage {
        total_usage_events_count,
        events,
        skipped,
    })
}

/// Parse one event. `None` is a legacy request event, which has neither
/// `usageEventId` nor `tokenUsage`.
fn parse_event(value: &Value) -> Result<Option<CursorUsageEvent>, ProviderUsageError> {
    let event = value
        .as_object()
        .ok_or(ProviderUsageError::Schema(SchemaReason::InvalidValue))?;
    let absent = |key: &str| event.get(key).is_none_or(Value::is_null);
    if absent("usageEventId") && absent("tokenUsage") {
        return Ok(None);
    }

    // Timestamp and usageEventId identify the row. Connect JSON does not omit
    // them when they hold a real value, and a default of 0 or "" cannot stand
    // in for a stated event.
    let timestamp_ms = required_u64(event.get("timestamp"))?;
    let usage_event_id = required_string(event.get("usageEventId"))?;

    let model = optional_string(event.get("model"))?.unwrap_or_default();
    let kind = optional_string(event.get("kind"))?.unwrap_or_default();
    let conversation_id = optional_string(event.get("conversationId"))?.unwrap_or_default();
    let charged_cents = optional_nonnegative_f64(event.get("chargedCents"))?.unwrap_or(0.0);
    let is_headless = optional_bool(event.get("isHeadless"))?.unwrap_or(false);

    let (input_tokens, output_tokens, cache_read_tokens, recorded_cost_micros) =
        parse_token_usage(event.get("tokenUsage"))?;

    Ok(Some(CursorUsageEvent {
        timestamp_ms,
        model,
        kind,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        recorded_cost_micros,
        charged_cents,
        conversation_id,
        is_headless,
        usage_event_id,
    }))
}

/// Absent or null `tokenUsage` yields zero tokens and no recorded cost.
fn parse_token_usage(
    value: Option<&Value>,
) -> Result<(u64, u64, u64, Option<u64>), ProviderUsageError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok((0, 0, 0, None));
    };
    let usage = value
        .as_object()
        .ok_or(ProviderUsageError::Schema(SchemaReason::InvalidValue))?;

    let input_tokens = optional_u64(usage.get("inputTokens"))?.unwrap_or(0);
    let output_tokens = optional_u64(usage.get("outputTokens"))?.unwrap_or(0);
    let cache_read_tokens = optional_u64(usage.get("cacheReadTokens"))?.unwrap_or(0);
    let recorded_cost_micros = match optional_nonnegative_f64(usage.get("totalCents"))? {
        Some(cents) => Some(cents_to_micros(cents)?),
        None => None,
    };

    Ok((
        input_tokens,
        output_tokens,
        cache_read_tokens,
        recorded_cost_micros,
    ))
}

fn cents_to_micros(cents: f64) -> Result<u64, ProviderUsageError> {
    let micros = (cents * MICROS_PER_CENT).round();
    if !micros.is_finite() || micros < 0.0 || micros > u64::MAX as f64 {
        return Err(invalid());
    }
    Ok(micros as u64)
}

/// Require a non-empty string. Reject a present non-string value.
fn required_string(value: Option<&Value>) -> Result<String, ProviderUsageError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Err(ProviderUsageError::Schema(
            SchemaReason::MissingRequiredField,
        ));
    };
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(invalid)
}

/// Accept a missing or empty string as the Connect default. Reject a present
/// non-string value.
fn optional_string(value: Option<&Value>) -> Result<Option<String>, ProviderUsageError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(invalid)
}

fn optional_bool(value: Option<&Value>) -> Result<Option<bool>, ProviderUsageError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(invalid)
}

fn optional_nonnegative_f64(value: Option<&Value>) -> Result<Option<f64>, ProviderUsageError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let number = value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or_else(invalid)?;
    if number < 0.0 {
        return Err(invalid());
    }
    Ok(Some(number))
}

fn required_u64(value: Option<&Value>) -> Result<u64, ProviderUsageError> {
    optional_u64(value)?.ok_or(ProviderUsageError::Schema(
        SchemaReason::MissingRequiredField,
    ))
}

/// Accept a JSON number or a decimal string, the two shapes Connect uses for
/// int64 fields such as `timestamp`.
fn optional_u64(value: Option<&Value>) -> Result<Option<u64>, ProviderUsageError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if let Some(number) = value.as_u64() {
        return Ok(Some(number));
    }
    if let Some(number) = value.as_i64() {
        if number < 0 {
            return Err(invalid());
        }
        return Ok(Some(number as u64));
    }
    if let Some(raw) = value.as_str() {
        let parsed = raw.parse::<u64>().map_err(|_| invalid())?;
        return Ok(Some(parsed));
    }
    // A finite float that is a whole non-negative integer still counts.
    if let Some(number) = value.as_f64().filter(|number| number.is_finite()) {
        if number < 0.0 || number.fract() != 0.0 || number > u64::MAX as f64 {
            return Err(invalid());
        }
        return Ok(Some(number as u64));
    }
    Err(invalid())
}

fn invalid() -> ProviderUsageError {
    ProviderUsageError::Schema(SchemaReason::InvalidValue)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_PAGE: &str = r#"{
      "totalUsageEventsCount": 2,
      "usageEventsDisplay": [
        {
          "timestamp": "1710000000000",
          "model": "composer-2.5",
          "kind": "USAGE_EVENT_KIND_INCLUDED",
          "tokenUsage": {
            "inputTokens": 100,
            "outputTokens": 50,
            "cacheReadTokens": 20,
            "totalCents": 1.5
          },
          "chargedCents": 0,
          "conversationId": "conv-full",
          "isHeadless": false,
          "usageEventId": "evt-full"
        },
        {
          "timestamp": 1710000001000,
          "model": "gpt-5",
          "kind": "USAGE_EVENT_KIND_ERRORED",
          "tokenUsage": {
            "inputTokens": 10,
            "outputTokens": 0,
            "cacheReadTokens": 0,
            "totalCents": 0
          },
          "chargedCents": 2.25,
          "conversationId": "conv-second",
          "isHeadless": true,
          "usageEventId": "evt-second"
        }
      ]
    }"#;

    #[test]
    fn a_full_page_carries_exact_tokens_and_cost() {
        let page = parse_usage_page(FULL_PAGE).expect("full page");
        assert_eq!(page.total_usage_events_count, 2);
        assert_eq!(page.events.len(), 2);

        let first = &page.events[0];
        assert_eq!(first.timestamp_ms, 1_710_000_000_000);
        assert_eq!(first.model, "composer-2.5");
        assert_eq!(first.kind, "USAGE_EVENT_KIND_INCLUDED");
        assert_eq!(first.input_tokens, 100);
        assert_eq!(first.output_tokens, 50);
        assert_eq!(first.cache_read_tokens, 20);
        // 1.5 cents → 15_000 micros of a dollar.
        assert_eq!(first.recorded_cost_micros, Some(15_000));
        assert_eq!(first.charged_cents, 0.0);
        assert_eq!(first.conversation_id, "conv-full");
        assert!(!first.is_headless);
        assert_eq!(first.usage_event_id, "evt-full");

        let second = &page.events[1];
        assert_eq!(second.timestamp_ms, 1_710_000_001_000);
        assert_eq!(second.recorded_cost_micros, Some(0));
        assert_eq!(second.charged_cents, 2.25);
        assert!(second.is_headless);
    }

    #[test]
    fn a_missing_token_usage_gives_zero_tokens_and_no_cost() {
        let page = parse_usage_page(
            r#"{
              "totalUsageEventsCount": 1,
              "usageEventsDisplay": [{
                "timestamp": "1710000000000",
                "model": "composer-2.5",
                "kind": "USAGE_EVENT_KIND_INCLUDED",
                "chargedCents": 0,
                "conversationId": "conv-missing",
                "isHeadless": false,
                "usageEventId": "evt-missing"
              }]
            }"#,
        )
        .expect("missing tokenUsage");
        let event = &page.events[0];
        assert_eq!(event.input_tokens, 0);
        assert_eq!(event.output_tokens, 0);
        assert_eq!(event.cache_read_tokens, 0);
        assert_eq!(event.recorded_cost_micros, None);
    }

    #[test]
    fn a_legacy_request_event_is_skipped_and_counted() {
        let page = parse_usage_page(
            r#"{
              "totalUsageEventsCount": 2,
              "usageEventsDisplay": [
                {
                  "timestamp": "1710000000000",
                  "usageEventId": "evt-current",
                  "tokenUsage": {"inputTokens": 7}
                },
                {
                  "timestamp": "1700000000000",
                  "model": "default",
                  "kind": "USAGE_EVENT_KIND_CUSTOM_SUBSCRIPTION",
                  "chargedCents": 0,
                  "requestsCosts": 1
                }
              ]
            }"#,
        )
        .expect("legacy request event");
        assert_eq!(page.skipped, 1);
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].usage_event_id, "evt-current");
    }

    #[test]
    fn an_event_with_tokens_but_no_usage_event_id_rejects_the_page() {
        let error = parse_usage_page(
            r#"{
              "totalUsageEventsCount": 1,
              "usageEventsDisplay": [{
                "timestamp": "1710000000000",
                "tokenUsage": {"inputTokens": 7, "totalCents": 1.5}
              }]
            }"#,
        )
        .expect_err("tokens without an id");
        assert_eq!(
            error,
            ProviderUsageError::Schema(SchemaReason::MissingRequiredField)
        );
    }

    #[test]
    fn a_null_token_usage_matches_a_missing_one() {
        let page = parse_usage_page(
            r#"{
              "totalUsageEventsCount": 1,
              "usageEventsDisplay": [{
                "timestamp": "1710000000000",
                "model": "composer-2.5",
                "kind": "USAGE_EVENT_KIND_INCLUDED",
                "tokenUsage": null,
                "chargedCents": 0,
                "conversationId": "conv-null",
                "isHeadless": false,
                "usageEventId": "evt-null"
              }]
            }"#,
        )
        .expect("null tokenUsage");
        assert_eq!(page.events[0].recorded_cost_micros, None);
        assert_eq!(page.events[0].input_tokens, 0);
    }

    #[test]
    fn omitted_connect_defaults_parse_as_zero_false_and_empty() {
        // Connect JSON may leave out default-valued fields. An empty object
        // is a valid empty last page; a non-headless event may omit
        // isHeadless and chargedCents when both are their defaults.
        let empty_page = parse_usage_page("{}").expect("empty page");
        assert_eq!(empty_page.total_usage_events_count, 0);
        assert!(empty_page.events.is_empty());

        let page = parse_usage_page(
            r#"{
              "usageEventsDisplay": [{
                "timestamp": "1710000000000",
                "usageEventId": "evt-defaults"
              }]
            }"#,
        )
        .expect("omitted defaults");
        assert_eq!(page.total_usage_events_count, 0);
        let event = &page.events[0];
        assert_eq!(event.timestamp_ms, 1_710_000_000_000);
        assert_eq!(event.usage_event_id, "evt-defaults");
        assert_eq!(event.model, "");
        assert_eq!(event.kind, "");
        assert_eq!(event.conversation_id, "");
        assert_eq!(event.charged_cents, 0.0);
        assert!(!event.is_headless);
        assert_eq!(event.input_tokens, 0);
        assert_eq!(event.output_tokens, 0);
        assert_eq!(event.cache_read_tokens, 0);
        assert_eq!(event.recorded_cost_micros, None);
    }

    #[test]
    fn a_malformed_number_rejects_the_page() {
        let malformed = r#"{
          "totalUsageEventsCount": 1,
          "usageEventsDisplay": [{
            "timestamp": "not-a-number",
            "model": "composer-2.5",
            "kind": "USAGE_EVENT_KIND_INCLUDED",
            "tokenUsage": {
              "inputTokens": 1,
              "outputTokens": 1,
              "cacheReadTokens": 0,
              "totalCents": 1
            },
            "chargedCents": 0,
            "conversationId": "conv-bad",
            "isHeadless": false,
            "usageEventId": "evt-bad"
          }]
        }"#;
        assert_eq!(
            parse_usage_page(malformed),
            Err(ProviderUsageError::Schema(SchemaReason::InvalidValue))
        );

        let bad_cents = r#"{
          "totalUsageEventsCount": 1,
          "usageEventsDisplay": [{
            "timestamp": "1710000000000",
            "model": "composer-2.5",
            "kind": "USAGE_EVENT_KIND_INCLUDED",
            "tokenUsage": {
              "inputTokens": 1,
              "outputTokens": 1,
              "cacheReadTokens": 0,
              "totalCents": "one"
            },
            "chargedCents": 0,
            "conversationId": "conv-bad-cents",
            "isHeadless": false,
            "usageEventId": "evt-bad-cents"
          }]
        }"#;
        assert_eq!(
            parse_usage_page(bad_cents),
            Err(ProviderUsageError::Schema(SchemaReason::InvalidValue))
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let page = parse_usage_page(
            r#"{
              "totalUsageEventsCount": 1,
              "nextPageCursor": "ignore-me",
              "usageEventsDisplay": [{
                "timestamp": "1710000000000",
                "model": "composer-2.5",
                "kind": "USAGE_EVENT_KIND_INCLUDED",
                "tokenUsage": {
                  "inputTokens": 3,
                  "outputTokens": 4,
                  "cacheReadTokens": 5,
                  "totalCents": 2,
                  "extraNested": true
                },
                "chargedCents": 1,
                "conversationId": "conv-extra",
                "isHeadless": false,
                "usageEventId": "evt-extra",
                "requestId": "ignore-this-too"
              }]
            }"#,
        )
        .expect("unknown fields");
        let event = &page.events[0];
        assert_eq!(event.input_tokens, 3);
        assert_eq!(event.output_tokens, 4);
        assert_eq!(event.cache_read_tokens, 5);
        assert_eq!(event.recorded_cost_micros, Some(20_000));
        assert_eq!(event.charged_cents, 1.0);
    }

    #[test]
    fn a_negative_token_count_rejects_the_page() {
        let page = r#"{
          "totalUsageEventsCount": 1,
          "usageEventsDisplay": [{
            "timestamp": "1710000000000",
            "model": "composer-2.5",
            "kind": "USAGE_EVENT_KIND_INCLUDED",
            "tokenUsage": {
              "inputTokens": -1,
              "outputTokens": 0,
              "cacheReadTokens": 0,
              "totalCents": 0
            },
            "chargedCents": 0,
            "conversationId": "conv-neg",
            "isHeadless": false,
            "usageEventId": "evt-neg"
          }]
        }"#;
        assert_eq!(
            parse_usage_page(page),
            Err(ProviderUsageError::Schema(SchemaReason::InvalidValue))
        );
    }

    #[test]
    fn invalid_json_and_a_non_object_envelope_fail_closed() {
        assert_eq!(
            parse_usage_page("not json"),
            Err(ProviderUsageError::Schema(SchemaReason::InvalidJson))
        );
        assert_eq!(
            parse_usage_page("[]"),
            Err(ProviderUsageError::Schema(SchemaReason::MissingEnvelope))
        );
    }

    #[test]
    fn a_present_wrong_type_for_a_defaultable_field_rejects_the_page() {
        assert_eq!(
            parse_usage_page(
                r#"{
                  "usageEventsDisplay": [{
                    "timestamp": "1710000000000",
                    "isHeadless": "no",
                    "usageEventId": "evt-bad-bool"
                  }]
                }"#,
            ),
            Err(ProviderUsageError::Schema(SchemaReason::InvalidValue))
        );
    }
}
