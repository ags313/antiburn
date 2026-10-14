//! JetBrains Junie `events.jsonl` reader.
//!
//! Junie writes one `events.jsonl` per session folder. Usage records are
//! `SessionA2uxEvent` lines whose `event.agentEvent.kind` is
//! `LlmResponseMetadataEvent`. Each record carries a top-level `timestampMs`
//! and one or more `modelUsage` entries. The reader emits one assistant event
//! per entry and ignores prompts, tool blocks, and other UI state.
//!
//! `inputTokens` excludes cache reads and cache writes. The sum of these
//! entries agrees with the per-model totals in a finished session's
//! `summary.json`. The recorded `cost` is not read; the catalog prices usage.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};

use anyhow::Context;
use serde_json::Value;

use crate::analysis::PartialReason;
use crate::analysis::framing::{BoundedJsonlReader, FramedRecord, RecordSkip};
use crate::analysis::interface::{
    NormalizedRecord, RawSource, RecordSink, SessionCollector, SessionInput, SessionReader,
    SessionSummary, VisitOutcome,
};
use crate::analysis::model::{NormalizedEvent, NormalizedSession, Role, Usage};
use crate::analysis::source_validity::{AppendOnlyGuarantee, PinnedSource, SourceClaim};
use crate::analysis::{SourceCapabilities, SourceFormat};

/// Top-level record kinds that carry no usage. The list holds the kinds seen
/// on macOS and Linux installs. A kind outside this list makes the session
/// partial, because it can hold usage that this reader does not know.
const INERT_KINDS: &[&str] = &[
    "CancelAgentEvent",
    "SessionCostTrajectorySnapshotEvent",
    "SessionModelEffortSetEvent",
    "SkillsStatusEvent",
    "SystemMessageEvent",
    "TaskStartedEvent",
    "TaskState",
    "UserMessagesCommittedToHistory",
    "UserMessagesFailedInHistory",
    "UserPromptEvent",
    "UserResponseEvent",
];

const AGENT_EVENT_KIND: &str = "SessionA2uxEvent";
const USAGE_KIND: &str = "LlmResponseMetadataEvent";

pub struct JunieSessionReader;

impl SessionReader for JunieSessionReader {
    fn agent(&self) -> &'static str {
        "junie"
    }

    fn capabilities(&self, input: &SessionInput) -> SourceCapabilities {
        match input.source_format_or(SourceFormat::JunieEventsJsonl) {
            SourceFormat::JunieEventsJsonl => SourceCapabilities::junie(),
            other => SourceCapabilities::uncharacterized(other),
        }
    }

    fn normalize(&self, input: &SessionInput) -> anyhow::Result<NormalizedSession> {
        let mut collector = SessionCollector::new(input.agent.clone(), input.session_id.clone());
        self.visit(input, &mut collector)?;
        collector.into_session()
    }

    fn visit(
        &self,
        input: &SessionInput,
        sink: &mut dyn RecordSink,
    ) -> anyhow::Result<VisitOutcome> {
        require_events_format(input)?;
        let summary = match &input.source {
            RawSource::File(path) => {
                visit_events(BufReader::new(std::fs::File::open(path)?), &|| false, sink)?
            }
            RawSource::Jsonl(content) => {
                let suffix: &[u8] = if content.ends_with('\n') { b"" } else { b"\n" };
                visit_events(
                    BufReader::new(content.as_bytes().chain(suffix)),
                    &|| false,
                    sink,
                )?
            }
            _ => anyhow::bail!("a Junie session must be an events.jsonl file"),
        };
        sink.finish(summary);
        Ok(VisitOutcome::Unvalidated)
    }

    fn visit_claimed(
        &self,
        input: &SessionInput,
        claim: &SourceClaim,
        guarantee: AppendOnlyGuarantee,
        cancel: &dyn Fn() -> bool,
        sink: &mut dyn RecordSink,
    ) -> anyhow::Result<VisitOutcome> {
        require_events_format(input)?;
        let RawSource::File(path) = &input.source else {
            return self.visit(input, sink);
        };
        let mut pinned = match PinnedSource::open(path, claim.clone())? {
            Ok(pinned) => pinned,
            Err(reason) => return Ok(VisitOutcome::SourceChanged(reason)),
        };
        let limit = match guarantee {
            AppendOnlyGuarantee::Evidenced => claim.boundary,
            AppendOnlyGuarantee::Absent => u64::MAX,
        };
        let summary = visit_events(BufReader::new(pinned.reader(limit)), cancel, sink)?;
        let outcome = match guarantee {
            AppendOnlyGuarantee::Evidenced => pinned.recheck_prefix()?.map_or(
                VisitOutcome::AcceptedPrefix {
                    boundary: claim.boundary,
                },
                VisitOutcome::SourceChanged,
            ),
            AppendOnlyGuarantee::Absent => pinned
                .recheck_full()?
                .map_or(VisitOutcome::AcceptedFull, VisitOutcome::SourceChanged),
        };
        if !matches!(outcome, VisitOutcome::SourceChanged(_)) {
            sink.finish(summary);
        }
        Ok(outcome)
    }
}

