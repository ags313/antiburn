use super::*;

fn prerelease_connection() -> rusqlite::Connection {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for sql in &super::super::schema::MIGRATIONS[..48] {
        connection.execute_batch(sql).unwrap();
    }
    connection
        .execute_batch(super::super::schema::MIGRATIONS[50])
        .unwrap();
    connection.pragma_update(None, "user_version", 49).unwrap();
    connection
}

#[test]
fn prerelease_upgrade_preserves_incarnations_and_applies_main_migrations() {
    let connection = prerelease_connection();
    connection
        .execute_batch(
            "INSERT INTO session (
            environment_key, agent, session_id, source_kind, source_label,
            first_seen_at, last_seen_at, incarnation)
         VALUES ('native', 'pi', 'legacy', 'file', '/synthetic/legacy.jsonl', 'now', 'now', 42);
         UPDATE session_incarnation_seq SET value = 57 WHERE id = 1;
         INSERT INTO setting (key, value) VALUES ('internal:liveUsageHistoryV2', '{}');",
        )
        .unwrap();
    let store = Store::from_connection(connection, Path::new("/tmp/legacy-v49").into()).unwrap();
    assert_eq!(
        store.schema_version().unwrap(),
        super::super::schema::MIGRATIONS.len() as i64
    );
    assert_eq!(counter(&store), 57);
    let key = SessionKey {
        environment_key: "native".into(),
        agent: "pi".into(),
        session_id: "legacy".into(),
    };
    assert_eq!(stored_incarnation(&store, &key), Some(42));
    let connection = store.lock();
    let remediation_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'remediation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(remediation_sql.contains("waitingForPromptUse"));
    let history: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM setting WHERE key = 'internal:liveUsageHistoryV2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history, 0);
    drop(connection);
    store.migrate().unwrap();
    assert_eq!(counter(&store), 57);
    assert_eq!(stored_incarnation(&store, &key), Some(42));
}

#[test]
fn prerelease_repair_rolls_back_if_a_main_migration_fails() {
    let store = store();
    *store.lock() = prerelease_connection();
    store
        .lock()
        .execute_batch(
            "INSERT INTO setting (key, value) VALUES ('internal:liveUsageHistoryV2', '{}');
         CREATE TRIGGER fail_history_delete BEFORE DELETE ON setting
         BEGIN SELECT RAISE(ABORT, 'synthetic migration failure'); END;",
        )
        .unwrap();
    assert!(store.migrate().is_err());
    assert_eq!(store.schema_version().unwrap(), 49);
    let sql: String = store
        .lock()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'remediation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!sql.contains("waitingForPromptUse"));
    store
        .lock()
        .execute_batch("DROP TRIGGER fail_history_delete")
        .unwrap();
    store.migrate().unwrap();
    assert_eq!(
        store.schema_version().unwrap(),
        super::super::schema::MIGRATIONS.len() as i64
    );
}

#[test]
fn branch_allowance_versions_receive_main_quota_schema() {
    for version in [52, 53] {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        for sql in &super::super::schema::MIGRATIONS[..51] {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .execute_batch(super::super::schema::MIGRATIONS[55])
            .unwrap();
        if version == 53 {
            connection
                .execute_batch(
                    "CREATE TABLE provider_usage_period_rollup (
                        period_id INTEGER PRIMARY KEY REFERENCES provider_usage_period(id),
                        peak_used_percent REAL,
                        last_used_percent REAL,
                        observation_count INTEGER NOT NULL,
                        refusal_count INTEGER NOT NULL
                    ) STRICT;",
                )
                .unwrap();
        }
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        let store =
            Store::from_connection(connection, Path::new("/tmp/branch-allowance-schema").into())
                .unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            super::super::schema::MIGRATIONS.len() as i64
        );
        let connection = store.lock();
        let table_exists = |name: &str| -> bool {
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [name],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert!(table_exists("quota_window_reported"));
        assert!(!table_exists("provider_usage_period_rollup"));
        let factor_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'provider_limit_factor_sample'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(factor_sql.contains("lane LIKE 'model:%'"));
        drop(connection);
        store.migrate().unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            super::super::schema::MIGRATIONS.len() as i64
        );
    }
}

