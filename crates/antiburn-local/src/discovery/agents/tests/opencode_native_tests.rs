use super::*;
use crate::analysis::SourceFormat;

#[test]
fn native_directory_entry_limit_does_not_silently_select_a_subset() {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..=MAX_DB_DIRECTORY_ENTRIES {
        std::fs::write(directory.path().join(format!("unrelated{index}")), []).unwrap();
    }
    assert_eq!(
        database_paths_for_root_config(directory.path(), None),
        Err("directory_limit")
    );
}

#[test]
fn native_mixed_store_keeps_new_v1_children_out_of_migrated_v2_clusters() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    connection.execute_batch(
        "INSERT INTO session (id, project_id, slug, parent_id, directory, title, version, time_created, time_updated)
         VALUES ('ses_new_child', 'global', 'new-child', 'ses_root', '/legacy', 'New V1 child', '1', 1700000020000, 1700000020000);",
    ).unwrap();
    let logs = recent_db_session_logs(directory.path(), 0);
    assert!(logs.iter().any(|log| matches!(
        &log.source,
        SessionSource::ProviderDb { session_id, source_format: Some(SourceFormat::OpenCodeSqliteV1), .. }
            if session_id == "ses_new_child"
    )));
}

#[test]
fn native_mixed_store_does_not_hash_migrated_child_work_into_a_v1_root() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    connection.execute_batch(
        "INSERT INTO session (id, project_id, slug, parent_id, directory, title, version, time_created, time_updated)
         VALUES ('ses_child', 'global', 'copied-child', 'ses_v1_only', '/legacy', 'V1 copy', '1', 1700000000000, 1700000000000);
         INSERT INTO message VALUES ('msg_copied_child', 'ses_child', 1700000000000, 1700000000000, '{\"role\":\"user\"}');",
    ).unwrap();
    let before = db_session_fingerprint_connection(&connection, "ses_v1_only").unwrap();
    let admission_before =
        db_session_admission_revision_connection(&connection, "ses_v1_only").unwrap();
    connection
        .execute(
            "UPDATE message SET data = '{\"role\":\"assistant\"}' WHERE id = 'msg_copied_child'",
            [],
        )
        .unwrap();
    assert_eq!(
        before,
        db_session_fingerprint_connection(&connection, "ses_v1_only").unwrap()
    );
    connection.execute_batch(
        "UPDATE session SET title = 'Changed copy', time_updated = 1700000040000 WHERE id = 'ses_child';
         UPDATE message SET time_updated = 1700000040000 WHERE id = 'msg_copied_child';
         INSERT INTO part VALUES ('prt_copied_child', 'msg_copied_child', 'ses_child', 1700000040000, 1700000040000, '{}');",
    ).unwrap();
    assert_eq!(
        admission_before,
        db_session_admission_revision_connection(&connection, "ses_v1_only").unwrap()
    );
    assert_eq!(
        before,
        db_session_fingerprint_connection(&connection, "ses_v1_only").unwrap()
    );
    let records = query_db_records_for_root_connection(&connection, &path, "ses_v1_only").unwrap();
    assert!(records.sessions.iter().all(|(id, _)| id != "ses_child"));
    assert!(
        records
            .messages
            .iter()
            .all(|message| message.session_id != "ses_child")
    );
}

#[test]
fn native_v2_fork_parent_uses_the_explicit_native_field() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    assert_eq!(
        db_fork_parent_blocking(&path, "ses_fork").as_deref(),
        Some("ses_root")
    );
    connection
        .execute(
            "UPDATE session_v2 SET title = 'Unrelated title' WHERE id = 'ses_fork'",
            [],
        )
        .unwrap();
    assert_eq!(
        db_fork_parent_blocking(&path, "ses_fork").as_deref(),
        Some("ses_root")
    );
    assert!(db_fork_parent_blocking(&path, "ses_child").is_none());
}

