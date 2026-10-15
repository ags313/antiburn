//! Bounded pre-alpha delivery from an immutable SQLite journal snapshot.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::ensure;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{db_session_has_parent_id, open_database, request_context, table_columns};
use crate::analysis::interface::{MAX_CONTENT_PART_BYTES, ToolResultContext};
use crate::analysis::records::tool_call_from_input;
use crate::analysis::{
    EVIDENCE_STRING_CAP, MAX_RECORD_BYTES, MAX_TOOL_RESULT_BYTES, MAX_TOOL_RESULT_OBSERVATIONS,
    PARSER_REVISION, SourceFormat, ToolResultChange, ToolResultCompleteness, ToolResultObservation,
    ToolResultPosition, ToolResultStatus,
};

const PAGE_EVENTS: u64 = 4096;
const CONTEXT_BYTES: usize = MAX_TOOL_RESULT_BYTES / 2;

/// Persist this watermark in the same transaction that publishes its page.
/// It validates the consumed prefix, not later appends or descendant activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeJournalCursor {
    session_id: String,
    owner_id: Option<String>,
    through_sequence: u64,
    prefix_hash: [u8; 32],
    parser_revision: i64,
}

/// One bounded delivery page. An empty page can still advance the watermark.
#[derive(Debug, Clone)]
pub struct OpenCodeToolResultPage {
    pub results: Vec<ToolResultObservation>,
    pub cursor: OpenCodeJournalCursor,
    pub has_more: bool,
    /// The transaction's initial native frontier, including events beyond this page.
    pub snapshot_end: u64,
    /// Page limits do not imply a source identity, order, or ownership gap.
    pub history_complete: bool,
    /// True only when this page contains the source's entire result membership.
    /// Partial pages must never infer removals from absent identities.
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenCodeJournalReadError {
    SourceChanged,
    IncompleteHistory,
    Unavailable,
}

impl std::fmt::Display for OpenCodeJournalReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::SourceChanged => "OpenCode journal prefix changed",
            Self::IncompleteHistory => "OpenCode journal has an unresolved sequence gap",
            Self::Unavailable => "OpenCode has no supported event journal",
        })
    }
}

impl std::error::Error for OpenCodeJournalReadError {}

/// Read the next pre-alpha page without changing the source database.
/// Reject a rewritten, removed, or reset prefix. A caller must not advance its
/// persisted cursor until its result publication commits.
pub fn read_opencode_tool_result_page(
    path: &Path,
    session_id: &str,
    after: Option<&OpenCodeJournalCursor>,
    cancel: &dyn Fn() -> bool,
) -> anyhow::Result<OpenCodeToolResultPage> {
    let connection = open_database(path)?;
    connection.execute_batch("BEGIN")?;
    let page = read(&connection, session_id, after, false, cancel)?
        .ok_or(OpenCodeJournalReadError::Unavailable)?;
    connection.execute_batch("COMMIT")?;
    Ok(page)
}

pub(super) fn has_schema(connection: &Connection) -> bool {
    [
        ("event_sequence", &["aggregate_id", "seq", "owner_id"][..]),
        ("event", &["id", "aggregate_id", "seq", "type", "data"][..]),
    ]
    .iter()
    .all(|(table, required)| {
        table_columns(connection, table)
            .is_ok_and(|columns| required.iter().all(|column| columns.contains(*column)))
    })
}

struct Event {
    sequence: u64,
    id: String,
    kind: String,
    data: Option<String>,
    bytes: u64,
}

fn events(connection: &Connection) -> rusqlite::Result<rusqlite::Statement<'_>> {
    connection.prepare(
        "SELECT seq,
                CASE WHEN length(CAST(id AS BLOB)) <= 512 THEN id END,
                CASE WHEN length(CAST(type AS BLOB)) <= 512 THEN type END,
                CASE WHEN length(CAST(data AS BLOB)) <= ?3 THEN data END,
                length(CAST(data AS BLOB))
           FROM event WHERE aggregate_id = ?1 AND seq <= ?2 ORDER BY seq",
    )
}