#[test]
fn branch_turn_cost_version_receives_main_migrations() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for sql in &super::super::schema::MIGRATIONS[..75] {
        connection.execute_batch(sql).unwrap();
    }
    // The branch build applied the turn recorded-cost column as v76.
    connection
        .execute_batch(antiburn_local::analysis::TURN_RECORDED_COST_SQL)
        .unwrap();
    connection.pragma_update(None, "user_version", 76).unwrap();

    let store = Store::from_connection(connection, Path::new("/tmp/branch-v76").into()).unwrap();

    assert_eq!(
        store.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );
    let connection = store.lock();
    let main_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
              AND name IN ('native_tool_result_publication', 'work_queue_schedule')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(main_tables, 2);
    let cost_columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('turn') WHERE name = 'recorded_cost_micros'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cost_columns, 1);
}

#[test]
fn turn_cost_is_unnumbered_for_fresh_upstream_and_fork_databases() {
    for (version, upstream_version, has_cost) in [
        (0, 0, false),
        (80, 80, false),
        (76, 75, true),
        (81, 80, true),
    ] {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        for sql in &crate::store::schema::MIGRATIONS[..upstream_version] {
            connection.execute_batch(sql).unwrap();
        }
        if has_cost {
            connection
                .execute_batch(antiburn_local::analysis::TURN_RECORDED_COST_SQL)
                .unwrap();
        }
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        let store =
            Store::from_connection(connection, Path::new("/tmp/turn-cost-schema").into()).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            crate::store::schema::MIGRATIONS.len() as i64,
            "starting version {version}"
        );
        store.migrate().unwrap();
        let count: i64 = store.lock().query_row(
            "SELECT COUNT(*) FROM pragma_table_info('turn') WHERE name = 'recorded_cost_micros'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);

        // Two future upstream migrations come after the current list.
        let current = crate::store::schema::MIGRATIONS.len();
        let mut migrations = crate::store::schema::MIGRATIONS.to_vec();
        migrations.push("CREATE TABLE synthetic_upstream_next (id INTEGER PRIMARY KEY) STRICT;");
        migrations.push("CREATE TABLE synthetic_upstream_later (id INTEGER PRIMARY KEY) STRICT;");
        store
            .migrate_with_migrations(&migrations[..current + 1])
            .unwrap();
        store
            .migrate_with_migrations(&migrations[..current + 1])
            .unwrap();
        store.migrate_with_migrations(&migrations).unwrap();
        store.migrate_with_migrations(&migrations).unwrap();
        assert_eq!(store.schema_version().unwrap(), current as i64 + 2);
    }
}

#[test]
fn legacy_fork_81_runs_new_upstream_migrations_on_first_open() {
    // Old fork v81 was upstream v80 plus the turn cost column. Upstream's own
    // v81 (the `turn_quota_published` index) and later migrations must run.
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for sql in &crate::store::schema::MIGRATIONS[..80] {
        connection.execute_batch(sql).unwrap();
    }
    connection
        .execute_batch(antiburn_local::analysis::TURN_RECORDED_COST_SQL)
        .unwrap();
    connection.pragma_update(None, "user_version", 81).unwrap();
    let store = store();
    *store.lock() = connection;
    let current = crate::store::schema::MIGRATIONS.len();
    let mut migrations = crate::store::schema::MIGRATIONS.to_vec();
    migrations.push("CREATE TABLE synthetic_upstream_next (id INTEGER PRIMARY KEY) STRICT;");
    store.migrate_with_migrations(&migrations).unwrap();
    store.migrate_with_migrations(&migrations).unwrap();
    assert_eq!(store.schema_version().unwrap(), current as i64 + 1);
    for name in ["turn_quota_published", "synthetic_upstream_next"] {
        let exists: bool = store
            .lock()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "{name}");
    }
}

