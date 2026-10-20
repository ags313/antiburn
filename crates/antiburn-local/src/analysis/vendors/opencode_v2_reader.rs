//! Normalizes the bounded facts in OpenCode V2 snapshot messages.

use anyhow::Context;
use serde_json::Value;

use super::opencode_v2_snapshot::{SnapshotMessage, visit_snapshot};
use crate::analysis::{
    ContentKind, ContentPart, EVIDENCE_STRING_CAP, MAX_CONTENT_PART_BYTES, MAX_USER_MESSAGE_PARTS,
    NormalizedRecord, NormalizedSession, PartialReason, RawSource, RecordSink, SessionCollector,
    SessionInput, SessionReader, SessionSummary, SourceCapabilities, SourceFormat, ToolCall,
    ToolCategory, TurnContent, Usage, VisitOutcome,
};
use crate::analysis::{NormalizedEvent, Role};

const MAX_TRANSFER_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRANSFER_MESSAGES: usize = 100_000;

pub struct OpenCodeV2SessionReader;

impl SessionReader for OpenCodeV2SessionReader {
    fn agent(&self) -> &'static str {
        "opencode"
    }

    fn capabilities(&self, input: &SessionInput) -> SourceCapabilities {
        SourceCapabilities::uncharacterized(input.source_format)
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
        visit_with_cancel(input, &|| false, sink)
    }
}

fn visit_with_cancel(
    input: &SessionInput,
    cancel: &dyn Fn() -> bool,
    sink: &mut dyn RecordSink,
) -> anyhow::Result<VisitOutcome> {
    let mut state = ReaderState::default();
    match (input.source_format, &input.source) {
        (SourceFormat::OpenCodeSqliteV2, RawSource::Sqlite(path)) => {
            let read = visit_snapshot(
                path,
                input.source_format,
                &input.session_id,
                cancel,
                &mut |message| {
                    state.observe(message, sink);
                    Ok(())
                },
            );
            let snapshot = match read {
                Ok(summary) => summary,
                Err(error) if is_malformed_snapshot_error(&error) => {
                    state.partial(PartialReason::MalformedRecord, sink);
                    sink.tool_result_gap();
                    sink.finish(state.finish());
                    return Ok(VisitOutcome::Unvalidated);
                }
                Err(error) => return Err(error).context("reading OpenCode V2 session snapshot"),
            };
            if snapshot.is_partial() {
                for _ in 0..snapshot.oversized_rows {
                    state.partial(PartialReason::Oversized, sink);
                }
                if snapshot.capacity_limit_reached {
                    state.partial(PartialReason::CapacityExceeded, sink);
                }
            }
        }
        (SourceFormat::OpenCodeV2SessionExport, RawSource::Jsonl(content)) => {
            visit_transfer(content, input, cancel, &mut state, sink)?;
        }
        _ => anyhow::bail!(
            "OpenCode V2 source format {:?} does not match its source",
            input.source_format
        ),
    }
    let summary = state.finish();
    sink.tool_result_gap();
    sink.finish(summary);
    Ok(VisitOutcome::Unvalidated)
}

fn visit_transfer(
    content: &str,
    input: &SessionInput,
    cancel: &dyn Fn() -> bool,
    state: &mut ReaderState,
    sink: &mut dyn RecordSink,
) -> anyhow::Result<()> {
    if content.len() > MAX_TRANSFER_BYTES {
        state.partial(PartialReason::Oversized, sink);
        return Ok(());
    }
    let transfer: Value = match serde_json::from_str(content) {
        Ok(value) => value,
        Err(_) => {
            state.partial(PartialReason::MalformedRecord, sink);
            return Ok(());
        }
    };
    let info = transfer.get("info").and_then(Value::as_object);
    let messages = transfer.get("messages").and_then(Value::as_array);
    if info.and_then(|info| info.get("id")).and_then(Value::as_str)
        != Some(input.session_id.as_str())
    {
        state.partial(PartialReason::AttributionIncomplete, sink);
        return Ok(());
    }
    let Some(messages) = messages else {
        state.partial(PartialReason::MalformedRecord, sink);
        return Ok(());
    };
    for (index, message) in messages.iter().take(MAX_TRANSFER_MESSAGES).enumerate() {
        if cancel() {
            anyhow::bail!("OpenCode V2 transfer read was cancelled");
        }
        let Some(object) = message.as_object() else {
            state.partial(PartialReason::MalformedRecord, sink);
            continue;
        };
        state.observe(
            SnapshotMessage {
                session_id: input.session_id.clone(),
                seq: index as i64,
                value: Value::Object(object.clone()),
            },
            sink,
        );
    }
    if messages.len() > MAX_TRANSFER_MESSAGES {
        state.partial(PartialReason::Oversized, sink);
    }
    Ok(())
}

