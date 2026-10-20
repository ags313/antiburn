//! Bounded join core for the pinned OpenCode V2 durable event payloads.
//!
//! This module deliberately returns an intermediate observation. It does not
//! prove that a desktop event belongs to a server-side session database row.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

pub const MAX_RETAINED_CALLS: usize = 256;
pub const MAX_RETAINED_USERS: usize = 128;
pub const MAX_STRING_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gap {
    MissingSequence,
    SequenceDiscontinuity,
    ContinuityLost,
    MissingJoin,
    ConflictingDuplicate,
    CapacityExceeded,
    UnknownShape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalStatus {
    Success,
    Failed,
}

/// A conservative terminal tool observation. `request_text` comes only from a
/// delivered `message.updated` user message and its text parts.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinedToolResult {
    pub session_id: String,
    pub assistant_message_id: String,
    pub call_id: String,
    pub event_id: String,
    pub aggregate_id: String,
    pub sequence: u64,
    pub created: Option<f64>,
    pub tool_name: String,
    pub input: Option<Value>,
    pub request_message_id: Option<String>,
    pub request_text: Option<String>,
    pub status: TerminalStatus,
    /// Text content is populated only when every content item is text.
    pub content: Option<String>,
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Clone)]
struct UserRequest {
    sequence: u64,
    message_id: String,
    text: Option<String>,
}

#[derive(Debug, Clone)]
enum PendingInbox {
    User(UserRequest),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StepContext {
    request_message_id: Option<String>,
    request_text: Option<String>,
}

#[derive(Debug, Clone)]
struct Call {
    name: Option<String>,
    input: Option<Value>,
    request_message_id: Option<String>,
    request_text: Option<String>,
    terminal: Option<JoinedToolResult>,
}

struct TerminalEvent<'a> {
    event: &'a Value,
    event_id: &'a str,
    aggregate_id: &'a str,
    sequence: u64,
    session_id: &'a str,
    data: &'a serde_json::Map<String, Value>,
    status: TerminalStatus,
}

/// Joins the durable V2 event envelope by native session/message/call IDs.
/// Feed events in delivery order. Sequence numbers are authoritative for
/// ordering; timestamps are retained for display only.
#[derive(Debug, Default)]
pub struct OpenCodeV2EventJoiner {
    session_id: Option<String>,
    last_sequence: Option<u64>,
    seen_event_ids: HashSet<String>,
    calls: BTreeMap<(String, String), Call>,
    pending_inbox: HashMap<String, PendingInbox>,
    users: Vec<UserRequest>,
    steps: HashMap<String, StepContext>,
    gaps: Vec<Gap>,
}

