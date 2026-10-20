#[path = "../src/analysis/vendors/opencode_v2_reader.rs"]
mod opencode_v2_reader;
#[path = "../src/analysis/opencode_v2_schema.rs"]
mod opencode_v2_schema;
#[path = "../src/analysis/vendors/opencode_v2_snapshot.rs"]
mod opencode_v2_snapshot;

mod analysis {
    pub(crate) use crate::opencode_v2_schema;
    pub use antiburn_local::analysis::*;
}

use std::path::PathBuf;

use antiburn_local::analysis::{
    ContentKind, NormalizedEvent, NormalizedRecord, RawSource, RecordSink, SessionInput,
    SessionReader, SessionSummary, SourceFormat, ToolResultObservation, TurnContent, VisitOutcome,
    reader_for_input,
};
use opencode_v2_reader::OpenCodeV2SessionReader;
use rusqlite::{Connection, params};
use tempfile::tempdir;

#[derive(Default)]
struct CaptureSink {
    events: Vec<NormalizedEvent>,
    contents: Vec<TurnContent>,
    tool_results: Vec<ToolResultObservation>,
    summary: Option<SessionSummary>,
    user_messages: usize,
    tool_result_gaps: usize,
    unusable: Vec<antiburn_local::analysis::PartialReason>,
}

impl RecordSink for CaptureSink {
    fn record(&mut self, record: NormalizedRecord) {
        match record {
            NormalizedRecord::MetricsEvent(event) => self.events.push(*event),
            NormalizedRecord::TurnContent(content) => self.contents.push(*content),
            NormalizedRecord::ToolResult(result) => self.tool_results.push(*result),
            NormalizedRecord::Unusable(reason) => self.unusable.push(reason),
            NormalizedRecord::Observation(_) => {}
        }
    }

    fn user_message(&mut self, _: antiburn_local::analysis::UserMessage) {
        self.user_messages += 1;
    }

    fn tool_result_gap(&mut self) {
        self.tool_result_gaps += 1;
    }

    fn finish(&mut self, summary: SessionSummary) {
        self.summary = Some(summary);
    }
}

fn fixture() -> (tempfile::TempDir, PathBuf, SessionInput) {
    let directory = tempdir().unwrap();
    let path = directory.path().join("opencode-v2.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE session_v2 (
                  id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT NOT NULL DEFAULT '/repo',
                  title TEXT, time_created INTEGER NOT NULL DEFAULT 0,
                  time_updated INTEGER NOT NULL DEFAULT 0
              );
              CREATE TABLE session_message (
                  id TEXT PRIMARY KEY,
                  session_id TEXT NOT NULL,
                  type TEXT NOT NULL,
                  seq INTEGER NOT NULL,
                  time_created INTEGER NOT NULL DEFAULT 0,
                  time_updated INTEGER NOT NULL DEFAULT 0,
                  data TEXT NOT NULL
              );
              CREATE UNIQUE INDEX session_message_session_seq_idx
                  ON session_message(session_id, seq);
              INSERT INTO session_v2 (id) VALUES ('ses_v2');",
        )
        .unwrap();
    let input = SessionInput {
        agent: "opencode".to_owned(),
        session_id: "ses_v2".to_owned(),
        source: RawSource::Sqlite(path.clone()),
        source_format: SourceFormat::OpenCodeSqliteV2,
        fork_parent_session_id: None,
    };
    (directory, path, input)
}

fn insert(path: &std::path::Path, id: &str, message_type: &str, seq: i64, data: &str) {
    Connection::open(path)
        .unwrap()
        .execute(
            "INSERT INTO session_message (id, session_id, type, seq, data)
             VALUES (?1, 'ses_v2', ?2, ?3, ?4)",
            params![id, message_type, seq, data],
        )
        .unwrap();
}

fn assistant_message() -> &'static str {
    r#"{
        "time":{"created":1735689600123},
        "agent":"build",
        "model":{"id":"model-v2","providerID":"provider-v2"},
        "tokens":{"input":101,"output":17,"reasoning":5,"cache":{"read":31,"write":13}},
        "content":[
            {"type":"text","text":"public answer"},
            {"type":"reasoning","text":"private chain of thought"},
            {"type":"tool","id":"call-1","name":"Read","state":{
                "status":"completed",
                "input":{"filePath":"src/main.rs"},
                "content":[{"type":"text","text":"public tool output"}]
            }}
        ]
    }"#
}