fn require_events_format(input: &SessionInput) -> anyhow::Result<()> {
    if input.source_format_or(SourceFormat::JunieEventsJsonl) != SourceFormat::JunieEventsJsonl {
        anyhow::bail!("Junie reader requires JunieEventsJsonl admission");
    }
    Ok(())
}

fn visit_events(
    reader: impl BufRead,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn RecordSink,
) -> anyhow::Result<SessionSummary> {
    let mut reader = BoundedJsonlReader::new(reader);
    let mut summary = SessionSummary {
        cache_write_tokens_available: true,
        ..SessionSummary::default()
    };
    let mut tokens_by_model = BTreeMap::<String, u64>::new();
    while let Some(record) = reader.next_record(cancel) {
        match record {
            FramedRecord::Complete { bytes, .. } => {
                let record = std::str::from_utf8(bytes).context("Junie record is not UTF-8")?;
                let Ok(value) = serde_json::from_str::<Value>(record) else {
                    sink.record(NormalizedRecord::Unusable(PartialReason::MalformedRecord));
                    continue;
                };
                match classify(&value) {
                    Record::Inert => {}
                    Record::Unrecognized => {
                        sink.record(NormalizedRecord::Unusable(
                            PartialReason::UnrecognizedRecordType,
                        ));
                    }
                    Record::Usage => match usage_events(&value) {
                        Some(events) => {
                            let first_ts = events.first().and_then(|event| event.ts_ms);
                            if summary.started_at_ms.is_none() {
                                summary.started_at_ms = first_ts;
                            }
                            for event in events {
                                if let Some(model) = &event.model {
                                    let usage = &event.usage;
                                    *tokens_by_model.entry(model.clone()).or_default() += usage
                                        .input_tokens
                                        + usage.cache_read_tokens
                                        + usage.cache_creation_tokens
                                        + usage.output_tokens;
                                }
                                sink.record(NormalizedRecord::MetricsEvent(Box::new(event)));
                            }
                        }
                        None => {
                            sink.record(NormalizedRecord::Unusable(PartialReason::MalformedRecord));
                        }
                    },
                }
            }
            FramedRecord::Skipped(RecordSkip::ReadFailed { index, kind }) => {
                anyhow::bail!("Junie record {index} read failed: {kind:?}");
            }
            FramedRecord::Skipped(RecordSkip::Cancelled { index }) => {
                anyhow::bail!("Junie record {index} read was cancelled");
            }
            FramedRecord::Skipped(skip) => {
                sink.record(NormalizedRecord::Unusable(skip.partial_reason()));
            }
        }
    }
    summary.model = headline_model(tokens_by_model);
    Ok(summary)
}

/// Junie routes, detects languages, and summarizes with small helper models
/// in the same main agent. The model that processes the most tokens is the
/// model that does the session's work, so it becomes the session model. A
/// tie goes to the first model name in sort order.
fn headline_model(tokens_by_model: BTreeMap<String, u64>) -> Option<String> {
    let mut best: Option<(String, u64)> = None;
    for (model, tokens) in tokens_by_model {
        if best.as_ref().is_none_or(|(_, most)| tokens > *most) {
            best = Some((model, tokens));
        }
    }
    best.map(|(model, _)| model)
}