#[test]
fn native_missing_and_busy_stores_do_not_look_like_successful_empty_discovery() {
    let missing = tempfile::tempdir().unwrap();
    assert!(matches!(
        query_recent_db_session_sources(missing.path(), 0),
        DbSourceRead::NotPresent
    ));
    let locked = tempfile::tempdir().unwrap();
    let connection = pinned_mixed_db(&locked.path().join("opencode.db"));
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    assert!(matches!(
        query_recent_db_session_sources(locked.path(), 0),
        DbSourceRead::Failed(_)
    ));
}

#[test]
fn native_v2_fork_lookup_rejects_invalid_ids_without_v1_title_inference() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    for parent in [
        None,
        Some("ses_fork"),
        Some("not a session id"),
        Some("msg_parent"),
    ] {
        connection.execute(
            "UPDATE session_v2 SET fork_session_id = ?1, title = 'V1 copy (fork #1)' WHERE id = 'ses_fork'",
            [parent],
        ).unwrap();
        assert!(db_fork_parent_blocking(&path, "ses_fork").is_none());
    }
}

#[test]
fn native_v1_grandchild_starts_a_new_cluster_below_a_migrated_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    connection.execute_batch(
        "INSERT INTO session (id, project_id, slug, parent_id, directory, title, version, time_created, time_updated)
         VALUES ('ses_child', 'global', 'copied-child', 'ses_v1_only', '/legacy', 'V1 child copy', '1', 1700000000000, 1700000000000),
                ('ses_new_grandchild', 'global', 'new-child', 'ses_child', '/legacy/new', 'New V1 work', '1', 1700000020000, 1700000020000),
                ('ses_new_descendant', 'global', 'new-descendant', 'ses_new_grandchild', '/legacy/new', 'New V1 descendant', '1', 1700000030000, 1700000030000);",
    ).unwrap();
    let logs = recent_db_session_logs(directory.path(), 1_700_000_015);
    assert_eq!(logs.len(), 1);
    assert!(matches!(
        &logs[0].source,
        SessionSource::ProviderDb { session_id, source_format: Some(SourceFormat::OpenCodeSqliteV1), .. }
            if session_id == "ses_new_grandchild"
    ));
    assert_eq!(logs[0].updated_at, Some(1_700_000_030));
    let records =
        query_db_records_for_root_connection(&connection, &path, "ses_new_grandchild").unwrap();
    let ids: HashSet<_> = records.sessions.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        HashSet::from(["ses_new_grandchild", "ses_new_descendant"])
    );
}

#[test]
fn native_wal_session_deletion_removes_the_exact_source_without_changing_the_main_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE)",
        )
        .unwrap();
    let main_file = std::fs::read(&path).unwrap();
    assert!(db_session_fingerprint_blocking(&path, "ses_fork").is_some());
    connection
        .execute("DELETE FROM session_v2 WHERE id = 'ses_fork'", [])
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), main_file);
    assert!(db_session_source_format(&path, "ses_fork").is_none());
    assert!(
        db_session_metadata_blocking(&path, "ses_fork", SourceFormat::OpenCodeSqliteV2).is_none()
    );
    assert!(db_session_fingerprint_blocking(&path, "ses_fork").is_none());
    assert!(db_session_admission_revision_connection(&connection, "ses_fork").is_none());
    assert!(db_fork_parent_blocking(&path, "ses_fork").is_none());
    assert!(query_session_title_by_id(&path, "ses_fork").is_none());
    assert!(
        recent_db_session_logs(directory.path(), 0)
            .iter()
            .all(|log| !matches!(&log.source, SessionSource::ProviderDb { session_id, .. } if session_id == "ses_fork"))
    );
}

