//! Removes the Unix device number from stored file identities.
//!
//! Older releases stored `<device>:<inode>` in file fingerprints and rollout
//! checkpoints. A volume can get a different device number at reboot, for
//! example on macOS. Then each stored value looks like a changed file, and
//! sessions outside the scan window stay in the `source-changed` retry loop.
//! The fork rewrites the values in place at open, without a numbered
//! migration. A rewritten value does not match again, so the step is
//! idempotent. The step reads each fingerprint column, and in
//! `source_resume` the column comes after a large snapshot blob, so a cold
//! read takes many seconds. Thus a marker table records a completed step,
//! and a later open does not read the columns again. The current build writes
//! only values without a device number.

use anyhow::Result;
use rusqlite::{Connection, params};

#[cfg(unix)]
use super::SessionKey;

/// Each stored column that can hold a file fingerprint.
#[cfg(unix)]
const FINGERPRINT_COLUMNS: [(&str, &str); 5] = [
    ("session", "source_fingerprint"),
    ("session", "activity_cursor"),
    ("session_analysis", "source_fingerprint"),
    ("session_evidence", "processed_fingerprint"),
    ("source_resume", "source_fingerprint"),
];

/// The table that records a completed step.
#[cfg(unix)]
const DONE_MARKER: &str =
    "CREATE TABLE IF NOT EXISTS fork_legacy_device_done (id INTEGER PRIMARY KEY) STRICT;";

#[cfg(unix)]
pub(super) fn drop_legacy_devices(connection: &mut Connection) -> Result<()> {
    use antiburn_local::discovery::source_version::without_legacy_device;

    let done: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name = 'fork_legacy_device_done')",
        [],
        |row| row.get(0),
    )?;
    if done {
        return Ok(());
    }
    let transaction = connection.transaction()?;
    for (table, column) in FINGERPRINT_COLUMNS {
        let rows = {
            let mut statement = transaction.prepare(&format!(
                "SELECT rowid, {column} FROM {table} WHERE {column} GLOB '*sv1:[0-9]*:[0-9]*:*'"
            ))?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut update = transaction.prepare(&format!(
            "UPDATE {table} SET {column} = ?2 WHERE rowid = ?1"
        ))?;
        let mut requeue = Vec::new();
        for (rowid, value) in rows {
            let Some(value) = without_legacy_device(&value) else {
                continue;
            };
            update.execute(params![rowid, value])?;
            if (table, column) == ("session", "source_fingerprint") {
                requeue.push(rowid);
            }
        }
        drop(update);
        for rowid in requeue {
            requeue_source_changed(&transaction, rowid)?;
        }
    }
    transaction.execute(
        "UPDATE provider_usage_rollout_checkpoint
            SET source_identity = substr(source_identity, instr(source_identity, ':') + 1)
          WHERE source_identity GLOB '[0-9]*:[0-9]*'",
        [],
    )?;
    transaction.execute_batch(DONE_MARKER)?;
    transaction.commit()?;
    Ok(())
}

/// Retries the evidence of a session at once when its old fingerprint caused
/// a `source-changed` result. Without this step a failed row stays failed.
#[cfg(unix)]
fn requeue_source_changed(connection: &Connection, session_rowid: i64) -> Result<()> {
    let key = connection.query_row(
        "SELECT session.environment_key, session.agent, session.session_id
           FROM session
           JOIN session_evidence USING (environment_key, agent, session_id)
          WHERE session.rowid = ?1
            AND session_evidence.status IN ('pending', 'failed')
            AND session_evidence.last_error = ?2",
        params![
            session_rowid,
            crate::insights_worker::EVIDENCE_ERROR_SOURCE_CHANGED
        ],
        |row| {
            Ok(SessionKey {
                environment_key: row.get(0)?,
                agent: row.get(1)?,
                session_id: row.get(2)?,
            })
        },
    );
    match key {
        Ok(key) => super::session_persistence::mark_evidence_pending_in(connection, &key),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(unix))]
pub(super) fn drop_legacy_devices(_connection: &mut Connection) -> Result<()> {
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::store::Store;

    const LEGACY: &str = "sv1:16777229:372276567:518849:1792628294:1:00ff";
    const CURRENT: &str = "sv1:ino372276567:518849:1792628294:1:00ff";

    fn insert(connection: &Connection, session_id: &str, status: &str, last_error: Option<&str>) {
        connection
            .execute(
                "INSERT INTO session (environment_key, agent, session_id, source_kind,
                     source_label, first_seen_at, last_seen_at, source_fingerprint)
                 VALUES ('native', 'claude-code', ?1, 'file', ?1, 't', 't', ?2)",
                params![session_id, LEGACY],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO session_evidence (environment_key, agent, session_id, status,
                     processed_fingerprint, retry_count, last_error)
                 VALUES ('native', 'claude-code', ?1, ?2, ?3, 4, ?4)",
                params![session_id, status, LEGACY, last_error],
            )
            .unwrap();
    }

    fn row(connection: &Connection, session_id: &str) -> (String, String, String, i64) {
        connection
            .query_row(
                "SELECT session.source_fingerprint, evidence.processed_fingerprint,
                        evidence.status, evidence.retry_count
                   FROM session
                   JOIN session_evidence AS evidence USING (environment_key, agent, session_id)
                  WHERE session.session_id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    }

    #[test]
    fn legacy_device_numbers_are_removed_once_and_stuck_evidence_is_retried() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(directory.path()).unwrap();
        let mut connection = store.lock();
        // The store open already ran the step on the empty database.
        connection
            .execute_batch("DROP TABLE fork_legacy_device_done")
            .unwrap();
        insert(&connection, "ready", "ready", None);
        insert(&connection, "stuck", "failed", Some("source-changed"));
        insert(&connection, "missing", "failed", Some("source-missing"));
        connection
            .execute(
                "INSERT INTO provider_usage_rollout_checkpoint
                     (environment_key, agent, session_id, provider, source_label,
                      source_identity, status, updated_at_epoch)
                 VALUES ('native', 'claude-code', 'ready', 'codex', 'ready',
                         '16777229:42', 'complete', 1)",
                [],
            )
            .unwrap();

        drop_legacy_devices(&mut connection).unwrap();
        drop_legacy_devices(&mut connection).unwrap();
        // The marker stops a later open from reading the columns again.
        insert(&connection, "later", "ready", None);
        drop_legacy_devices(&mut connection).unwrap();
        assert_eq!(
            row(&connection, "later"),
            (LEGACY.into(), LEGACY.into(), "ready".into(), 4)
        );

        assert_eq!(
            row(&connection, "ready"),
            (CURRENT.into(), CURRENT.into(), "ready".into(), 4)
        );
        assert_eq!(
            row(&connection, "stuck"),
            (CURRENT.into(), CURRENT.into(), "pending".into(), 0)
        );
        assert_eq!(
            row(&connection, "missing"),
            (CURRENT.into(), CURRENT.into(), "failed".into(), 4)
        );
        let identity: String = connection
            .query_row(
                "SELECT source_identity FROM provider_usage_rollout_checkpoint",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(identity, "42");
    }
}