fn event(row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        sequence: row.get(0)?,
        id: row.get(1)?,
        kind: row.get(2)?,
        data: row.get(3)?,
        bytes: row.get(4)?,
    })
}

fn hash_event(
    connection: &Connection,
    session: &str,
    hash: &mut Sha256,
    event: &Event,
    cancel: &dyn Fn() -> bool,
) -> anyhow::Result<()> {
    hash.update(event.sequence.to_le_bytes());
    for text in [&event.id, &event.kind] {
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    }
    hash.update(event.bytes.to_le_bytes());
    if let Some(data) = &event.data {
        hash.update(data.as_bytes());
    } else {
        let mut offset = 0;
        while offset < event.bytes {
            ensure!(!cancel(), "OpenCode journal hash was cancelled");
            let chunk: Vec<u8> = connection.query_row(
                "SELECT substr(CAST(data AS BLOB), ?3, 65536) FROM event
                  WHERE aggregate_id = ?1 AND seq = ?2",
                params![session, event.sequence, offset + 1],
                |row| row.get(0),
            )?;
            ensure!(
                !chunk.is_empty(),
                "OpenCode journal data ended before its recorded length"
            );
            offset += chunk.len() as u64;
            hash.update(&chunk);
        }
    }
    Ok(())
}

fn identity(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= EVIDENCE_STRING_CAP)
}

fn part_id(kind: &str, data: &Value) -> Option<String> {
    let id = match kind {
        "message.part.updated.1" => &data["part"]["id"],
        "message.part.removed.1" => &data["partID"],
        _ => return None,
    };
    identity(id).map(str::to_owned)
}

struct JournalMetadata {
    latest: u64,
    owner: Option<String>,
    root: bool,
    all_events_visible: bool,
}

fn metadata(connection: &Connection, session: &str) -> anyhow::Result<Option<JournalMetadata>> {
    if !has_schema(connection) {
        return Ok(None);
    }
    ensure!(
        !session.is_empty() && session.len() <= EVIDENCE_STRING_CAP,
        "OpenCode session identity exceeds its bound"
    );
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)",
        [session],
        |row| row.get(0),
    )?;
    ensure!(exists, "OpenCode journal has no owning session");
    let root = !db_session_has_parent_id(connection)
        || connection.query_row(
            "SELECT parent_id IS NULL FROM session WHERE id = ?1",
            [session],
            |row| row.get::<_, bool>(0),
        )?;
    let metadata: Option<(u64, Option<String>)> = connection
        .query_row(
            "SELECT seq, owner_id FROM event_sequence WHERE aggregate_id = ?1
              AND (owner_id IS NULL OR length(CAST(owner_id AS BLOB)) <= ?2)",
            params![session, EVIDENCE_STRING_CAP],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((advertised, owner)) = metadata else {
        return Ok(None);
    };
    ensure!(
        advertised < i64::MAX as u64,
        "OpenCode journal sequence exceeds SQLite range"
    );
    let actual: Option<u64> = connection.query_row(
        "SELECT MAX(seq) FROM event WHERE aggregate_id = ?1",
        [session],
        |row| row.get(0),
    )?;
    let Some(actual) = actual else {
        return Ok(None);
    };
    // A reserved sequence does not prove that its event is available.
    let latest = advertised.min(actual);
    Ok(Some(JournalMetadata {
        latest,
        owner,
        root,
        all_events_visible: actual == latest,
    }))
}

