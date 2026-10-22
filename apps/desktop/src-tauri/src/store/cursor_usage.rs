use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use rusqlite::params;

use super::{SessionKey, Store, UsageEvidenceRecord, UsageOrigins, UsageTurnDay};

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

/// One stored event with the session it belongs to. `session` is `None` for
/// account-level usage that no stored Cursor session claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributedCursorUsageEvent {
    pub event: StoredCursorUsageEvent,
    pub session: Option<SessionKey>,
}

const CURSOR_AGENT: &str = "cursor";

/// The conversation id a session answers to: its own id or its transcript file stem.
fn transcript_stem(source_label: &str) -> Option<&str> {
    let name = source_label.rsplit(['/', '\\']).next()?;
    let (stem, _) = name.rsplit_once('.')?;
    (!stem.is_empty()).then_some(stem)
}

impl Store {
    pub(super) fn cursor_usage_evidence(
        &self,
        since_epoch: i64,
        utc_offset_seconds: i32,
        origins: UsageOrigins,
    ) -> Result<BTreeMap<Option<SessionKey>, UsageEvidenceRecord>> {
        let mut records = BTreeMap::new();
        for attributed in
            self.attributed_cursor_usage_events(since_epoch.saturating_mul(1000), i64::MAX)?
        {
            if origins == UsageOrigins::Local
                && attributed
                    .session
                    .as_ref()
                    .is_none_or(|key| key.environment_key.starts_with("ssh:"))
            {
                continue;
            }
            let event = attributed.event;
            let at = event.ts_ms.div_euclid(1000);
            let account_level = attributed.session.is_none();
            let record = records
                .entry(attributed.session)
                .or_insert_with(|| UsageEvidenceRecord {
                    account_level,
                    agent: CURSOR_AGENT.into(),
                    updated_at_epoch: at,
                    model_breakdown_json: None,
                    pricing_breakdown_json: None,
                    provider_hints_json: None,
                    provider_accounts_json: "[]".into(),
                    turn_days: Vec::new(),
                });
            record.updated_at_epoch = record.updated_at_epoch.max(at);
            let offset = i64::from(utc_offset_seconds);
            record.turn_days.push(UsageTurnDay {
                day_start_epoch: Some((at + offset).div_euclid(86_400) * 86_400 - offset),
                model: event.model,
                speed: None,
                tokens: antiburn_local::pricing::ModelTokens {
                    input_tokens: event.input_tokens,
                    output_tokens: event.output_tokens,
                    cache_read_tokens: event.cache_read_tokens,
                    recorded_cost_micros: event.recorded_cost_micros,
                    ..Default::default()
                },
            });
        }
        Ok(records)
    }

    pub fn cursor_account_usage_for_session(
        &self,
        key: &SessionKey,
    ) -> Result<Option<crate::dto::CursorAccountUsage>> {
        if key.agent != CURSOR_AGENT {
            return Ok(None);
        }
        let mut usage = None;
        for attributed in self.attributed_cursor_usage_events(i64::MIN, i64::MAX)? {
            if attributed.session.as_ref() != Some(key) {
                continue;
            }
            let total = usage.get_or_insert(crate::dto::CursorAccountUsage {
                tokens: Default::default(),
                cost_usd: Some(0.0),
            });
            let event = attributed.event;
            total.tokens.input_tokens += event.input_tokens;
            total.tokens.output_tokens += event.output_tokens;
            total.tokens.cache_read_tokens += event.cache_read_tokens;
            total.cost_usd = total
                .cost_usd
                .zip(event.recorded_cost_micros)
                .map(|(cost, micros)| cost + micros as f64 / 1_000_000.0);
        }
        Ok(usage)
    }

    pub fn newest_cursor_usage_timestamp_ms(&self) -> Result<Option<i64>> {
        Ok(self
            .lock()
            .query_row("SELECT MAX(ts_ms) FROM cursor_usage_event", [], |row| {
                row.get(0)
            })?)
    }

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