#[test]
fn native_session_recreation_reads_the_new_rows_instead_of_cached_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    let before = db_session_fingerprint_blocking(&path, "ses_fork").unwrap();
    connection
        .execute("DELETE FROM session_v2 WHERE id = 'ses_fork'", [])
        .unwrap();
    assert!(db_session_fingerprint_blocking(&path, "ses_fork").is_none());
    connection.execute_batch(
        "INSERT INTO session_v2 (id, project_id, slug, directory, title, version, time_created, time_updated)
         VALUES ('ses_fork', 'global', 'recreated', '/new/exact', 'Recreated session', '2.0.32', 1700000040000, 1700000040000);
         INSERT INTO session_message VALUES ('msg_recreated', 'ses_fork', 'user', 0, 1700000040000, 1700000040000, '{}');",
    ).unwrap();
    let after = db_session_fingerprint_blocking(&path, "ses_fork").unwrap();
    assert_ne!(before, after);
    let metadata =
        db_session_metadata_blocking(&path, "ses_fork", SourceFormat::OpenCodeSqliteV2).unwrap();
    assert_eq!(metadata.title.as_deref(), Some("Recreated session"));
    assert_eq!(metadata.cwd.as_deref(), Some("/new/exact"));
    assert!(db_fork_parent_blocking(&path, "ses_fork").is_none());
    let logs = recent_db_session_logs(directory.path(), 1_700_000_035);
    assert_eq!(logs.len(), 1);
    assert!(matches!(
        &logs[0].source,
        SessionSource::ProviderDb { session_id, source_format: Some(SourceFormat::OpenCodeSqliteV2), .. }
            if session_id == "ses_fork"
    ));
}

#[test]
fn native_database_deletion_and_identical_recreation_change_source_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    let before = db_session_fingerprint_blocking(&path, "ses_fork").unwrap();
    let admission_before =
        db_session_admission_revision_connection(&connection, "ses_fork").unwrap();
    drop(connection);
    let contents = std::fs::read(&path).unwrap();
    let original_file = std::fs::File::open(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        query_recent_db_session_sources(directory.path(), 0),
        DbSourceRead::NotPresent
    ));
    assert!(db_session_source_format(&path, "ses_fork").is_none());
    assert!(
        db_session_metadata_blocking(&path, "ses_fork", SourceFormat::OpenCodeSqliteV2).is_none()
    );
    assert!(db_session_fingerprint_blocking(&path, "ses_fork").is_none());
    assert!(!path.exists());
    std::fs::write(&path, contents).unwrap();
    let after = db_session_fingerprint_blocking(&path, "ses_fork").unwrap();
    assert_eq!(before.1, after.1);
    assert_ne!(before.0, after.0);
    let recreated = Connection::open(&path).unwrap();
    assert_ne!(
        admission_before,
        db_session_admission_revision_connection(&recreated, "ses_fork").unwrap()
    );
    drop(original_file);
}

#[test]
fn native_deleted_v2_without_a_tombstone_exposes_the_retained_v1_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let connection = pinned_mixed_db(&path);
    assert_eq!(
        db_session_source_format(&path, "ses_root"),
        Some(SourceFormat::OpenCodeSqliteV2)
    );
    let v2_fingerprint = db_session_fingerprint_blocking(&path, "ses_root").unwrap();
    connection
        .execute_batch(
            "DELETE FROM session_message WHERE session_id IN ('ses_root', 'ses_child');
         DELETE FROM session_v2 WHERE id IN ('ses_root', 'ses_child');",
        )
        .unwrap();
    assert!(
        db_session_metadata_blocking(&path, "ses_root", SourceFormat::OpenCodeSqliteV2).is_none()
    );
    assert_eq!(
        db_session_source_format(&path, "ses_root"),
        Some(SourceFormat::OpenCodeSqliteV1)
    );
    assert_ne!(
        v2_fingerprint,
        db_session_fingerprint_blocking(&path, "ses_root").unwrap()
    );
    let metadata =
        db_session_metadata_blocking(&path, "ses_root", SourceFormat::OpenCodeSqliteV1).unwrap();
    assert_eq!(metadata.title.as_deref(), Some("V1 copy"));
    assert_eq!(metadata.cwd.as_deref(), Some("/old"));
    let selected: Vec<_> = recent_db_session_logs(directory.path(), 0)
        .into_iter()
        .filter(|log| matches!(&log.source, SessionSource::ProviderDb { session_id, .. } if session_id == "ses_root"))
        .collect();
    assert_eq!(selected.len(), 1);
    assert!(matches!(
        selected[0].source,
        SessionSource::ProviderDb {
            source_format: Some(SourceFormat::OpenCodeSqliteV1),
            ..
        }
    ));
}