fn select_parts(
    connection: &Connection,
    session: &str,
    after: Option<&OpenCodeJournalCursor>,
    range: std::ops::RangeInclusive<u64>,
    cancel: &dyn Fn() -> bool,
) -> anyhow::Result<(HashSet<String>, bool)> {
    let mut statement = events(connection)?;
    let mut rows = statement.query(params![session, range.end(), MAX_RECORD_BYTES])?;
    let mut selected = HashSet::new();
    let mut expected = 0;
    let mut ordered = true;
    let mut hash = Sha256::new();
    let mut cursor_verified = after.is_none();
    while let Some(row) = rows.next()? {
        ensure!(!cancel(), "OpenCode journal read was cancelled");
        let event = event(row)?;
        ordered &= event.sequence == expected;
        expected = event.sequence.saturating_add(1);
        hash_event(connection, session, &mut hash, &event, cancel)?;
        if let Some(cursor) = after
            && cursor.through_sequence == event.sequence
        {
            ensure!(
                <[u8; 32]>::from(hash.clone().finalize()) == cursor.prefix_hash,
                OpenCodeJournalReadError::SourceChanged
            );
            cursor_verified = true;
        }
        let data = event
            .data
            .as_deref()
            .and_then(|data| serde_json::from_str::<Value>(data).ok());
        let Some(data) = data else {
            ordered = false;
            continue;
        };
        ordered &= valid_event(&event.kind, &data, session);
        if range.contains(&event.sequence)
            && let Some(id) = part_id(&event.kind, &data)
        {
            selected.insert(id);
        }
    }
    ensure!(cursor_verified, OpenCodeJournalReadError::SourceChanged);
    ordered &= expected == range.end().saturating_add(1);
    Ok((selected, ordered))
}

