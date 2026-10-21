use anyhow::Result;
use rusqlite::params;

use super::Store;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS cursor_usage_event (
    usage_event_id TEXT PRIMARY KEY NOT NULL,
    ts_ms INTEGER NOT NULL,
    model TEXT NOT NULL,
    kind TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    recorded_cost_micros INTEGER,
    conversation_id TEXT,
    headless INTEGER NOT NULL CHECK (headless IN (0, 1)),
    fetched_at INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS cursor_usage_event_ts ON cursor_usage_event(ts_ms);
";

/// One account usage event. Event time uses milliseconds; fetch time uses epoch seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCursorUsageEvent {
    pub usage_event_id: String,
    pub ts_ms: i64,
    pub model: String,
    pub kind: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub recorded_cost_micros: Option<u64>,
    pub conversation_id: Option<String>,
    pub headless: bool,
    pub fetched_at: i64,
}

impl Store {
    /// Store each event once. Return the number of new events.
    pub fn insert_cursor_usage_events(&self, events: &[StoredCursorUsageEvent]) -> Result<usize> {
        let mut connection = self.lock();
        let tx = connection.transaction()?;
        let mut inserted = 0;
        {
            let mut statement = tx.prepare_cached(
                "INSERT OR IGNORE INTO cursor_usage_event
                 (usage_event_id, ts_ms, model, kind, input_tokens, output_tokens,
                  cache_read_tokens, recorded_cost_micros, conversation_id, headless, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for event in events {
                inserted += statement.execute(params![
                    event.usage_event_id,
                    event.ts_ms,
                    event.model,
                    event.kind,
                    event.input_tokens,
                    event.output_tokens,
                    event.cache_read_tokens,
                    event.recorded_cost_micros,
                    event.conversation_id,
                    event.headless,
                    event.fetched_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Read stored events in the half-open millisecond interval without network access.
    pub fn cursor_usage_events(
        &self,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<StoredCursorUsageEvent>> {
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT usage_event_id, ts_ms, model, kind, input_tokens, output_tokens,
                    cache_read_tokens, recorded_cost_micros, conversation_id, headless, fetched_at
             FROM cursor_usage_event WHERE ts_ms >= ?1 AND ts_ms < ?2
             ORDER BY ts_ms, usage_event_id",
        )?;
        Ok(statement
            .query_map(params![start_ms, end_ms], |row| {
                Ok(StoredCursorUsageEvent {
                    usage_event_id: row.get(0)?,
                    ts_ms: row.get(1)?,
                    model: row.get(2)?,
                    kind: row.get(3)?,
                    input_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    cache_read_tokens: row.get(6)?,
                    recorded_cost_micros: row.get(7)?,
                    conversation_id: row.get(8)?,
                    headless: row.get(9)?,
                    fetched_at: row.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::AppSettings;

    fn event(id: &str, ts_ms: i64) -> StoredCursorUsageEvent {
        StoredCursorUsageEvent {
            usage_event_id: id.into(),
            ts_ms,
            model: "test-model".into(),
            kind: "usage".into(),
            input_tokens: 12,
            output_tokens: 3,
            cache_read_tokens: 4,
            recorded_cost_micros: Some(125),
            conversation_id: Some("conversation".into()),
            headless: true,
            fetched_at: 2_000_000_000,
        }
    }

    #[test]
    fn cursor_usage_deduplicates_and_reads_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(dir.path()).unwrap();
        let event = event("one", 1234);
        assert_eq!(
            store
                .insert_cursor_usage_events(&[event.clone(), event.clone()])
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .insert_cursor_usage_events(std::slice::from_ref(&event))
                .unwrap(),
            0
        );
        let mut duplicate = event.clone();
        duplicate.output_tokens = 999;
        assert_eq!(store.insert_cursor_usage_events(&[duplicate]).unwrap(), 0);
        assert_eq!(store.cursor_usage_events(1234, 1235).unwrap(), vec![event]);
        assert!(store.cursor_usage_events(0, 1234).unwrap().is_empty());
    }

    #[test]
    fn cursor_usage_preserves_unknown_cost_and_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(dir.path()).unwrap();
        let mut event = event("unknown", 1000);
        event.recorded_cost_micros = None;
        event.conversation_id = None;
        event.headless = false;
        store
            .insert_cursor_usage_events(std::slice::from_ref(&event))
            .unwrap();
        assert_eq!(store.cursor_usage_events(0, 2000).unwrap(), vec![event]);
    }

    #[test]
    fn cursor_usage_retention_uses_event_time_and_clear_removes_events() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(dir.path()).unwrap();
        let now = 2_000_000_000;
        let cutoff_ms = (now - 30 * 86_400) * 1000;
        let events = [
            event("old", cutoff_ms - 1),
            event("boundary", cutoff_ms),
            event("recent", now * 1000),
        ];
        store.insert_cursor_usage_events(&events).unwrap();
        store.apply_session_retention(now).unwrap();
        assert_eq!(store.cursor_usage_events(0, i64::MAX).unwrap(), events);
        store
            .save_settings(&AppSettings {
                session_data_retention_days: 30,
                ..store.settings().unwrap()
            })
            .unwrap();
        store.apply_session_retention(now).unwrap();
        assert_eq!(store.cursor_usage_events(0, i64::MAX).unwrap(), events[1..]);
        store.clear_local_session_data().unwrap();
        assert!(store.cursor_usage_events(0, i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn cursor_usage_schema_is_idempotent_without_numbered_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let event = event("persisted", 1234);
        store
            .insert_cursor_usage_events(std::slice::from_ref(&event))
            .unwrap();
        let version: i64 = store
            .lock()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.cursor_usage_events(0, i64::MAX).unwrap(), vec![event]);
        let connection = store.lock();
        connection.execute_batch(SCHEMA).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
    #[test]
    fn cursor_usage_table_is_added_to_an_existing_database() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let version: i64 = store
            .lock()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        store
            .lock()
            .execute_batch("DROP TABLE cursor_usage_event")
            .unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store
                .insert_cursor_usage_events(&[event("new", 1000)])
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .lock()
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
}
