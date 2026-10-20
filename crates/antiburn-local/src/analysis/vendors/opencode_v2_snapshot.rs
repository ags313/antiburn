//! Reads the pinned OpenCode V2 SQLite message projection.

use std::path::Path;

use anyhow::Context;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;

use crate::analysis::SourceFormat;

const MAX_SNAPSHOT_ROW_BYTES: i64 = 8 * 1024 * 1024;
const MAX_SNAPSHOT_ROWS: u64 = 100_000;
const MAX_SNAPSHOT_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

/// One source-shaped message reconstructed from the V2 projection row.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotMessage {
    pub session_id: String,
    pub seq: i64,
    pub value: Value,
}

/// Counts the rows read from one complete SQLite cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotReadSummary {
    pub rows_seen: u64,
    pub messages_visited: u64,
    pub oversized_rows: u64,
    pub capacity_limit_reached: bool,
    pub bytes_visited: u64,
    /// True only when the cursor reached its end.
    pub reached_eof: bool,
}

impl SnapshotReadSummary {
    /// True when the reader skipped one or more oversized rows.
    pub fn is_partial(self) -> bool {
        self.oversized_rows > 0 || self.capacity_limit_reached
    }
}

/// Read one session from an OpenCode V2 snapshot.
///
/// The read transaction gives SQLite one consistent view of the database and
/// its committed WAL frames. The connection is read-only and does not create
/// or change database files. The visitor handles each accepted row before the
/// reader loads the next one. Oversized rows are skipped and mark the summary
/// partial. Malformed rows, visitor failures, and cancellation return errors.
pub fn visit_snapshot(
    path: &Path,
    source_format: SourceFormat,
    session_id: &str,
    cancel: &dyn Fn() -> bool,
    visitor: &mut dyn FnMut(SnapshotMessage) -> anyhow::Result<()>,
) -> anyhow::Result<SnapshotReadSummary> {
    if source_format != SourceFormat::OpenCodeSqliteV2 {
        anyhow::bail!("OpenCode V2 snapshot reader rejects source format {source_format:?}");
    }
    if session_id.is_empty() {
        anyhow::bail!("OpenCode V2 snapshot reader requires a session id");
    }

    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("opening OpenCode V2 snapshot {}", path.display()))?;
    connection.execute_batch("BEGIN")?;
    validate_schema(&connection)?;

    let session_exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_v2 WHERE id = ?1)",
        [session_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !session_exists {
        anyhow::bail!("OpenCode V2 snapshot has no session {session_id}");
    }

    let mut statement = connection.prepare(
        "SELECT
             CASE WHEN row_bytes <= ?2 THEN id END,
             CASE WHEN row_bytes <= ?2 THEN session_id END,
             CASE WHEN row_bytes <= ?2 THEN type END,
             seq,
             CASE WHEN row_bytes <= ?2 THEN data END,
             row_bytes
           FROM (
             SELECT id, session_id, type, seq, data,
                    length(CAST(id AS BLOB))
                      + length(CAST(session_id AS BLOB))
                      + length(CAST(type AS BLOB))
                      + length(CAST(data AS BLOB)) AS row_bytes
               FROM session_message
              WHERE session_id = ?1
           )
          ORDER BY session_id, seq",
    )?;
    let mut rows = statement.query(params![session_id, MAX_SNAPSHOT_ROW_BYTES])?;
    let mut summary = SnapshotReadSummary {
        rows_seen: 0,
        messages_visited: 0,
        oversized_rows: 0,
        capacity_limit_reached: false,
        bytes_visited: 0,
        reached_eof: false,
    };
    let mut previous_seq = None;
    while let Some(row) = rows.next()? {
        if summary.rows_seen >= MAX_SNAPSHOT_ROWS {
            summary.capacity_limit_reached = true;
            break;
        }
        if cancel() {
            anyhow::bail!("OpenCode V2 snapshot read was cancelled");
        }
        summary.rows_seen = summary.rows_seen.saturating_add(1);
        let row_bytes = row
            .get::<_, Option<i64>>(5)?
            .context("invalid message row size")?;
        if row_bytes > MAX_SNAPSHOT_ROW_BYTES {
            summary.oversized_rows = summary.oversized_rows.saturating_add(1);
            continue;
        }
        let row_bytes = row_bytes as u64;
        if summary.bytes_visited.saturating_add(row_bytes) > MAX_SNAPSHOT_TOTAL_BYTES {
            summary.capacity_limit_reached = true;
            break;
        }
        summary.bytes_visited = summary.bytes_visited.saturating_add(row_bytes);

        let id = required_text(row, 0, "message id")?;
        let row_session_id = required_text(row, 1, "message session id")?;
        let message_type = required_text(row, 2, "message type")?;
        let seq = row.get::<_, i64>(3).context("invalid message sequence")?;
        let data = required_text(row, 4, "message data")?;

        if row_session_id != session_id {
            anyhow::bail!("OpenCode V2 snapshot returned a message for another session");
        }
        if previous_seq.is_some_and(|previous| seq <= previous) {
            anyhow::bail!("OpenCode V2 snapshot has duplicate or out-of-order message sequence");
        }
        previous_seq = Some(seq);
        if !known_message_type(&message_type) {
            anyhow::bail!("OpenCode V2 snapshot has unknown message type {message_type:?}");
        }

        let mut value = serde_json::from_str::<Value>(&data)
            .context("OpenCode V2 snapshot has malformed message JSON")?;
        let Some(object) = value.as_object_mut() else {
            anyhow::bail!("OpenCode V2 snapshot message data is not a JSON object");
        };
        object.insert("id".into(), Value::String(id));
        object.insert("type".into(), Value::String(message_type));
        visitor(SnapshotMessage {
            session_id: row_session_id,
            seq,
            value,
        })?;
        summary.messages_visited = summary.messages_visited.saturating_add(1);
    }
    summary.reached_eof = !summary.capacity_limit_reached;

    connection.execute_batch("COMMIT")?;
    Ok(summary)
}

fn validate_schema(connection: &Connection) -> anyhow::Result<()> {
    if !crate::analysis::opencode_v2_schema::is_supported(connection)? {
        anyhow::bail!("OpenCode V2 snapshot has an unsupported schema");
    }
    Ok(())
}

fn required_text(row: &rusqlite::Row<'_>, index: usize, label: &str) -> anyhow::Result<String> {
    row.get::<_, String>(index)
        .with_context(|| format!("OpenCode V2 snapshot has invalid {label}"))
}

fn known_message_type(message_type: &str) -> bool {
    matches!(
        message_type,
        "agent-switched"
            | "model-switched"
            | "location-switched"
            | "user"
            | "synthetic"
            | "system"
            | "skill"
            | "shell"
            | "assistant"
            | "compaction"
            | "idle"
    )
}
