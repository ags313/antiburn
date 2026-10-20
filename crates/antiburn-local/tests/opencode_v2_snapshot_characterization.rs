#[path = "../src/analysis/vendors/opencode_v2_snapshot.rs"]
mod opencode_v2_snapshot;

mod analysis {
    pub use antiburn_local::analysis::*;
}

use antiburn_local::analysis::SourceFormat;
use opencode_v2_snapshot::{SnapshotMessage, SnapshotReadSummary, visit_snapshot};
use rusqlite::{Connection, params};
use tempfile::tempdir;

fn snapshot() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE session_v2 (id TEXT PRIMARY KEY);
             CREATE TABLE session_message (
                 id TEXT PRIMARY KEY,
                 session_id TEXT NOT NULL,
                 type TEXT NOT NULL,
                 seq INTEGER NOT NULL,
                 data TEXT NOT NULL
             );
             INSERT INTO session_v2 (id) VALUES ('ses_fixture');",
        )
        .unwrap();
    (directory, path)
}

fn insert(path: &std::path::Path, id: &str, message_type: &str, seq: i64, data: &str) {
    Connection::open(path)
        .unwrap()
        .execute(
            "INSERT INTO session_message (id, session_id, type, seq, data)
             VALUES (?1, 'ses_fixture', ?2, ?3, ?4)",
            params![id, message_type, seq, data],
        )
        .unwrap();
}

fn collect(path: &std::path::Path) -> (SnapshotReadSummary, Vec<SnapshotMessage>) {
    let mut messages = Vec::new();
    let summary = visit_snapshot(
        path,
        SourceFormat::OpenCodeSqliteV2,
        "ses_fixture",
        &|| false,
        &mut |message| {
            messages.push(message);
            Ok(())
        },
    )
    .unwrap();
    (summary, messages)
}

#[test]
fn reconstructs_row_identity_and_type_and_orders_by_sequence() {
    let (_directory, path) = snapshot();
    insert(&path, "msg_later", "assistant", 9, r#"{"text":"answer"}"#);
    insert(&path, "msg_first", "user", 2, r#"{"text":"request"}"#);

    let (summary, messages) = collect(&path);

    assert_eq!(
        messages
            .iter()
            .map(|message| message.seq)
            .collect::<Vec<_>>(),
        [2, 9]
    );
    assert_eq!(messages[0].value["id"], "msg_first");
    assert_eq!(messages[0].value["type"], "user");
    assert_eq!(messages[0].value["text"], "request");
    assert_eq!(messages[1].value["id"], "msg_later");
    assert_eq!(messages[1].value["type"], "assistant");
    assert!(summary.reached_eof);
    assert!(!summary.is_partial());
}

#[test]
fn rejects_malformed_message_json() {
    let (_directory, path) = snapshot();
    insert(&path, "msg_bad", "user", 0, "{");

    let error = visit_snapshot(
        &path,
        SourceFormat::OpenCodeSqliteV2,
        "ses_fixture",
        &|| false,
        &mut |_| Ok(()),
    )
    .unwrap_err();

    assert!(error.to_string().contains("malformed message JSON"));
}

#[test]
fn rejects_non_object_message_data_and_unknown_types() {
    let (_directory, path) = snapshot();
    insert(&path, "msg_array", "user", 0, "[]");
    assert!(
        visit_snapshot(
            &path,
            SourceFormat::OpenCodeSqliteV2,
            "ses_fixture",
            &|| false,
            &mut |_| Ok(()),
        )
        .is_err()
    );

    Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM session_message", [])
        .unwrap();
    insert(&path, "msg_unknown", "future-message", 0, "{}");
    assert!(
        visit_snapshot(
            &path,
            SourceFormat::OpenCodeSqliteV2,
            "ses_fixture",
            &|| false,
            &mut |_| Ok(()),
        )
        .is_err()
    );
}

#[test]
fn rejects_other_source_formats() {
    let (_directory, path) = snapshot();

    let error = visit_snapshot(
        &path,
        SourceFormat::OpenCodeSqliteV1,
        "ses_fixture",
        &|| false,
        &mut |_| Ok(()),
    )
    .unwrap_err();

    assert!(error.to_string().contains("rejects source format"));
}

#[test]
fn reads_committed_wal_rows_from_a_read_only_snapshot() {
    let (_directory, path) = snapshot();
    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE;")
        .unwrap();
    writer
        .execute(
            "INSERT INTO session_message (id, session_id, type, seq, data)
             VALUES ('msg_wal', 'ses_fixture', 'user', 0, '{\"text\":\"wal\"}')",
            [],
        )
        .unwrap();
    writer.execute_batch("COMMIT;").unwrap();

    let (summary, messages) = collect(&path);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].value["text"], "wal");
    assert!(summary.reached_eof);
}

#[test]
fn oversized_rows_are_skipped_and_report_partial_results() {
    let (_directory, path) = snapshot();
    let oversized = format!("{{\"text\":\"{}\"}}", "x".repeat(8 * 1024 * 1024));
    insert(&path, "msg_large", "user", 0, &oversized);
    insert(&path, "msg_after", "assistant", 1, r#"{"text":"after"}"#);

    let (summary, messages) = collect(&path);

    assert_eq!(summary.rows_seen, 2);
    assert_eq!(summary.messages_visited, 1);
    assert_eq!(summary.oversized_rows, 1);
    assert!(summary.reached_eof);
    assert!(summary.is_partial());
    assert_eq!(messages[0].value["id"], "msg_after");
}

#[test]
fn visitor_streams_all_rows_before_claiming_eof() {
    let (_directory, path) = snapshot();
    let connection = Connection::open(&path).unwrap();
    for seq in 0..257 {
        connection
            .execute(
                "INSERT INTO session_message (id, session_id, type, seq, data)
                 VALUES (?1, 'ses_fixture', 'user', ?2, '{}')",
                params![format!("msg_{seq}"), seq],
            )
            .unwrap();
    }
    drop(connection);

    let (summary, messages) = collect(&path);

    assert_eq!(summary.rows_seen, 257);
    assert_eq!(summary.messages_visited, 257);
    assert!(summary.reached_eof);
    assert_eq!(messages.last().unwrap().seq, 256);
}

#[test]
fn cancellation_stops_without_reporting_eof() {
    let (_directory, path) = snapshot();
    insert(&path, "msg_one", "user", 0, "{}");
    let error = visit_snapshot(
        &path,
        SourceFormat::OpenCodeSqliteV2,
        "ses_fixture",
        &|| true,
        &mut |_| Ok(()),
    )
    .unwrap_err();

    assert!(error.to_string().contains("cancelled"));
}