impl OpenCodeV2EventJoiner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe one event. Returns a result only for the first terminal event
    /// that can be joined to a known call. Unknown or unsafe input is retained
    /// as an explicit gap and never guessed into a result.
    pub fn observe(&mut self, event: &Value) -> Option<JoinedToolResult> {
        let (Some(event_id), Some(event_type), Some(data)) = (
            string(event, "id"),
            string(event, "type"),
            event.get("data").and_then(Value::as_object),
        ) else {
            self.gap(Gap::UnknownShape);
            return None;
        };
        if event_id.len() > MAX_STRING_BYTES || self.seen_event_ids.contains(event_id) {
            return None;
        }
        if self.seen_event_ids.len() >= MAX_RETAINED_CALLS * 8 {
            self.gap(Gap::CapacityExceeded);
            return None;
        }
        self.seen_event_ids.insert(event_id.to_owned());

        let Some(sequence) = event.pointer("/durable/seq").and_then(Value::as_u64) else {
            self.gap(Gap::MissingSequence);
            return None;
        };
        let Some(aggregate_id) = event
            .pointer("/durable/aggregateID")
            .and_then(Value::as_str)
        else {
            self.gap(Gap::UnknownShape);
            return None;
        };
        if aggregate_id.len() > MAX_STRING_BYTES {
            self.gap(Gap::UnknownShape);
            return None;
        }
        if self
            .last_sequence
            .is_some_and(|previous| sequence <= previous)
        {
            self.gap(Gap::SequenceDiscontinuity);
        }
        self.last_sequence = Some(sequence);

        let Some(session_id) = bounded_field(data, "sessionID") else {
            self.gap(Gap::UnknownShape);
            return None;
        };
        if let Some(known) = &self.session_id {
            if known != &session_id || aggregate_id != known {
                self.gap(Gap::UnknownShape);
                return None;
            }
        } else {
            self.session_id = Some(session_id.clone());
            if aggregate_id != session_id {
                self.gap(Gap::UnknownShape);
                return None;
            }
        }

        match event_type {
            "session.updated" => {
                if data
                    .get("info")
                    .and_then(|info| info.get("id"))
                    .and_then(Value::as_str)
                    != Some(session_id.as_str())
                {
                    self.gap(Gap::UnknownShape);
                }
                None
            }
            "session.inbox.enqueued" => {
                self.observe_user_enqueued(sequence, data);
                None
            }
            "session.inbox.delivered" => {
                self.observe_user_delivered(sequence, data);
                None
            }
            "session.inbox.cancelled" => {
                self.observe_inbox_cancelled(data);
                None
            }
            "session.step.started" => {
                self.observe_step_started(sequence, data);
                None
            }
            "session.tool.input.started" => {
                self.observe_tool_started(data);
                None
            }
            "session.tool.input.ended" => {
                self.observe_tool_input_ended(data);
                None
            }
            "session.tool.called" => {
                self.observe_tool_called(sequence, data);
                None
            }
            "session.tool.success" => self.observe_terminal(TerminalEvent {
                event,
                event_id,
                aggregate_id,
                sequence,
                session_id: &session_id,
                data,
                status: TerminalStatus::Success,
            }),
            "session.tool.failed" => self.observe_terminal(TerminalEvent {
                event,
                event_id,
                aggregate_id,
                sequence,
                session_id: &session_id,
                data,
                status: TerminalStatus::Failed,
            }),
            _ => None,
        }
    }

    pub fn gaps(&self) -> &[Gap] {
        &self.gaps
    }

    /// Mark transport continuity loss. Later joined results retain this gap
    /// until the caller creates a new joiner for a separately proven segment.
    pub fn mark_gap(&mut self) {
        self.gap(Gap::ContinuityLost);
    }

    fn observe_step_started(&mut self, sequence: u64, data: &serde_json::Map<String, Value>) {
        let Some(assistant_message_id) = bounded_field(data, "assistantMessageID") else {
            self.gap(Gap::UnknownShape);
            return;
        };
        if self.steps.len() >= MAX_RETAINED_CALLS && !self.steps.contains_key(&assistant_message_id)
        {
            self.gap(Gap::CapacityExceeded);
            return;
        }
        let request = self
            .users
            .iter()
            .filter(|user| user.sequence < sequence)
            .max_by_key(|user| user.sequence)
            .map(|user| StepContext {
                request_message_id: Some(user.message_id.clone()),
                request_text: user.text.clone(),
            });
        if request
            .as_ref()
            .is_none_or(|context| context.request_text.is_none())
        {
            self.gap(Gap::MissingJoin);
        }
        let context = request.unwrap_or(StepContext {
            request_message_id: None,
            request_text: None,
        });
        if let Some(existing) = self.steps.get(&assistant_message_id) {
            if existing != &context {
                self.gap(Gap::ConflictingDuplicate);
            }
            return;
        }
        self.steps.insert(assistant_message_id, context);
    }

    fn observe_user_enqueued(&mut self, sequence: u64, data: &serde_json::Map<String, Value>) {
        let Some(inbox_id) = bounded_field(data, "inboxID") else {
            self.gap(Gap::UnknownShape);
            return;
        };
        if self.pending_inbox.len() + self.users.len() >= MAX_RETAINED_USERS {
            self.gap(Gap::CapacityExceeded);
            return;
        }
        let item = data.get("item").and_then(Value::as_object);
        let pending = match item
            .and_then(|item| item.get("type"))
            .and_then(Value::as_str)
        {
            Some("user") => {
                let payload = item
                    .and_then(|item| item.get("payload"))
                    .and_then(Value::as_object);
                let text = payload
                    .and_then(|payload| payload.get("text"))
                    .and_then(Value::as_str);
                if text.is_none_or(|text| text.len() > MAX_STRING_BYTES) {
                    self.gap(Gap::MissingJoin);
                }
                PendingInbox::User(UserRequest {
                    sequence,
                    message_id: inbox_id.clone(),
                    text: text
                        .filter(|text| text.len() <= MAX_STRING_BYTES)
                        .map(str::to_owned),
                })
            }
            Some("synthetic" | "compaction" | "move") => PendingInbox::Other,
            _ => {
                self.gap(Gap::UnknownShape);
                PendingInbox::Other
            }
        };
        if self.pending_inbox.insert(inbox_id, pending).is_some() {
            self.gap(Gap::ConflictingDuplicate);
        }
    }

    fn observe_user_delivered(&mut self, sequence: u64, data: &serde_json::Map<String, Value>) {
        let Some(message_id) = bounded_field(data, "inboxID") else {
            self.gap(Gap::UnknownShape);
            return;
        };
        let mut request = match self.pending_inbox.remove(&message_id) {
            Some(PendingInbox::User(request)) => request,
            Some(PendingInbox::Other) => return,
            None => {
                self.gap(Gap::MissingJoin);
                return;
            }
        };
        request.sequence = sequence;
        if self.users.len() >= MAX_RETAINED_USERS {
            self.gap(Gap::CapacityExceeded);
            return;
        }
        self.users.push(request);
    }

    fn observe_inbox_cancelled(&mut self, data: &serde_json::Map<String, Value>) {
        let Some(message_id) = bounded_field(data, "inboxID") else {
            self.gap(Gap::UnknownShape);
            return;
        };
        self.pending_inbox.remove(&message_id);
    }

    fn observe_tool_started(&mut self, data: &serde_json::Map<String, Value>) {
        let (Some(message_id), Some(call_id), Some(name)) = (
            bounded_field(data, "assistantMessageID"),
            bounded_field(data, "id"),
            bounded_field(data, "name"),
        ) else {
            self.gap(Gap::UnknownShape);
            return;
        };
        if !self.ensure_call(message_id.clone(), call_id.clone()) {
            return;
        }
        let call = self
            .calls
            .get_mut(&(message_id, call_id))
            .expect("inserted call");
        if call.name.as_ref().is_some_and(|existing| existing != &name) {
            self.gap(Gap::ConflictingDuplicate);
            return;
        }
        call.name = Some(name);
    }

    fn observe_tool_input_ended(&mut self, data: &serde_json::Map<String, Value>) {
        let (Some(message_id), Some(call_id), Some(text)) = (
            bounded_field(data, "assistantMessageID"),
            bounded_field(data, "id"),
            data.get("text").and_then(Value::as_str),
        ) else {
            self.gap(Gap::UnknownShape);
            return;
        };
        if text.len() > MAX_STRING_BYTES {
            self.gap(Gap::CapacityExceeded);
            return;
        }
        if let Ok(input) = serde_json::from_str::<Value>(text) {
            if self.ensure_call(message_id.clone(), call_id.clone()) {
                self.calls
                    .get_mut(&(message_id, call_id))
                    .expect("inserted call")
                    .input = Some(input);
            }
        } else {
            self.gap(Gap::UnknownShape);
        }
    }

    fn observe_tool_called(&mut self, _sequence: u64, data: &serde_json::Map<String, Value>) {
        let (Some(message_id), Some(call_id), Some(input)) = (
            bounded_field(data, "assistantMessageID"),
            bounded_field(data, "id"),
            data.get("input"),
        ) else {
            self.gap(Gap::UnknownShape);
            return;
        };
        if input.to_string().len() > MAX_STRING_BYTES
            || !self.ensure_call(message_id.clone(), call_id.clone())
        {
            self.gap(Gap::CapacityExceeded);
            return;
        }
        let context = self.steps.get(&message_id).cloned();
        if context.as_ref().is_none_or(|context| {
            context.request_message_id.is_none() || context.request_text.is_none()
        }) {
            self.gap(Gap::MissingJoin);
        }
        let call = self
            .calls
            .get_mut(&(message_id, call_id))
            .expect("inserted call");
        call.input = Some(input.clone());
        if let Some(context) = context {
            call.request_message_id = context.request_message_id;
            call.request_text = context.request_text;
        }
    }

    fn observe_terminal(&mut self, terminal: TerminalEvent<'_>) -> Option<JoinedToolResult> {
        let TerminalEvent {
            event,
            event_id,
            aggregate_id,
            sequence,
            session_id,
            data,
            status,
        } = terminal;
        let (Some(message_id), Some(call_id)) = (
            bounded_field(data, "assistantMessageID"),
            bounded_field(data, "id"),
        ) else {
            self.gap(Gap::UnknownShape);
            return None;
        };
        let Some(call) = self.calls.get(&(message_id.clone(), call_id.clone())) else {
            self.gap(Gap::MissingJoin);
            return None;
        };
        let Some(name) = call.name.clone() else {
            self.gap(Gap::MissingJoin);
            return None;
        };
        let input = call.input.clone();
        let request_message_id = call.request_message_id.clone();
        let request_text = call.request_text.clone();
        let prior_terminal = call.terminal.clone();
        let content = decode_content(data.get("content"));
        if data.get("content").is_some() && content.is_none() && status == TerminalStatus::Success {
            self.gap(Gap::UnknownShape);
        }
        let created = event.get("created").and_then(Value::as_f64);
        if created.is_none() {
            self.gap(Gap::UnknownShape);
        }
        let result = JoinedToolResult {
            session_id: session_id.to_owned(),
            assistant_message_id: message_id.clone(),
            call_id: call_id.clone(),
            event_id: event_id.to_owned(),
            aggregate_id: aggregate_id.to_owned(),
            sequence,
            created,
            tool_name: name,
            input,
            request_message_id,
            request_text,
            status,
            content,
            gaps: self.gaps.clone(),
        };
        if let Some(first) = &prior_terminal {
            if first.status != result.status || first.content != result.content {
                self.gap(Gap::ConflictingDuplicate);
            }
            return None;
        }
        self.calls
            .get_mut(&(message_id, call_id))
            .expect("call was checked")
            .terminal = Some(result.clone());
        Some(result)
    }

    fn ensure_call(&mut self, message_id: String, call_id: String) -> bool {
        let key = (message_id.clone(), call_id.clone());
        if self.calls.contains_key(&key) {
            return true;
        }
        if self.calls.len() >= MAX_RETAINED_CALLS {
            self.gap(Gap::CapacityExceeded);
            return false;
        }
        self.calls.insert(
            key,
            Call {
                name: None,
                input: None,
                request_message_id: None,
                request_text: None,
                terminal: None,
            },
        );
        true
    }

    fn gap(&mut self, gap: Gap) {
        if !self.gaps.contains(&gap) {
            self.gaps.push(gap);
        }
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn bounded_field(value: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= MAX_STRING_BYTES)
        .map(str::to_owned)
}

fn decode_content(value: Option<&Value>) -> Option<String> {
    let parts = value?.as_array()?;
    let mut output = String::new();
    for part in parts {
        let text = part.get("text")?.as_str()?;
        if output.len().saturating_add(text.len()) > MAX_STRING_BYTES {
            return None;
        }
        output.push_str(text);
    }
    Some(output)
}