    /// Attribute each event in the half-open millisecond interval to one Cursor
    /// session in any environment. A conversation that several sessions claim goes
    /// to the lowest `(environment_key, session_id)`, and an own-id match wins over
    /// a transcript-stem match. Events with no match stay account-level.
    pub fn attributed_cursor_usage_events(
        &self,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<AttributedCursorUsageEvent>> {
        if !self.settings_snapshot().cursor_account_usage_enabled {
            return Ok(Vec::new());
        }
        let events = self.cursor_usage_events(start_ms, end_ms)?;
        let mut owners: HashMap<String, ((bool, String, String), SessionKey)> = HashMap::new();
        {
            let connection = self.lock();
            let mut statement = connection.prepare(
                "SELECT environment_key, session_id, source_label FROM session WHERE agent = ?1",
            )?;
            let rows = statement.query_map(params![CURSOR_AGENT], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (environment_key, session_id, source_label) = row?;
                let key = SessionKey::new(&environment_key, CURSOR_AGENT, &session_id);
                let stem = transcript_stem(&source_label).map(str::to_owned);
                let ids =
                    std::iter::once((session_id.clone(), false)).chain(stem.map(|s| (s, true)));
                for (conversation, by_stem) in ids {
                    let rank = (by_stem, environment_key.clone(), session_id.clone());
                    match owners.get(&conversation) {
                        Some((best, _)) if *best <= rank => {}
                        _ => {
                            owners.insert(conversation, (rank, key.clone()));
                        }
                    }
                }
            }
        }
        Ok(events
            .into_iter()
            .map(|event| {
                let session = event
                    .conversation_id
                    .as_ref()
                    .and_then(|id| owners.get(id))
                    .map(|(_, key)| key.clone());
                AttributedCursorUsageEvent { event, session }
            })
            .collect())
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
    fn session_detail_account_totals_require_a_match_and_consent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(dir.path()).unwrap();
        store
            .update_settings(|s| s.cursor_account_usage_enabled = true)
            .unwrap();
        insert_cursor_session(&store, "native", "matched", "/synthetic/conversation.jsonl");
        insert_cursor_session(&store, "native", "unmatched", "/synthetic/unmatched.jsonl");
        store
            .insert_cursor_usage_events(&[event("one", 1000), event("two", 2000)])
            .unwrap();
        let key = SessionKey::new("native", "cursor", "matched");
        let usage = store
            .cursor_account_usage_for_session(&key)
            .unwrap()
            .unwrap();
        assert_eq!(usage.tokens.input_tokens, 24);
        assert_eq!(usage.tokens.output_tokens, 6);
        assert_eq!(usage.tokens.cache_read_tokens, 8);
        assert_eq!(usage.cost_usd, Some(0.00025));
        assert_eq!(
            store
                .cursor_account_usage_for_session(&SessionKey::new("native", "cursor", "unmatched"))
                .unwrap(),
            None
        );
        let mut unknown = event("unknown", 3000);
        unknown.recorded_cost_micros = None;
        store.insert_cursor_usage_events(&[unknown]).unwrap();
        assert_eq!(
            store
                .cursor_account_usage_for_session(&key)
                .unwrap()
                .unwrap()
                .cost_usd,
            None
        );
        store
            .update_settings(|s| s.cursor_account_usage_enabled = false)
            .unwrap();
        assert_eq!(store.cursor_account_usage_for_session(&key).unwrap(), None);
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

    fn insert_cursor_session(store: &Store, environment_key: &str, session_id: &str, label: &str) {
        use crate::store::SessionRecord;
        store
            .upsert_sessions(
                &[SessionRecord {
                    key: SessionKey::new(environment_key, "cursor", session_id),
                    source_kind: "file".into(),
                    source_label: label.into(),
                    wsl_distro: None,
                    title: None,
                    title_source: None,
                    cwd: None,
                    surface: "unknown".into(),
                    updated_at_epoch: Some(1),
                    activity_cursor: "synthetic".into(),
                    activity_source: "event".into(),
                    subagent_count: 0,
                    fork_parent_session_id: None,
                    source_fingerprint: Some("synthetic".into()),
                }],
                &[],
            )
            .unwrap();
    }

    fn conversation_event(id: &str, conversation: Option<&str>) -> StoredCursorUsageEvent {
        StoredCursorUsageEvent {
            conversation_id: conversation.map(str::to_owned),
            ..event(id, 1000)
        }
    }

    #[test]
    fn overview_counts_cursor_event_days_costs_and_origins_without_turn_duplicates() {
        use crate::provider_usage::{summarize, window_bounds};

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(dir.path()).unwrap();
        let now = 1_800_000_000;
        let today = window_bounds(now, 60).today_start;
        insert_cursor_session(&store, "native", "local", "/synthetic/local.jsonl");
        insert_cursor_session(&store, "ssh:box", "remote", "/synthetic/remote.jsonl");
        insert_cursor_session(&store, "wsl:Ubuntu", "wsl", "/synthetic/wsl.jsonl");
        {
            let connection = store.lock();
            connection
                .execute_batch(
                    "INSERT INTO session_evidence
                    (environment_key, agent, session_id, published_fence)
                 SELECT environment_key, agent, session_id, 1 FROM session;
                 INSERT INTO turn
                    (environment_key, agent, session_id, claim_fence, source_key,
                     thread_id, turn_index, scope, role, ts_ms, model,
                     input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                     is_compaction_boundary)
                 SELECT environment_key, agent, session_id, 1, 'synthetic',
                        'main', 0, 'main', 'assistant', 1000, 'test-model',
                        999, 999, 0, 0, 0 FROM session;",
                )
                .unwrap();
            connection
                .execute("UPDATE session SET updated_at_epoch = ?1", params![now])
                .unwrap();
            connection
                .execute("UPDATE turn SET ts_ms = ?1", params![now * 1000])
                .unwrap();
        }
        let events = [
            ("local", Some("local"), today - 2 * 86_400 + 60),
            ("remote", Some("remote"), today - 86_400 + 60),
            ("account", Some("unmatched"), today + 60),
            ("account-no-id", None, today + 120),
            ("wsl", Some("wsl"), today - 2 * 86_400 + 120),
        ]
        .map(|(id, conversation, at)| StoredCursorUsageEvent {
            ts_ms: at * 1000,
            recorded_cost_micros: Some(15_000),
            ..conversation_event(id, conversation)
        });
        store.insert_cursor_usage_events(&events).unwrap();
        let since = today - 59 * 86_400;
        let read = |origins| store.usage_evidence(since, 3600, origins).unwrap();
        let disabled = read(UsageOrigins::LocalAndRemote);
        assert_eq!(disabled.len(), 3);
        assert!(
            disabled
                .iter()
                .all(|row| row.turn_days[0].tokens.input_tokens == 999)
        );

        store
            .update_settings(|s| s.cursor_account_usage_enabled = true)
            .unwrap();
        let overview = summarize(&read(UsageOrigins::LocalAndRemote), now, 60);
        assert_eq!(overview.totals.week.tokens_in, 60);
        assert_eq!(overview.totals.week.tokens_out, 15);
        assert_eq!(overview.totals.week.cache_read, 20);
        assert_eq!(overview.totals.week.session_count, 3);
        assert!((overview.totals.week.estimated_usd.unwrap() - 0.075).abs() < 1e-12);
        for (day, count, sessions) in [(27, 2, 2), (28, 1, 1), (29, 2, 0)] {
            let usage = &overview.days[day].usage;
            assert_eq!(usage.tokens_in, count * 12);
            assert_eq!(usage.tokens_out, count * 3);
            assert_eq!(usage.cache_read, count * 4);
            assert_eq!(usage.session_count, sessions);
            assert!((usage.estimated_usd.unwrap() - count as f64 * 0.015).abs() < 1e-12);
            assert!(usage.cost_complete);
            assert_eq!(overview.days[day].agents[0].agent, "cursor");
        }
        let cursor = overview
            .providers
            .iter()
            .find(|p| p.provider == "cursor")
            .unwrap();
        assert_eq!(cursor.windows.week, overview.totals.week);
        assert_eq!(cursor.agents[0].agent, "cursor");

        let local = summarize(&read(UsageOrigins::Local), now, 60);
        assert_eq!(local.totals.week.tokens_in, 24);
        assert_eq!(local.totals.week.session_count, 2);
        assert!((local.totals.week.estimated_usd.unwrap() - 0.03).abs() < 1e-12);
        assert_eq!(local.days[28].usage.tokens_in, 0);
        assert_eq!(local.days[29].usage.tokens_in, 0);
        assert!(
            store
                .usage_evidence(today + 121, 3600, UsageOrigins::LocalAndRemote)
                .unwrap()
                .iter()
                .all(|row| row
                    .turn_days
                    .iter()
                    .all(|day| day.tokens.input_tokens == 999))
        );
    }

    #[test]
    fn overview_cursor_events_use_event_time_and_preserve_zero_and_unknown_cost() {
        use crate::provider_usage::{summarize, window_bounds};

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(dir.path()).unwrap();
        store
            .update_settings(|s| s.cursor_account_usage_enabled = true)
            .unwrap();
        insert_cursor_session(&store, "native", "old", "/synthetic/old.jsonl");
        let now = 1_800_000_000;
        let today = window_bounds(now, -60).today_start;
        let events = [
            StoredCursorUsageEvent {
                ts_ms: (today - 1) * 1000,
                recorded_cost_micros: None,
                ..conversation_event("unknown", Some("old"))
            },
            StoredCursorUsageEvent {
                ts_ms: today * 1000,
                recorded_cost_micros: Some(0),
                ..conversation_event("free", None)
            },
        ];
        store.insert_cursor_usage_events(&events).unwrap();
        let rows = store
            .usage_evidence(today - 86_400, -3600, UsageOrigins::LocalAndRemote)
            .unwrap();
        let overview = summarize(&rows, now, -60);
        assert_eq!(overview.days[28].usage.tokens_in, 12);
        assert_eq!(overview.days[28].usage.estimated_usd, None);
        assert!(!overview.days[28].usage.cost_complete);
        assert_eq!(overview.days[29].usage.tokens_in, 12);
        assert_eq!(overview.days[29].usage.estimated_usd, Some(0.0));
        assert!(overview.days[29].usage.cost_complete);
        assert_eq!(overview.totals.week.session_count, 1);
        let rows = store
            .usage_evidence(today, -3600, UsageOrigins::LocalAndRemote)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].account_level);
        assert!(
            store
                .usage_evidence(today + 1, -3600, UsageOrigins::LocalAndRemote)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cursor_usage_attributes_each_event_to_one_session_or_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(dir.path()).unwrap();
        store
            .update_settings(|settings| settings.cursor_account_usage_enabled = true)
            .unwrap();
        insert_cursor_session(&store, "native", "local-conv", "/home/a/local-conv.jsonl");
        insert_cursor_session(&store, "ssh:box", "other-id", "/home/b/remote-conv.jsonl");
        insert_cursor_session(&store, "wsl:Ubuntu", "shared-conv", "/x/shared-conv.jsonl");
        insert_cursor_session(&store, "native", "shared-conv", "/y/shared-conv.jsonl");
        let events = [
            conversation_event("local", Some("local-conv")),
            conversation_event("remote", Some("remote-conv")),
            conversation_event("unmatched", Some("nobody")),
            conversation_event("no-conversation", None),
            conversation_event("shared", Some("shared-conv")),
        ];
        store.insert_cursor_usage_events(&events).unwrap();
        let attributed = store.attributed_cursor_usage_events(0, i64::MAX).unwrap();
        assert_eq!(attributed.len(), events.len());
        store
            .update_settings(|settings| settings.cursor_account_usage_enabled = false)
            .unwrap();
        assert!(
            store
                .attributed_cursor_usage_events(0, i64::MAX)
                .unwrap()
                .is_empty()
        );
        let session_of = |id: &str| {
            attributed
                .iter()
                .find(|a| a.event.usage_event_id == id)
                .unwrap()
                .session
                .clone()
        };
        assert_eq!(
            session_of("local"),
            Some(SessionKey::new("native", "cursor", "local-conv"))
        );
        assert_eq!(
            session_of("remote"),
            Some(SessionKey::new("ssh:box", "cursor", "other-id"))
        );
        assert_eq!(session_of("unmatched"), None);
        assert_eq!(session_of("no-conversation"), None);
        assert_eq!(
            session_of("shared"),
            Some(SessionKey::new("native", "cursor", "shared-conv"))
        );
    }
}
