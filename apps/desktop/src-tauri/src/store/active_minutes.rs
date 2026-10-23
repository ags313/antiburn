//! Per-session record of the minutes in which a session has a published turn.
//!
//! The table holds the distinct `(session, minute)` pairs that the
//! Overview's turn-minute scan would return for the session's published turn
//! rows. Publication rebuilds one session's rows in its own transaction, and
//! every path that deletes the session's turn rows deletes them too. The fork
//! adds the table after the numbered migrations, so a fill pass covers the
//! sessions that exist before the table.

use std::time::Duration;

use anyhow::Result;
use rusqlite::{Connection, params};

use super::{SessionKey, Store};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS session_active_minute (
    environment_key TEXT NOT NULL,
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    minute_epoch INTEGER NOT NULL,
    PRIMARY KEY (environment_key, agent, session_id, minute_epoch),
    FOREIGN KEY (environment_key, agent, session_id)
        REFERENCES session (environment_key, agent, session_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS session_active_minute_epoch
    ON session_active_minute (minute_epoch);
";

/// Set to `done` when the fill pass has covered every existing session.
const BACKFILL_KEY: &str = "internal:activeMinutesBackfillV1";
const BACKFILL_DONE: &str = "done";
/// Sessions rebuilt per transaction. The writer lock is free between batches.
const BACKFILL_BATCH: usize = 25;
const BACKFILL_PAUSE: Duration = Duration::from_millis(20);

/// Creates the table and its index when the database does not have them.
pub(super) fn ensure_schema(connection: &Connection) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name = 'session_active_minute')
           AND EXISTS(SELECT 1 FROM sqlite_master
            WHERE type = 'index' AND name = 'session_active_minute_epoch')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        connection.execute_batch(SCHEMA)?;
    }
    Ok(())
}

/// Replaces one session's rows with the minutes of its published turn rows.
///
/// The row filter matches the Overview scan: a turn counts when its fence is
/// the session's published fence, its time is positive, and its environment
/// is not an `ssh:` one.
pub(super) fn rebuild_in(connection: &Connection, key: &SessionKey) -> Result<()> {
    delete_in(connection, key)?;
    connection.execute(
        "INSERT INTO session_active_minute
             (environment_key, agent, session_id, minute_epoch)
         SELECT DISTINCT t.environment_key, t.agent, t.session_id,
                (t.ts_ms / 60000) * 60
           FROM turn t
           JOIN session_evidence e
             ON e.environment_key = t.environment_key
            AND e.agent = t.agent AND e.session_id = t.session_id
            AND e.published_fence = t.claim_fence
          WHERE t.environment_key = ?1 AND t.agent = ?2 AND t.session_id = ?3
            AND t.ts_ms > 0
            AND t.environment_key NOT LIKE 'ssh:%'",
        params![key.environment_key, key.agent, key.session_id],
    )?;
    Ok(())
}

/// Removes one session's rows.
pub(super) fn delete_in(connection: &Connection, key: &SessionKey) -> Result<()> {
    connection.execute(
        "DELETE FROM session_active_minute
          WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3",
        params![key.environment_key, key.agent, key.session_id],
    )?;
    Ok(())
}

impl Store {
    /// Fills the table for sessions that existed before it, then records
    /// completion in `setting`.
    ///
    /// Each batch runs in its own transaction and releases the connection
    /// lock before the pause, so a publication waits for at most one batch.
    /// A rebuild equals the rebuild that publication runs, so a pass that
    /// races with a publication, or restarts after an exit, is safe.
    pub(crate) fn backfill_active_minutes(&self) -> Result<()> {
        let done: Option<String> = {
            let connection = self.lock();
            connection
                .query_row(
                    "SELECT value FROM setting WHERE key = ?1",
                    [BACKFILL_KEY],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    error => Err(error),
                })?
        };
        if done.as_deref() == Some(BACKFILL_DONE) {
            return Ok(());
        }
        let mut cursor: Option<SessionKey> = None;
        while let Some(last) = self.backfill_active_minute_batch(cursor.as_ref())? {
            cursor = Some(last);
            std::thread::sleep(BACKFILL_PAUSE);
        }
        self.lock().execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![BACKFILL_KEY, BACKFILL_DONE],
        )?;
        Ok(())
    }

    /// Rebuilds the next batch of sessions after `after`, in key order.
    /// Returns the last key of the batch, or `None` when no session is left.
    fn backfill_active_minute_batch(
        &self,
        after: Option<&SessionKey>,
    ) -> Result<Option<SessionKey>> {
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        let mut keys = {
            let mut statement = transaction.prepare(
                "SELECT environment_key, agent, session_id FROM session_evidence
                  WHERE published_fence IS NOT NULL
                    AND (?1 IS NULL OR (environment_key, agent, session_id) > (?1, ?2, ?3))
                  ORDER BY environment_key, agent, session_id
                  LIMIT ?4",
            )?;
            statement
                .query_map(
                    params![
                        after.map(|key| key.environment_key.as_str()),
                        after.map(|key| key.agent.as_str()),
                        after.map(|key| key.session_id.as_str()),
                        BACKFILL_BATCH as i64
                    ],
                    |row| {
                        Ok(SessionKey::new(
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for key in &keys {
            rebuild_in(&transaction, key)?;
        }
        transaction.commit()?;
        Ok(keys.pop())
    }
}