fn remote_cache_dir_columns(store: &Store) -> i64 {
    store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('session') WHERE name = 'remote_cache_dir'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn remote_cache_dir_is_unnumbered_for_fresh_and_upstream_databases() {
    // A fresh database gets the column.
    let fresh = store();
    assert_eq!(remote_cache_dir_columns(&fresh), 1);
    assert_eq!(
        fresh.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );

    // An upstream database has only the numbered ladder and no column.
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for sql in crate::store::schema::MIGRATIONS {
        connection.execute_batch(sql).unwrap();
    }
    connection
        .pragma_update(
            None,
            "user_version",
            crate::store::schema::MIGRATIONS.len() as i64,
        )
        .unwrap();
    let has_column: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('session') WHERE name = 'remote_cache_dir')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!has_column);
    let upstream =
        Store::from_connection(connection, Path::new("/tmp/remote-cache-dir-v80").into()).unwrap();
    assert_eq!(remote_cache_dir_columns(&upstream), 1);
    assert_eq!(
        upstream.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );
}

#[test]
fn remote_cache_dir_migration_is_a_no_op_on_a_database_that_has_the_column() {
    let store = store();
    let key = SessionKey::new("ssh:host", "claude-code", "synthetic");
    store.upsert_sessions(&[session_for(&key)], &[]).unwrap();
    store
        .set_remote_cache_dir(&key, Path::new("/synthetic/generation"))
        .unwrap();
    store.migrate().unwrap();
    store.migrate().unwrap();
    assert_eq!(remote_cache_dir_columns(&store), 1);
    assert_eq!(
        store.remote_cache_dir(&key).unwrap().as_deref(),
        Some(Path::new("/synthetic/generation"))
    );
    assert_eq!(
        store.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );
}

fn session_for(key: &SessionKey) -> SessionRecord {
    SessionRecord {
        key: key.clone(),
        ..session(&key.session_id, 1)
    }
}

fn active_minute_objects(store: &Store) -> i64 {
    store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN
                ('session_active_minute', 'session_active_minute_epoch')",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn active_minute_table_is_unnumbered_for_fresh_upstream_and_current_databases() {
    let fresh = store();
    assert_eq!(active_minute_objects(&fresh), 2);

    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for sql in crate::store::schema::MIGRATIONS {
        connection.execute_batch(sql).unwrap();
    }
    connection
        .pragma_update(
            None,
            "user_version",
            crate::store::schema::MIGRATIONS.len() as i64,
        )
        .unwrap();
    let upstream =
        Store::from_connection(connection, Path::new("/tmp/active-minute-v80").into()).unwrap();
    assert_eq!(active_minute_objects(&upstream), 2);
    assert_eq!(
        upstream.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );

    fresh.migrate().unwrap();
    fresh.migrate().unwrap();
    assert_eq!(active_minute_objects(&fresh), 2);
    assert_eq!(
        fresh.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );
}

fn hourly_usage_objects(store: &Store) -> i64 {
    store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN
                ('session_minute_usage', 'session_minute_usage_minute')",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn hourly_usage_table_is_unnumbered_for_fresh_upstream_and_current_databases() {
    let fresh = store();
    assert_eq!(hourly_usage_objects(&fresh), 2);

    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for sql in crate::store::schema::MIGRATIONS {
        connection.execute_batch(sql).unwrap();
    }
    connection
        .pragma_update(
            None,
            "user_version",
            crate::store::schema::MIGRATIONS.len() as i64,
        )
        .unwrap();
    let upstream =
        Store::from_connection(connection, Path::new("/tmp/hourly-usage-v80").into()).unwrap();
    assert_eq!(hourly_usage_objects(&upstream), 2);
    assert_eq!(
        upstream.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );

    fresh.migrate().unwrap();
    fresh.migrate().unwrap();
    assert_eq!(hourly_usage_objects(&fresh), 2);
    assert_eq!(
        fresh.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );
}