struct ReaderState {
    model: Option<String>,
    assistant_messages: u64,
    cache_write_complete: bool,
    coverage_gaps: Vec<PartialReason>,
}

impl Default for ReaderState {
    fn default() -> Self {
        Self {
            model: None,
            assistant_messages: 0,
            cache_write_complete: true,
            coverage_gaps: Vec::new(),
        }
    }
}

impl ReaderState {
    fn observe(&mut self, message: SnapshotMessage, sink: &mut dyn RecordSink) {
        let value = &message.value;
        let Some(message_id) = value.get("id").and_then(Value::as_str).map(str::to_owned) else {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        };
        if message_id.is_empty() || message_id.len() > EVIDENCE_STRING_CAP {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        }
        let Some(message_type) = value.get("type").and_then(Value::as_str) else {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        };
        match message_type {
            "assistant" => self.observe_assistant(message, &message_id, sink),
            "user" => self.observe_user(message, &message_id, sink),
            // These rows are valid source records, but they do not define a
            // user or assistant turn for this bounded adapter.
            "agent-switched" | "model-switched" | "location-switched" | "synthetic" | "system"
            | "skill" | "shell" | "idle" => {}
            "compaction" => self.cache_write_complete = false,
            _ => self.partial(PartialReason::MalformedRecord, sink),
        }
    }

    fn observe_user(
        &mut self,
        message: SnapshotMessage,
        message_id: &str,
        sink: &mut dyn RecordSink,
    ) {
        let value = &message.value;
        let Some(mut event) = base_event(value, Role::User) else {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        };
        event.uuid = Some(message_id.to_owned());
        event.thread_id = Some(message.session_id);
        sink.record(NormalizedRecord::MetricsEvent(Box::new(event)));
    }

    fn observe_assistant(
        &mut self,
        message: SnapshotMessage,
        message_id: &str,
        sink: &mut dyn RecordSink,
    ) {
        let value = &message.value;
        let Some(mut event) = base_event(value, Role::Assistant) else {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        };
        let Some(model) = value.get("model").and_then(Value::as_object) else {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        };
        let Some(model_id) = model.get("id").and_then(Value::as_str) else {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        };
        let Some(provider_id) = model.get("providerID").and_then(Value::as_str) else {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        };
        if model_id.is_empty()
            || provider_id.is_empty()
            || model_id.len() > EVIDENCE_STRING_CAP
            || provider_id.len() > EVIDENCE_STRING_CAP
        {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        }
        event.model = Some(model_id.to_owned());
        event.provider = Some(provider_id.to_owned());
        event.uuid = Some(message_id.to_owned());
        event.thread_id = Some(message.session_id);

        match value.get("tokens") {
            Some(tokens) => match parse_usage(tokens) {
                Some(usage) => {
                    event.usage = usage;
                }
                None => {
                    self.partial(PartialReason::MalformedRecord, sink);
                    self.cache_write_complete = false;
                }
            },
            None => self.cache_write_complete = false,
        };

        let Some(contents) = value.get("content").and_then(Value::as_array) else {
            self.partial(PartialReason::MalformedRecord, sink);
            return;
        };
        let mut captured = CapturedContent::default();
        for content in contents {
            match content.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if !capture_assistant_text(content, &mut captured) {
                        self.partial(PartialReason::MalformedRecord, sink);
                    }
                }
                Some("reasoning") => {
                    if content.get("text").and_then(Value::as_str).is_none() {
                        self.partial(PartialReason::MalformedRecord, sink);
                    }
                    event.has_thinking = true;
                }
                Some("tool") => {
                    if !capture_tool(content, &mut event, &mut captured) {
                        self.partial(PartialReason::MalformedRecord, sink);
                    }
                }
                _ => self.partial(PartialReason::MalformedRecord, sink),
            }
        }
        if captured.omitted_non_text {
            self.partial(PartialReason::UnrecognizedRecordType, sink);
        }
        if captured.oversized {
            self.partial(PartialReason::Oversized, sink);
        }

        self.assistant_messages = self.assistant_messages.saturating_add(1);
        if self.model.is_none() {
            self.model = Some(model_id.to_owned());
        }
        sink.record(NormalizedRecord::MetricsEvent(Box::new(event)));
        if !captured.parts.is_empty() {
            sink.record(NormalizedRecord::TurnContent(Box::new(TurnContent {
                parts: captured.parts,
            })));
        }
    }

    fn partial(&mut self, reason: PartialReason, sink: &mut dyn RecordSink) {
        self.cache_write_complete = false;
        if !self.coverage_gaps.contains(&reason) {
            self.coverage_gaps.push(reason);
        }
        sink.record(NormalizedRecord::Unusable(reason));
    }

    fn finish(&self) -> SessionSummary {
        SessionSummary {
            cache_write_tokens_available: self.assistant_messages > 0 && self.cache_write_complete,
            model: self.model.clone(),
            coverage_gaps: self.coverage_gaps.clone(),
            ..SessionSummary::default()
        }
    }
}

