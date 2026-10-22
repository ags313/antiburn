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
        crate::store::schema::MIGRATIONS.len() as i64
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
        crate::store::schema::MIGRATIONS.len() as i64
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
            crate::store::schema::MIGRATIONS.len() as i64
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
            crate::store::schema::MIGRATIONS.len() as i64
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
        .execute_batch(antiburn_local::analysis::TURN_SCHEMA_V12_SQL)
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
                .execute_batch(antiburn_local::analysis::TURN_SCHEMA_V12_SQL)
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

        let mut migrations = crate::store::schema::MIGRATIONS.to_vec();
        migrations.push("CREATE TABLE synthetic_upstream_81 (id INTEGER PRIMARY KEY) STRICT;");
        migrations.push("CREATE TABLE synthetic_upstream_82 (id INTEGER PRIMARY KEY) STRICT;");
        store.migrate_with_migrations(&migrations[..81]).unwrap();
        store.migrate_with_migrations(&migrations[..81]).unwrap();
        store.migrate_with_migrations(&migrations).unwrap();
        store.migrate_with_migrations(&migrations).unwrap();
        assert_eq!(store.schema_version().unwrap(), 82);
    }
}

#[test]
fn legacy_fork_81_runs_new_upstream_migrations_on_first_open() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for sql in crate::store::schema::MIGRATIONS {
        connection.execute_batch(sql).unwrap();
    }
    connection
        .execute_batch(antiburn_local::analysis::TURN_SCHEMA_V12_SQL)
        .unwrap();
    connection.pragma_update(None, "user_version", 81).unwrap();
    let store = store();
    *store.lock() = connection;
    let mut migrations = crate::store::schema::MIGRATIONS.to_vec();
    migrations.push("CREATE TABLE synthetic_upstream_81 (id INTEGER PRIMARY KEY) STRICT;");
    store.migrate_with_migrations(&migrations).unwrap();
    store.migrate_with_migrations(&migrations).unwrap();
    assert_eq!(store.schema_version().unwrap(), 81);
    let exists: bool = store
        .lock()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'synthetic_upstream_81')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(exists);
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