#[test]
fn minute_usage_step_drops_the_replaced_hourly_table() {
    let store = store();
    store
        .lock()
        .execute_batch(
            "CREATE TABLE session_hourly_usage (hour_epoch INTEGER);
             CREATE INDEX session_hourly_usage_hour ON session_hourly_usage (hour_epoch);",
        )
        .unwrap();
    store.migrate().unwrap();
    store.migrate().unwrap();
    let leftovers: i64 = store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'session_hourly_usage%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leftovers, 0);
    assert_eq!(hourly_usage_objects(&store), 2);
}

#[test]
fn both_version_81_schemas_upgrade_without_losing_runtime_definitions() {
    const FEATURE_V81: &str = r#"
CREATE TABLE burn_check_runtime_definition (
    check_id TEXT NOT NULL,
    activation_scope_key TEXT NOT NULL,
    generation INTEGER CHECK (generation IS NULL OR generation > 0),
    definition_revision TEXT NOT NULL,
    name TEXT NOT NULL,
    availability TEXT NOT NULL CHECK (availability IN ('available', 'paused', 'invalid', 'unsupported')),
    PRIMARY KEY (check_id, activation_scope_key)
) STRICT;
CREATE TABLE burn_check_runtime_definition_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    protocol_version INTEGER NOT NULL CHECK (protocol_version = 1),
    initialized INTEGER NOT NULL CHECK (initialized IN (0, 1))
) STRICT;
INSERT INTO burn_check_runtime_definition_state VALUES (1, 1, 0);
"#;
    for (feature_schema, version) in [(false, 81), (true, 81), (true, 82), (true, 83)] {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        for sql in &super::super::schema::MIGRATIONS[..80] {
            connection.execute_batch(sql).unwrap();
        }
        if feature_schema {
            connection.execute_batch(FEATURE_V81).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO burn_check_runtime_definition VALUES
                 ('retry-loop', 'main', 1, 'r1', 'Retry loop', 'available');
                 UPDATE burn_check_runtime_definition_state SET initialized = 1;",
                )
                .unwrap();
        } else {
            connection
                .execute_batch(super::super::schema::MIGRATIONS[80])
                .unwrap();
        }
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        let store =
            Store::from_connection(connection, Path::new("/tmp/version-81-collision").into())
                .unwrap();
        assert_eq!(store.schema_version().unwrap(), 92);
        store.migrate().unwrap();
        let connection = store.lock();
        let initialized: i64 = connection
            .query_row(
                "SELECT initialized FROM burn_check_runtime_definition_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(initialized, i64::from(feature_schema));
        let definitions: i64 = connection.query_row(
            "SELECT count(*) FROM burn_check_runtime_definition WHERE check_id = 'retry-loop' AND definition_revision = 'r1'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(definitions, i64::from(feature_schema));
        let indexes: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'turn_quota_published'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(indexes, 1);
    }
}

#[test]
fn opencode_branch_fixture_frames_lf_and_crlf_without_changing_sql() {
    let expected = opencode_branch_migrations(include_str!(
        "../../schema/compatibility/branch_history.sql"
    ));
    assert_eq!(expected.len(), 7);
    for newline in ["\n", "\r\n"] {
        let fixture = expected.join(&format!("-- next migration{newline}"));
        assert_eq!(opencode_branch_migrations(&fixture), expected);
    }
}

fn opencode_branch_migrations(sql: &str) -> Vec<&str> {
    let mut steps = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    for line in sql.split_inclusive('\n') {
        if matches!(line, "-- next migration\n" | "-- next migration\r\n") {
            steps.push(&sql[start..offset]);
            start = offset + line.len();
        }
        offset += line.len();
    }
    steps.push(&sql[start..]);
    steps
}