fn base_event(value: &Value, role: Role) -> Option<NormalizedEvent> {
    let created = value.pointer("/time/created")?.as_i64()?;
    let mut event = NormalizedEvent::new(role);
    event.ts_ms = Some(created);
    Some(event)
}

fn parse_usage(value: &Value) -> Option<Usage> {
    let cache = value.get("cache")?;
    Some(Usage {
        input_tokens: value.get("input")?.as_u64()?,
        output_tokens: value
            .get("output")?
            .as_u64()?
            .saturating_add(value.get("reasoning")?.as_u64()?),
        cache_read_tokens: cache.get("read")?.as_u64()?,
        cache_creation_tokens: cache.get("write")?.as_u64()?,
        cache_creation_1h_tokens: 0,
    })
}

#[derive(Default)]
struct CapturedContent {
    parts: Vec<ContentPart>,
    retained_bytes: usize,
    oversized: bool,
    omitted_non_text: bool,
}

impl CapturedContent {
    fn push(
        &mut self,
        kind: ContentKind,
        text: &str,
        tool_name: Option<&str>,
        call_id: Option<&str>,
    ) {
        if text.is_empty() {
            return;
        }
        if self.parts.len() >= MAX_USER_MESSAGE_PARTS {
            self.oversized = true;
            return;
        }
        let remaining = MAX_CONTENT_PART_BYTES.saturating_sub(self.retained_bytes);
        if remaining == 0 {
            self.oversized = true;
            return;
        }
        let limit = text.len().min(MAX_CONTENT_PART_BYTES).min(remaining);
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end < text.len() {
            self.oversized = true;
        }
        let mut part = ContentPart::new(kind, &text[..end]);
        part.truncated = end < text.len();
        if let Some(tool_name) = tool_name {
            part = part.with_tool_identity(Some(tool_name.to_owned()), call_id.map(str::to_owned));
        }
        self.retained_bytes = self.retained_bytes.saturating_add(part.text.len());
        self.parts.push(part);
    }
}

fn capture_assistant_text(value: &Value, output: &mut CapturedContent) -> bool {
    let Some(text) = value.get("text").and_then(Value::as_str) else {
        return false;
    };
    output.push(ContentKind::AssistantText, text, None, None);
    true
}

fn capture_tool(value: &Value, event: &mut NormalizedEvent, output: &mut CapturedContent) -> bool {
    let Some(name) = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= EVIDENCE_STRING_CAP)
    else {
        return false;
    };
    let Some(call_id) = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= EVIDENCE_STRING_CAP)
        .map(str::to_owned)
    else {
        return false;
    };
    let Some(state) = value.get("state").and_then(Value::as_object) else {
        return false;
    };
    let status = state.get("status").and_then(Value::as_str);
    if !matches!(
        status,
        Some("streaming" | "running" | "completed" | "error")
    ) {
        return false;
    }

    let Some(input) = state.get("input") else {
        return false;
    };
    if (status == Some("streaming") && !input.is_string())
        || (status != Some("streaming") && !input.is_object())
    {
        return false;
    }
    let contents = state.get("content").and_then(Value::as_array);
    if (status == Some("completed") || state.contains_key("content")) && contents.is_none() {
        return false;
    }
    event.tools.push(ToolCall {
        name: name.to_owned(),
        category: ToolCategory::from_tool_name(name),
        detail: None,
    });
    let text = match input {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    output.push(ContentKind::ToolInput, &text, Some(name), Some(&call_id));
    if let Some(contents) = contents {
        for content in contents {
            match content.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let Some(text) = content.get("text").and_then(Value::as_str) else {
                        return false;
                    };
                    output.push(ContentKind::ToolResult, text, Some(name), Some(&call_id));
                }
                _ => output.omitted_non_text = true,
            }
        }
    }
    true
}

fn is_malformed_snapshot_error(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}");
    [
        "unknown message type",
        "malformed message JSON",
        "message data is not a JSON object",
        "invalid message id",
        "invalid message session id",
        "invalid message type",
        "invalid message data",
        "invalid message sequence",
        "invalid message row size",
        "duplicate or out-of-order message sequence",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}