#[test]
fn maps_proven_assistant_usage_identity_timestamps_and_public_content() {
    let (_directory, path, input) = fixture();
    insert(&path, "msg_assistant", "assistant", 0, assistant_message());
    let mut sink = CaptureSink::default();

    let outcome = OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();

    assert_eq!(outcome, VisitOutcome::Unvalidated);
    assert_eq!(sink.events.len(), 1);
    let event = &sink.events[0];
    assert_eq!(event.ts_ms, Some(1_735_689_600_123));
    assert_eq!(event.uuid.as_deref(), Some("msg_assistant"));
    assert_eq!(event.thread_id.as_deref(), Some("ses_v2"));
    assert_eq!(event.model.as_deref(), Some("model-v2"));
    assert_eq!(event.provider.as_deref(), Some("provider-v2"));
    assert_eq!(event.usage.input_tokens, 101);
    assert_eq!(event.usage.output_tokens, 22);
    assert_eq!(event.usage.cache_read_tokens, 31);
    assert_eq!(event.usage.cache_creation_tokens, 13);
    assert_eq!(event.tools.len(), 1);
    assert_eq!(event.tools[0].name, "Read");

    let captured = &sink.contents[0].parts;
    assert_eq!(captured.len(), 3);
    assert_eq!(captured[0].kind, ContentKind::AssistantText);
    assert_eq!(captured[0].text, "public answer");
    assert_eq!(captured[1].kind, ContentKind::ToolInput);
    assert_eq!(captured[1].tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(captured[2].kind, ContentKind::ToolResult);
    assert_eq!(captured[2].text, "public tool output");
    assert!(
        captured
            .iter()
            .all(|part| !part.text.contains("private chain of thought"))
    );
    assert!(sink.tool_results.is_empty());
    assert_eq!(sink.user_messages, 0);
    assert_eq!(sink.tool_result_gaps, 1);
    assert_eq!(
        sink.summary.as_ref().unwrap().model.as_deref(),
        Some("model-v2")
    );
    assert!(sink.summary.as_ref().unwrap().cache_write_tokens_available);
}

#[test]
fn normalize_matches_metrics_emitted_by_visit() {
    let (_directory, path, input) = fixture();
    insert(&path, "msg_assistant", "assistant", 0, assistant_message());
    let mut sink = CaptureSink::default();

    OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();
    let normalized = OpenCodeV2SessionReader.normalize(&input).unwrap();

    assert_eq!(normalized.events, sink.events);
    assert_eq!(normalized.session_id, "ses_v2");
    assert_eq!(normalized.agent, "opencode");
}

#[test]
fn unknown_rows_report_partial_evidence() {
    let (_directory, path, input) = fixture();
    insert(&path, "msg_before", "user", 0, r#"{"time":{"created":1}}"#);
    insert(&path, "msg_unknown", "future-type", 1, "{}");
    let mut sink = CaptureSink::default();

    OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();

    assert_eq!(sink.events.len(), 1);
    assert!(!sink.unusable.is_empty());
    assert!(
        sink.summary
            .as_ref()
            .unwrap()
            .coverage_gaps
            .contains(&antiburn_local::analysis::PartialReason::MalformedRecord)
    );
    assert_eq!(sink.tool_result_gaps, 1);
}

#[test]
fn malformed_json_reports_partial_evidence() {
    let (_directory, path, input) = fixture();
    insert(&path, "msg_bad", "assistant", 0, "{");
    let mut sink = CaptureSink::default();

    OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();

    assert!(sink.events.is_empty());
    assert!(
        sink.summary
            .as_ref()
            .unwrap()
            .coverage_gaps
            .contains(&antiburn_local::analysis::PartialReason::MalformedRecord)
    );
}

#[test]
fn user_text_does_not_become_captured_content_or_a_complete_request_fact() {
    let (_directory, path, input) = fixture();
    insert(
        &path,
        "msg_user",
        "user",
        0,
        r#"{"time":{"created":1},"text":"private user text"}"#,
    );
    let mut sink = CaptureSink::default();

    OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();

    assert_eq!(sink.events.len(), 1);
    assert_eq!(sink.events[0].role, antiburn_local::analysis::Role::User);
    assert!(sink.contents.is_empty());
    assert_eq!(sink.user_messages, 0);
}

#[test]
fn oversized_rows_mark_the_session_partial_and_do_not_emit_facts() {
    let (_directory, path, input) = fixture();
    let oversized = format!(
        "{{\"time\":{{\"created\":1}},\"text\":\"{}\"}}",
        "x".repeat(8 * 1024 * 1024)
    );
    insert(&path, "msg_large", "assistant", 0, &oversized);
    insert(&path, "msg_after", "user", 1, r#"{"time":{"created":2}}"#);
    let mut sink = CaptureSink::default();

    OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();

    assert_eq!(sink.events.len(), 1);
    assert_eq!(sink.events[0].role, antiburn_local::analysis::Role::User);
    assert!(
        sink.unusable
            .contains(&antiburn_local::analysis::PartialReason::Oversized)
    );
    assert!(
        sink.summary
            .as_ref()
            .unwrap()
            .coverage_gaps
            .contains(&antiburn_local::analysis::PartialReason::Oversized)
    );
}

#[test]
fn transfer_format_uses_the_v2_reader_and_checks_session_identity() {
    let (_directory, _path, mut input) = fixture();
    input.source_format = SourceFormat::OpenCodeV2SessionExport;
    input.source = RawSource::Jsonl(
        serde_json::json!({
            "info": {"id":"ses_v2","location":{"directory":"/repo"}},
            "messages": [{
                "id":"msg_export","type":"assistant",
                    "time":{"created":1735689600123_i64},"agent":"build",
                "model":{"id":"model-v2","providerID":"provider-v2"},
                "tokens":{"input":9,"output":3,"reasoning":2,"cache":{"read":0,"write":0}},
                "content":[]
            }]
        })
        .to_string(),
    );

    let reader = reader_for_input(&input);
    assert_eq!(
        reader.capabilities(&input).source_format,
        SourceFormat::OpenCodeV2SessionExport
    );
    let normalized = reader.normalize(&input).unwrap();
    assert_eq!(normalized.events.len(), 1);
    assert_eq!(normalized.events[0].usage.output_tokens, 5);

    input.session_id = "ses_other".to_owned();
    assert!(reader.normalize(&input).unwrap().events.is_empty());
}

#[test]
fn text_tool_content_is_kept_and_non_text_content_marks_partial() {
    let (_directory, path, input) = fixture();
    let mut value: serde_json::Value = serde_json::from_str(assistant_message()).unwrap();
    value["content"][2]["state"]["content"] = serde_json::json!([
        {"type":"text", "text":"safe output"},
        {"type":"file", "uri":"file:///private/output.png", "mime":"image/png"},
        {"type":"image", "data":"private-image-data"}
    ]);
    insert(
        &path,
        "msg_assistant",
        "assistant",
        0,
        &serde_json::to_string(&value).unwrap(),
    );
    let mut sink = CaptureSink::default();

    OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();

    let parts = &sink.contents[0].parts;
    assert_eq!(parts.last().unwrap().text, "safe output");
    assert!(
        !parts
            .iter()
            .any(|part| part.text.contains("private-image-data"))
    );
    assert!(
        sink.unusable
            .contains(&antiburn_local::analysis::PartialReason::UnrecognizedRecordType)
    );
    assert!(
        sink.summary
            .as_ref()
            .unwrap()
            .coverage_gaps
            .contains(&antiburn_local::analysis::PartialReason::UnrecognizedRecordType)
    );
}

#[test]
fn content_capture_caps_part_count_and_aggregate_bytes() {
    let (_directory, path, input) = fixture();
    let mut too_many: serde_json::Value = serde_json::from_str(assistant_message()).unwrap();
    too_many["content"] = serde_json::Value::Array(
        (0..1025)
            .map(|_| serde_json::json!({"type":"text", "text":"x"}))
            .collect(),
    );
    insert(
        &path,
        "msg_many",
        "assistant",
        0,
        &serde_json::to_string(&too_many).unwrap(),
    );
    let mut sink = CaptureSink::default();

    OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();

    assert_eq!(sink.contents[0].parts.len(), 1024);
    assert!(
        sink.unusable
            .contains(&antiburn_local::analysis::PartialReason::Oversized)
    );

    let (_directory, path, input) = fixture();
    let mut too_large: serde_json::Value = serde_json::from_str(assistant_message()).unwrap();
    too_large["content"] = serde_json::json!([
        {"type":"text", "text":"a".repeat(200_000)},
        {"type":"text", "text":"b".repeat(200_000)}
    ]);
    insert(
        &path,
        "msg_large_content",
        "assistant",
        0,
        &serde_json::to_string(&too_large).unwrap(),
    );
    let mut sink = CaptureSink::default();

    OpenCodeV2SessionReader.visit(&input, &mut sink).unwrap();

    let parts = &sink.contents[0].parts;
    assert_eq!(
        parts.iter().map(|part| part.text.len()).sum::<usize>(),
        256 * 1024
    );
    assert!(parts[1].truncated);
    assert!(
        sink.unusable
            .contains(&antiburn_local::analysis::PartialReason::Oversized)
    );
}

#[test]
fn rejects_a_source_format_or_container_mismatch() {
    let (_directory, _path, mut input) = fixture();
    input.source_format = SourceFormat::OpenCodeSqliteV1;
    assert!(OpenCodeV2SessionReader.normalize(&input).is_err());

    input.source_format = SourceFormat::OpenCodeSqliteV2;
    input.source = RawSource::Jsonl(String::new());
    assert!(OpenCodeV2SessionReader.normalize(&input).is_err());
}