#[test]
fn opencode_branch_versions_receive_the_main_quota_index() {
    let branch_migrations = opencode_branch_migrations(include_str!(
        "../../schema/compatibility/branch_history.sql"
    ));
    assert_eq!(branch_migrations.len(), 7);
    for version in 81..=83 {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        for sql in &super::super::schema::MIGRATIONS[..80] {
            connection.execute_batch(sql).unwrap();
        }
        for sql in &branch_migrations[..version - 80] {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .pragma_update(None, "user_version", version as i64)
            .unwrap();
        let store =
            Store::from_connection(connection, Path::new("/tmp/opencode-branch").into()).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            super::super::schema::MIGRATIONS.len() as i64
        );
        let index_exists: bool = store
            .lock()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema
                 WHERE type = 'index' AND name = 'turn_quota_published')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(index_exists);
        for (table, column) in [
            ("opencode_v2_capture", "admitted_source_json"),
            ("opencode_v2_capture", "retirement_reason"),
            ("turn", "accounting_json"),
        ] {
            let exists: bool = store
                .lock()
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
                    [table, column],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "{table}.{column} must survive migration");
        }
        store.migrate().unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            super::super::schema::MIGRATIONS.len() as i64
        );
    }
}

#[test]
fn typed_source_and_gap_metadata_migrates_existing_publications() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for sql in &super::super::schema::MIGRATIONS[..84] {
        connection.execute_batch(sql).unwrap();
    }
    connection.pragma_update(None, "user_version", 84).unwrap();
    connection
        .execute_batch(
            r#"INSERT INTO session (environment_key, agent, session_id, source_kind,
                   source_label, first_seen_at, last_seen_at)
               VALUES ('native', 'opencode', 'ses_v1', 'providerDb',
                       'opencode:ses_v1', 'now', 'now'),
                      ('native', 'opencode', 'ses_v2', 'providerDb',
                       'opencode:ses_v2:open_code_v2_sqlite', 'now', 'now'),
                      ('native', 'opencode', 'ses_export', 'inline',
                       'opencode:ses_export', 'now', 'now');
               INSERT INTO native_tool_result_publication
                   (outcome, environment_key, agent, session_id, source_format,
                    complete, source_generation, claim_fence, gap_reason)
               VALUES ('coverage_gap', 'native', 'opencode', 'ses_v1',
                       '"open_code_sqlite_v2"', 0, 1, 1, NULL),
                      ('coverage_gap', 'native', 'opencode', 'ses_v2',
                       '"open_code_v2_sqlite"', 0, 1, 1, NULL),
                      ('coverage_gap', 'native', 'opencode', 'ses_v2',
                       '"open_code_v2_sqlite"', 0, 1, 1, 'v2-history-incomplete');"#,
        )
        .unwrap();
    let store = Store::from_connection(connection, Path::new("/tmp/typed-source").into()).unwrap();
    let connection = store.lock();
    for (id, format) in [
        ("ses_v1", "\"open_code_sqlite_v2\""),
        ("ses_v2", "\"open_code_v2_sqlite\""),
        ("ses_export", "\"open_code_jsonl\""),
    ] {
        let stored: String = connection
            .query_row(
                "SELECT source_format FROM session WHERE session_id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, format);
    }
    for (id, reason, kind) in [
        ("ses_v1", None, "invalidated"),
        ("ses_v2", None, "segment_reset"),
        ("ses_v2", Some("v2-history-incomplete"), "partial"),
    ] {
        let stored: String = connection
            .query_row(
                "SELECT gap_kind FROM native_tool_result_publication
                 WHERE session_id = ?1 AND gap_reason IS ?2",
                params![id, reason],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, kind);
    }
    assert!(
        connection
            .execute(
                "UPDATE native_tool_result_publication SET gap_kind = 'partial'",
                [],
            )
            .is_err()
    );
}