enum Record {
    Inert,
    Unrecognized,
    Usage,
}

fn classify(value: &Value) -> Record {
    let Some(kind) = value.get("kind").and_then(Value::as_str) else {
        return Record::Unrecognized;
    };
    if kind != AGENT_EVENT_KIND {
        return if INERT_KINDS.contains(&kind) {
            Record::Inert
        } else {
            Record::Unrecognized
        };
    }
    let Some(agent_event) = value.pointer("/event/agentEvent") else {
        return Record::Unrecognized;
    };
    match agent_event.get("kind").and_then(Value::as_str) {
        Some(USAGE_KIND) => Record::Usage,
        // Agent UI blocks change often and carry no usage. An unknown block
        // that holds `modelUsage` is not safe to ignore.
        Some(_) if agent_event.get("modelUsage").is_none() => Record::Inert,
        _ => Record::Unrecognized,
    }
}

/// Returns one event per `modelUsage` entry. Returns `None` when any required
/// field is missing or invalid, so that a damaged record adds no partial usage.
fn usage_events(value: &Value) -> Option<Vec<NormalizedEvent>> {
    let ts_ms = value
        .get("timestampMs")
        .and_then(Value::as_i64)
        .filter(|ts| *ts > 0)?;
    let agent_event = value.pointer("/event/agentEvent")?;
    let entries = agent_event
        .get("modelUsage")
        .and_then(Value::as_array)
        .filter(|entries| !entries.is_empty())?;
    let mut events = Vec::with_capacity(entries.len());
    for entry in entries {
        let model = entry
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())?;
        let mut event = NormalizedEvent::new(Role::Assistant);
        event.ts_ms = Some(ts_ms);
        event.usage_ts_ms = Some(ts_ms);
        event.model = Some(model.to_owned());
        event.usage = Usage {
            input_tokens: metric(entry, "inputTokens")?,
            output_tokens: metric(entry, "outputTokens")?,
            cache_read_tokens: metric(entry, "cacheInputTokens")?,
            cache_creation_tokens: metric(entry, "cacheCreateTokens")?,
            cache_creation_1h_tokens: 0,
        };
        events.push(event);
    }
    Some(events)
}

