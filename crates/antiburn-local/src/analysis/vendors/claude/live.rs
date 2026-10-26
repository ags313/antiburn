//! Native result delivery for the pinned CLI shape, without metric accumulation.

use super::*;
use crate::analysis::resume::identity_filter::IdentityFilter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct State {
    parser: ClaudeStreamState,
    retired_calls: IdentityFilter,
    records: IdentityFilter,
    last: Option<String>,
    root: bool,
    /// A bounded prefix or summary hit can leave the predecessor unproved.
    order_unproven: bool,
}

impl State {
    pub fn new(input: &SessionInput, header: &Value, suffix_only: bool) -> Self {
        Self {
            parser: ClaudeStreamState::for_input(input),
            retired_calls: IdentityFilter::default(),
            records: IdentityFilter::default(),
            last: None,
            order_unproven: suffix_only,
            root: is_root_input(input)
                && root_record(header, &input.session_id)
                && header.get("parentUuid") == Some(&Value::Null)
                && header.get("logicalParentUuid").is_none_or(Value::is_null)
                && (retained_root::is_user_content(header)
                    || retained_root::is_informational_root(header)),
        }
    }

    pub fn observe(&mut self, mut value: Value, index: u64, sink: &mut dyn RecordSink) {
        let suffix_start = self.order_unproven && self.last.is_none();
        let Some(id) = thread_identity_field(&value, "uuid") else {
            if matches!(value["type"].as_str(), Some("user" | "assistant")) {
                self.root = false;
            }
            return;
        };
        let retained_record = self.parser.user_message_root.has_record(&value);
        if self.parser.user_message_root.exact_record_replay(&value) {
            return;
        }
        let summary_hit = self.records.contains(&id);
        if retained_record || summary_hit {
            for block in value["message"]["content"].as_array().into_iter().flatten() {
                if block["type"] == "tool_use"
                    && let Some(call_id) = block["id"].as_str()
                {
                    self.retired_calls.insert(call_id);
                    self.parser.native_calls.remove(call_id);
                }
            }
            self.parser.native_calls.retain(|_, call| {
                call.as_ref()
                    .is_none_or(|call| call.record_id.as_deref() != Some(&id))
            });
            self.parser.native_call_bytes = self
                .parser
                .native_calls
                .iter()
                .map(|(id, call)| id.len() + call.as_ref().map_or(0, |call| call.retained_bytes))
                .sum();
            if retained_record {
                sink.native_admission_failure(crate::analysis::NativeAdmissionFailure {
                    source_format: self.parser.source_format,
                    session_id: self.parser.session_id.clone(),
                    position: ToolResultPosition::Jsonl {
                        record_index: index,
                        item_index: u32::MAX,
                    },
                    reason: ToolResultCompleteness::UnsafeIdentity,
                });
                return;
            }
            sink.tool_result_gap();
            self.order_unproven = !self.continues_root(&value);
            if !self.order_unproven {
                self.last = Some(id);
                self.parser.user_message_root.observe_user_slot(&value);
            }
            return;
        }
        self.records.insert(&id);
        if self.order_unproven && !suffix_start && !self.continues_root(&value) {
            sink.tool_result_gap();
            return;
        }
        self.order_unproven = false;
        let unsafe_owner = value["sessionId"]
            .as_str()
            .is_some_and(|session| session != self.parser.session_id)
            || value["isSidechain"] == true
            || value.get("agentId").is_some();
        let scope_failure = self.root
            && (unsafe_owner
                || (suffix_start && retained_root::retained_parent(&value) == Some(id.as_str()))
                || self.last.as_ref().is_some_and(|last| {
                    value.get("parentUuid").is_some()
                        && retained_root::retained_parent(&value) != Some(last.as_str())
                }));
        if scope_failure {
            sink.native_admission_failure(crate::analysis::NativeAdmissionFailure {
                source_format: self.parser.source_format,
                session_id: self.parser.session_id.clone(),
                position: ToolResultPosition::Jsonl {
                    record_index: index,
                    item_index: u32::MAX,
                },
                reason: if unsafe_owner {
                    ToolResultCompleteness::UnsafeOwnership
                } else {
                    ToolResultCompleteness::UnsafeOrder
                },
            });
        }
        self.root = if suffix_start {
            self.root
                && root_record(&value, &self.parser.session_id)
                && retained_root::retained_parent(&value).is_some()
        } else {
            self.continues_root(&value)
        };
        self.last = Some(id);
        if suffix_start && self.root {
            self.parser.user_message_root.observe_retained_start(&value);
        } else {
            self.parser.user_message_root.observe_user_slot(&value);
        }
        if retained_root::is_compaction_boundary(&value) || value["isCompactSummary"] == true {
            sink.tool_result_gap();
        }
        if self.root
            && !self
                .parser
                .user_message_root
                .admits(&self.parser.session_id)
        {
            sink.tool_result_gap();
            self.root = false;
        }
        if self.root {
            emit_user_message(&value, index, &self.parser.session_id, sink);
            if value["type"] == "user"
                && !retained_root::is_user_content(&value)
                && value["message"]["content"]
                    .as_array()
                    .is_none_or(|blocks| blocks.iter().any(|block| block["type"] != "tool_result"))
            {
                sink.tool_result_gap();
            }
        }
        if value["type"] == "assistant"
            && let Some(blocks) = value
                .pointer_mut("/message/content")
                .and_then(Value::as_array_mut)
        {
            for block in blocks {
                if block["type"] == "tool_use"
                    && block["id"].as_str().is_some_and(|id| {
                        !self.parser.native_calls.contains_key(id)
                            && self.retired_calls.contains(id)
                    })
                {
                    *block = Value::Null;
                }
            }
        }
        self.parser.capture_native_tool_results_with_public(
            &value,
            index,
            sink,
            scope_failure
                || (self.root
                    && self
                        .parser
                        .user_message_root
                        .admits(&self.parser.session_id)),
        );
        if self.root && native_activity_record(&value) {
            crate::analysis::public_events::emit_activity(
                sink,
                self.parser.source_format,
                &self.parser.session_id,
                index,
                value["uuid"].as_str().unwrap_or("message"),
                value.get("timestamp").and_then(parse_ts),
            );
        }
        self.parser.native_calls.retain(|_, call| call.is_some());
        self.parser.native_call_bytes = self
            .parser
            .native_calls
            .iter()
            .map(|(id, call)| id.len() + call.as_ref().map_or(0, |call| call.retained_bytes))
            .sum();
        if value["type"] == "assistant" {
            for block in value["message"]["content"].as_array().into_iter().flatten() {
                if block["type"] == "tool_use"
                    && let Some(id) = block["id"].as_str()
                {
                    self.retired_calls.insert(id);
                }
            }
        }
        if value["type"] == "user" {
            for block in value["message"]["content"].as_array().into_iter().flatten() {
                if block["type"] == "tool_result"
                    && let Some(id) = block["tool_use_id"].as_str()
                {
                    self.retired_calls.insert(id);
                    self.parser.native_calls.remove(id);
                }
            }
            self.parser.native_call_bytes = self
                .parser
                .native_calls
                .iter()
                .map(|(id, call)| id.len() + call.as_ref().map_or(0, |call| call.retained_bytes))
                .sum();
        }
    }

