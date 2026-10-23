//! Per-session per-minute totals of the published assistant turn rows.
//!
//! Each row sums the rows that the `turn_days` subquery of
//! `USAGE_EVIDENCE_SQL` groups by day, but by UTC minute. Only rows with a
//! positive timestamp count. The minutes of one session add up to its day
//! totals for any UTC offset, and they keep the minute precision that limit
//! periods need. The `unrecorded_*` sums cover the rows without a recorded
//! cost, which the readers price from catalog rates. Publication rebuilds one
//! session's rows in its own transaction, and every path that deletes the
//! session's turn rows deletes them too. The fork adds the table after the
//! numbered migrations, so a fill pass covers the sessions that exist before
//! the table.

use std::time::Duration;

use anyhow::Result;
use rusqlite::{Connection, params};

use super::{SessionKey, Store};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS session_minute_usage (
    environment_key TEXT NOT NULL,
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    minute_epoch INTEGER NOT NULL,
    model TEXT NOT NULL,
    speed TEXT NOT NULL,
    row_count INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    cache_write_1h_tokens INTEGER NOT NULL,
    unrecorded_input_tokens INTEGER NOT NULL,
    unrecorded_output_tokens INTEGER NOT NULL,
    unrecorded_cache_read_tokens INTEGER NOT NULL,
    unrecorded_cache_write_tokens INTEGER NOT NULL,
    unrecorded_cache_write_1h_tokens INTEGER NOT NULL,
    recorded_cost_micros INTEGER NOT NULL,
    recorded_rows INTEGER NOT NULL,
    PRIMARY KEY (environment_key, agent, session_id, minute_epoch, model, speed),
    FOREIGN KEY (environment_key, agent, session_id)
        REFERENCES session (environment_key, agent, session_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS session_minute_usage_minute
    ON session_minute_usage (minute_epoch);
";

/// Set to `done` when the fill pass has covered every existing session.
const BACKFILL_KEY: &str = "internal:minuteUsageBackfillV1";
const BACKFILL_DONE: &str = "done";
/// Sessions rebuilt per transaction. The writer lock is free between batches.
const BACKFILL_BATCH: usize = 25;
const BACKFILL_PAUSE: Duration = Duration::from_millis(20);

/// Creates the table and its index when the database does not have them, and
/// drops the hourly table that this one replaces.
pub(super) fn ensure_schema(connection: &Connection) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name = 'session_minute_usage')
           AND EXISTS(SELECT 1 FROM sqlite_master
            WHERE type = 'index' AND name = 'session_minute_usage_minute')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        connection.execute_batch(SCHEMA)?;
    }
    connection.execute_batch(
        "DROP INDEX IF EXISTS session_hourly_usage_hour;
         DROP TABLE IF EXISTS session_hourly_usage;",
    )?;
    Ok(())
}

/// Replaces one session's rows with the per-minute sums of its published turn
/// rows. The row filter is the one of the `turn_days` subquery, plus a
/// positive timestamp. A `NULL` speed is stored as an empty string.
pub(super) fn rebuild_in(connection: &Connection, key: &SessionKey) -> Result<()> {
    delete_in(connection, key)?;
    connection.execute(
        "INSERT INTO session_minute_usage
             (environment_key, agent, session_id, minute_epoch, model, speed,
              row_count, input_tokens, output_tokens, cache_read_tokens,
              cache_write_tokens, cache_write_1h_tokens,
              unrecorded_input_tokens, unrecorded_output_tokens,
              unrecorded_cache_read_tokens, unrecorded_cache_write_tokens,
              unrecorded_cache_write_1h_tokens, recorded_cost_micros, recorded_rows)
         SELECT t.environment_key, t.agent, t.session_id,
                (t.ts_ms / 60000) * 60 AS minute_epoch,
                t.model, COALESCE(t.speed, ''),
                COUNT(*),
                CAST(TOTAL(t.input_tokens) AS INTEGER),
                CAST(TOTAL(t.output_tokens) AS INTEGER),
                CAST(TOTAL(t.cache_read_tokens) AS INTEGER),
                CAST(TOTAL(t.cache_write_tokens) AS INTEGER),
                CAST(TOTAL(t.cache_write_1h_tokens) AS INTEGER),
                CAST(TOTAL(CASE WHEN t.recorded_cost_micros IS NULL
                                THEN t.input_tokens END) AS INTEGER),
                CAST(TOTAL(CASE WHEN t.recorded_cost_micros IS NULL
                                THEN t.output_tokens END) AS INTEGER),
                CAST(TOTAL(CASE WHEN t.recorded_cost_micros IS NULL
                                THEN t.cache_read_tokens END) AS INTEGER),
                CAST(TOTAL(CASE WHEN t.recorded_cost_micros IS NULL
                                THEN t.cache_write_tokens END) AS INTEGER),
                CAST(TOTAL(CASE WHEN t.recorded_cost_micros IS NULL
                                THEN t.cache_write_1h_tokens END) AS INTEGER),
                CAST(TOTAL(t.recorded_cost_micros) AS INTEGER),
                COUNT(t.recorded_cost_micros)
           FROM session_evidence e
           JOIN turn t
             ON t.environment_key = e.environment_key
            AND t.agent = e.agent
            AND t.session_id = e.session_id
            AND t.claim_fence = e.published_fence
          WHERE e.environment_key = ?1 AND e.agent = ?2 AND e.session_id = ?3
            AND t.ts_ms > 0
            AND t.role = 'assistant'
            AND t.model IS NOT NULL
            AND (t.input_tokens != 0 OR t.output_tokens != 0
                 OR t.cache_read_tokens != 0 OR t.cache_write_tokens != 0)
          GROUP BY minute_epoch, t.model, COALESCE(t.speed, '')",
        params![key.environment_key, key.agent, key.session_id],
    )?;
    Ok(())
}

/// Removes one session's rows.
pub(super) fn delete_in(connection: &Connection, key: &SessionKey) -> Result<()> {
    connection.execute(
        "DELETE FROM session_minute_usage
          WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3",
        params![key.environment_key, key.agent, key.session_id],
    )?;
    Ok(())
}

impl Store {
    /// Fills the table for sessions that existed before it, then records
    /// completion in `setting`. Batching and restart safety match
    /// `backfill_active_minutes`.
    pub(crate) fn backfill_minute_usage(&self) -> Result<()> {
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
        while let Some(last) = self.backfill_minute_usage_batch(cursor.as_ref())? {
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
    fn backfill_minute_usage_batch(
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