fn metric(entry: &Value, key: &str) -> Option<u64> {
    entry.get(key).and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str =
        include_str!("../../../tests/fixtures/junie_characterization/events.jsonl");

    fn input(content: &str) -> SessionInput {
        SessionInput {
            agent: "junie".to_owned(),
            session_id: "session-261001-090000-syn1".to_owned(),
            source: RawSource::Jsonl(content.to_owned()),
            source_format: SourceFormat::JunieEventsJsonl,
            fork_parent_session_id: None,
        }
    }

    fn read(content: &str) -> (NormalizedSession, Vec<PartialReason>) {
        let mut collector = SessionCollector::new("junie", "session-261001-090000-syn1");
        JunieSessionReader
            .visit(&input(content), &mut collector)
            .unwrap();
        let reasons = collector.partial_reasons().iter().copied().collect();
        (collector.into_session().unwrap(), reasons)
    }

    #[test]
    fn usage_records_become_assistant_events_with_disjoint_token_classes() {
        let (session, reasons) = read(FIXTURE);
        assert!(reasons.is_empty(), "{reasons:?}");
        let models: Vec<_> = session
            .events
            .iter()
            .map(|event| event.model.as_deref().unwrap())
            .collect();
        assert_eq!(
            models,
            [
                "gpt-4.1-mini-2025-04-14",
                "gpt-5.4-nano",
                "gemini-3.7-flash",
                "gpt-5.4-nano"
            ]
        );
        let main = &session.events[2];
        assert_eq!(main.role, Role::Assistant);
        assert_eq!(main.ts_ms, Some(1_790_845_230_000));
        assert_eq!(main.usage.input_tokens, 1_200);
        assert_eq!(main.usage.cache_read_tokens, 9_000);
        assert_eq!(main.usage.cache_creation_tokens, 300);
        assert_eq!(main.usage.output_tokens, 450);
        assert_eq!(session.model.as_deref(), Some("gemini-3.7-flash"));
    }

    #[test]
    fn the_session_model_is_the_model_with_the_most_tokens() {
        let tokens = |pairs: &[(&str, u64)]| {
            pairs
                .iter()
                .map(|(model, tokens)| ((*model).to_owned(), *tokens))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(
            headline_model(tokens(&[("helper", 900), ("main", 9_000)])).as_deref(),
            Some("main")
        );
        assert_eq!(
            headline_model(tokens(&[("b", 5), ("a", 5)])).as_deref(),
            Some("a")
        );
        assert_eq!(headline_model(BTreeMap::new()), None);
    }

    #[test]
    fn unknown_kinds_and_usage_bearing_blocks_make_the_session_partial() {
        let content = format!(
            "{FIXTURE}{}\n{}\n",
            r#"{"kind":"FutureSessionEvent","timestampMs":1790845300000}"#,
            r#"{"kind":"SessionA2uxEvent","taskId":"task-1","timestampMs":1790845300001,"event":{"state":"IN_PROGRESS","agentEvent":{"kind":"FutureUsageEvent","modelUsage":[]}}}"#,
        );
        let (session, reasons) = read(&content);
        assert_eq!(session.events.len(), 4);
        assert_eq!(reasons, [PartialReason::UnrecognizedRecordType]);
    }

    #[test]
    fn damaged_usage_records_add_no_tokens() {
        for damaged in [
            r#"{"kind":"SessionA2uxEvent","taskId":"t","event":{"state":"IN_PROGRESS","agentEvent":{"kind":"LlmResponseMetadataEvent","agent":{"kind":"MainAgent"},"modelUsage":[{"model":"m","inputTokens":1,"cacheInputTokens":0,"cacheCreateTokens":0,"outputTokens":1}]}}}"#,
            r#"{"kind":"SessionA2uxEvent","taskId":"t","timestampMs":1790845300000,"event":{"state":"IN_PROGRESS","agentEvent":{"kind":"LlmResponseMetadataEvent","agent":{"kind":"MainAgent"},"modelUsage":[]}}}"#,
            r#"{"kind":"SessionA2uxEvent","taskId":"t","timestampMs":1790845300000,"event":{"state":"IN_PROGRESS","agentEvent":{"kind":"LlmResponseMetadataEvent","agent":{"kind":"MainAgent"},"modelUsage":[{"model":"m","inputTokens":-1,"cacheInputTokens":0,"cacheCreateTokens":0,"outputTokens":1}]}}}"#,
            r#"{"kind":"SessionA2uxEvent","taskId":"t","timestampMs":1790845300000,"event":{"state":"IN_PROGRESS","agentEvent":{"kind":"LlmResponseMetadataEvent","agent":{"kind":"MainAgent"},"modelUsage":[{"model":"ok","inputTokens":1,"cacheInputTokens":0,"cacheCreateTokens":0,"outputTokens":1},{"model":"","inputTokens":1,"cacheInputTokens":0,"cacheCreateTokens":0,"outputTokens":1}]}}}"#,
            "{not json",
        ] {
            let (session, reasons) = read(&format!("{FIXTURE}{damaged}\n"));
            assert_eq!(session.events.len(), 4, "{damaged}");
            assert_eq!(reasons, [PartialReason::MalformedRecord], "{damaged}");
        }
    }

    #[test]
    fn other_source_formats_fail_closed() {
        let other = SessionInput {
            source_format: SourceFormat::CopilotCliJsonl,
            ..input(FIXTURE)
        };
        assert_eq!(
            JunieSessionReader.capabilities(&other),
            SourceCapabilities::uncharacterized(SourceFormat::CopilotCliJsonl)
        );
        let mut collector = SessionCollector::new("junie", "s");
        assert!(JunieSessionReader.visit(&other, &mut collector).is_err());
    }
}