    pub fn main_session(&self) -> bool {
        self.root
    }

    pub(crate) fn migrate_pre_65_unobserved_suffix(&mut self) {
        if self.root && self.last.is_none() && self.parser.user_message_root.is_pristine_user_slot()
        {
            self.order_unproven = true;
        }
    }

    fn continues_root(&self, value: &Value) -> bool {
        self.root
            && root_record(value, &self.parser.session_id)
            && self
                .last
                .as_deref()
                .is_none_or(|last| retained_root::retained_parent(value) == Some(last))
    }

    pub fn invalidate(&mut self) {
        self.root = false;
    }
}

fn root_record(record: &Value, session: &str) -> bool {
    record["version"] == "2.1.278"
        && record["entrypoint"] == "cli"
        && record["sessionId"] == session
        && record["isSidechain"] == false
        && record.get("agentId").is_none()
        && (!matches!(record["type"].as_str(), Some("user" | "assistant"))
            || record.pointer("/message/role") == record.get("type"))
}

/// Exact revision-4 layout. Conversion keeps the existing root and record proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StateV4 {
    parser: ClaudeStreamState,
    retired_calls: IdentityFilter,
    records: IdentityFilter,
    last: Option<String>,
    root: bool,
}

impl StateV4 {
    pub(crate) fn convert(self, history_incomplete: bool) -> State {
        State {
            parser: self.parser,
            retired_calls: self.retired_calls,
            records: self.records,
            last: self.last,
            root: self.root,
            order_unproven: history_incomplete,
        }
    }
}

#[cfg(test)]
impl State {
    pub(crate) fn as_v4(&self) -> StateV4 {
        StateV4 {
            parser: self.parser.clone(),
            retired_calls: self.retired_calls.clone(),
            records: self.records.clone(),
            last: self.last.clone(),
            root: self.root,
        }
    }
}