pub(super) fn read(
    connection: &Connection,
    session: &str,
    after: Option<&OpenCodeJournalCursor>,
    tail: bool,
    cancel: &dyn Fn() -> bool,
) -> anyhow::Result<Option<OpenCodeToolResultPage>> {
    let Some(JournalMetadata {
        latest,
        owner,
        root,
        all_events_visible,
    }) = metadata(connection, session)?
    else {
        return Ok(None);
    };
    if let Some(cursor) = after {
        ensure!(
            cursor.parser_revision == PARSER_REVISION
                && cursor.session_id == session
                && cursor.owner_id == owner
                && cursor.through_sequence <= latest,
            OpenCodeJournalReadError::SourceChanged
        );
    }
    let start = if tail {
        latest.saturating_sub(PAGE_EVENTS - 1)
    } else {
        after.map_or(0, |cursor| cursor.through_sequence + 1)
    };
    let end = if tail {
        latest
    } else {
        latest.min(start.saturating_add(PAGE_EVENTS - 1))
    };
    let (selected, ordered) = select_parts(connection, session, after, start..=end, cancel)?;
    let mut statement = events(connection)?;
    let mut rows = statement.query(params![session, end, MAX_RECORD_BYTES])?;
    let mut requests = request_context::RequestHistory::default();
    let mut parts = HashMap::<String, PartState>::new();
    let mut results = VecDeque::new();
    let mut bytes = 0;
    let mut omitted = start != 0;
    let mut source_gap = false;
    let mut through = after.map_or(0, |cursor| cursor.through_sequence);
    let mut hash = Sha256::new();
    let mut accepted_hash = after.map_or([0; 32], |cursor| cursor.prefix_hash);
    let mut advanced = false;
    while let Some(row) = rows.next()? {
        ensure!(!cancel(), "OpenCode journal read was cancelled");
        let event = event(row)?;
        let data = event
            .data
            .as_deref()
            .and_then(|data| serde_json::from_str::<Value>(data).ok());
        let result = if let Some(data) = data {
            requests.observe(&event.kind, &data, session, event.sequence);
            if let Some(id) = part_id(&event.kind, &data).filter(|id| selected.contains(id)) {
                let state = parts.entry(id.clone()).or_default();
                let mut result = state.observe(&event.kind, &data, session, &id, event.sequence);
                source_gap |= state.invalid
                    || (event.kind == "message.part.removed.1" && state.identity.is_none());
                if event.sequence >= start
                    && let Some(result) = &mut result
                {
                    let terminal = state.first_terminal.unwrap_or(event.sequence);
                    let message = result
                        .record_id
                        .as_deref()
                        .expect("a journal result retains its message identity");
                    let mut context = if terminal == event.sequence {
                        requests.freeze(session, message, terminal)
                    } else {
                        frozen_context(connection, session, message, terminal, cancel)?
                    };
                    context.main_session &= root;
                    result.context = Some(bound_context(context));
                    if !ordered && result.completeness != ToolResultCompleteness::UnsafeIdentity {
                        result.completeness = ToolResultCompleteness::UnsafeOrder;
                    }
                    if result
                        .context
                        .as_ref()
                        .is_some_and(|context| !context.main_session)
                        && matches!(
                            result.completeness,
                            ToolResultCompleteness::Complete | ToolResultCompleteness::Incomplete
                        )
                    {
                        result.completeness = ToolResultCompleteness::UnsafeOwnership;
                    }
                    bound_content(result);
                }
                result.filter(|_| event.sequence >= start)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(result) = result {
            let size = serde_json::to_vec(&result)?.len();
            if !tail
                && (results.len() >= MAX_TOOL_RESULT_OBSERVATIONS
                    || bytes + size > MAX_TOOL_RESULT_BYTES)
            {
                break;
            }
            while tail
                && !results.is_empty()
                && (results.len() >= MAX_TOOL_RESULT_OBSERVATIONS
                    || bytes + size > MAX_TOOL_RESULT_BYTES)
            {
                let removed = results.pop_front().expect("the window is not empty");
                bytes -= serde_json::to_vec(&removed)?.len();
                omitted = true;
            }
            ensure!(
                size <= MAX_TOOL_RESULT_BYTES,
                "OpenCode result exceeds its bounded page"
            );
            bytes += size;
            results.push_back(result);
        }
        hash_event(connection, session, &mut hash, &event, cancel)?;
        through = event.sequence;
        advanced |= event.sequence >= start;
        accepted_hash = hash.clone().finalize().into();
    }
    ensure!(
        tail || through == latest || advanced,
        OpenCodeJournalReadError::IncompleteHistory
    );
    Ok(Some(OpenCodeToolResultPage {
        results: results.into(),
        cursor: OpenCodeJournalCursor {
            session_id: session.to_owned(),
            owner_id: owner,
            through_sequence: through,
            prefix_hash: accepted_hash,
            parser_revision: PARSER_REVISION,
        },
        has_more: through < latest,
        snapshot_end: latest,
        history_complete: root && requests.main_session() && ordered && !source_gap,
        complete: all_events_visible
            && root
            && requests.main_session()
            && !omitted
            && !source_gap
            && ordered
            && through == latest,
    }))
}

fn valid_event(kind: &str, data: &Value, session: &str) -> bool {
    if data["sessionID"].as_str() != Some(session) {
        return false;
    }
    match kind {
        "session.created.1" | "session.updated.1" => data["info"].is_object(),
        "message.updated.1" => {
            identity(&data["info"]["id"]).is_some()
                && matches!(data["info"]["role"].as_str(), Some("user" | "assistant"))
        }
        "message.part.updated.1" => {
            identity(&data["part"]["id"]).is_some()
                && identity(&data["part"]["messageID"]).is_some()
                && identity(&data["part"]["type"]).is_some()
                && data["part"]["sessionID"]
                    .as_str()
                    .is_none_or(|id| id == session)
        }
        "message.part.removed.1" => {
            identity(&data["partID"]).is_some() && identity(&data["messageID"]).is_some()
        }
        _ => false,
    }
}

#[derive(Default)]
struct PartState {
    identity: Option<(String, Option<String>, Option<String>, String)>,
    revision: u64,
    first_terminal: Option<u64>,
    invalid: bool,
}

impl PartState {
    fn observe(
        &mut self,
        kind: &str,
        data: &Value,
        session: &str,
        id: &str,
        sequence: u64,
    ) -> Option<ToolResultObservation> {
        if data["sessionID"].as_str() != Some(session) {
            self.invalid = true;
            return None;
        }
        let removal = kind == "message.part.removed.1";
        let part = &data["part"];
        if !removal {
            let current = (
                identity(&part["messageID"])?.to_owned(),
                identity(&part["callID"]).map(str::to_owned),
                identity(&part["tool"]).map(str::to_owned),
                identity(&part["type"])?.to_owned(),
            );
            if let Some(original) = &self.identity {
                self.invalid |= original != &current;
            } else {
                self.identity = Some(current);
            }
        }
        let (message, call, tool, original_kind) = self.identity.as_ref()?;
        if original_kind != "tool" {
            if part["type"] == "tool" {
                self.invalid = true;
            }
            return None;
        }
        self.invalid |= call.is_none()
            || tool.is_none()
            || part["sessionID"]
                .as_str()
                .is_some_and(|owner| owner != session)
            || (removal && data["messageID"].as_str() != Some(message));
        self.revision += 1;
        let state = &part["state"];
        let (status, content, terminal) = match state["status"].as_str() {
            Some("completed") => (ToolResultStatus::Success, state["output"].as_str(), true),
            Some("error")
                if state
                    .pointer("/metadata/interrupted")
                    .and_then(Value::as_bool)
                    == Some(true) =>
            {
                (
                    ToolResultStatus::Interrupted,
                    state.pointer("/metadata/output").and_then(Value::as_str),
                    true,
                )
            }
            Some("error") => (ToolResultStatus::Error, state["error"].as_str(), true),
            _ => (ToolResultStatus::Unknown, None, false),
        };
        if terminal {
            self.first_terminal.get_or_insert(sequence);
        }
        Some(ToolResultObservation {
            source_format: SourceFormat::OpenCodeSqliteV2,
            session_id: session.to_owned(),
            record_id: Some(message.clone()),
            call_id: call.clone(),
            result_id: Some(id.to_owned()),
            position: ToolResultPosition::OpenCodeEvent {
                sequence,
                item_index: 0,
            },
            revision: self.revision,
            change: if removal && !self.invalid {
                ToolResultChange::Removal
            } else {
                ToolResultChange::Update
            },
            tool_name: tool.clone(),
            tool_category: tool
                .as_deref()
                .map(|tool| tool_call_from_input(tool, state.get("input")).category),
            arguments: (!self.invalid)
                .then(|| state.get("input").cloned())
                .flatten(),
            content: (!self.invalid)
                .then(|| content.map(str::to_owned))
                .flatten(),
            status,
            finality: terminal && !self.invalid,
            completeness: if self.invalid {
                ToolResultCompleteness::UnsafeIdentity
            } else if terminal && content.is_some() {
                ToolResultCompleteness::Complete
            } else {
                ToolResultCompleteness::Incomplete
            },
            context: None,
        })
    }
}

fn frozen_context(
    connection: &Connection,
    session: &str,
    message: &str,
    sequence: u64,
    cancel: &dyn Fn() -> bool,
) -> anyhow::Result<ToolResultContext> {
    let mut requests = request_context::RequestHistory::default();
    let mut statement = events(connection)?;
    let mut rows = statement.query(params![session, sequence, MAX_RECORD_BYTES])?;
    while let Some(row) = rows.next()? {
        ensure!(!cancel(), "OpenCode request-prefix read was cancelled");
        let event = event(row)?;
        if let Some(data) = event
            .data
            .as_deref()
            .and_then(|data| serde_json::from_str::<Value>(data).ok())
        {
            requests.observe(&event.kind, &data, session, event.sequence);
        }
    }
    Ok(requests.freeze(session, message, sequence))
}

fn bound_context(mut context: ToolResultContext) -> ToolResultContext {
    while !context.user_messages.is_empty()
        && serde_json::to_vec(&context)
            .expect("source context encodes")
            .len()
            > CONTEXT_BYTES
    {
        context.user_messages.remove(0);
        context.complete_through_event = false;
    }
    context
}

fn bound_content(result: &mut ToolResultObservation) {
    let content_large = result
        .content
        .as_ref()
        .is_some_and(|text| text.len() > MAX_CONTENT_PART_BYTES);
    let arguments_large = result
        .arguments
        .as_ref()
        .is_some_and(|value| value.to_string().len() > MAX_CONTENT_PART_BYTES);
    if content_large {
        result.content = None;
    }
    if arguments_large {
        result.arguments = None;
    }
    if (content_large || arguments_large) && result.completeness == ToolResultCompleteness::Complete
    {
        result.completeness = ToolResultCompleteness::Incomplete;
    }
}
