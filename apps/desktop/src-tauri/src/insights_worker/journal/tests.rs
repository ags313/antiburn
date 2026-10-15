use std::path::PathBuf;

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::*;
use crate::store::SessionRecord;

pub(crate) struct SyntheticJournal {
    _directory: tempfile::TempDir,
    pub path: PathBuf,
    pub connection: Connection,
    pub sequence: i64,
}

impl SyntheticJournal {
    pub(crate) fn new(request_bytes: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(r#"PRAGMA journal_mode = WAL;
            CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, time_created INTEGER, time_updated INTEGER);
            INSERT INTO session VALUES ('native-session', NULL, 1000, 1000);
            CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
            CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
            INSERT INTO message VALUES ('assistant', 'native-session', 1000, 1000,
                '{"role":"assistant","modelID":"synthetic","tokens":{"input":1,"output":1}}');
            CREATE TABLE event_sequence (aggregate_id TEXT PRIMARY KEY, seq INTEGER NOT NULL, owner_id TEXT);
            CREATE TABLE event (id TEXT PRIMARY KEY, aggregate_id TEXT NOT NULL, seq INTEGER NOT NULL, type TEXT NOT NULL, data TEXT NOT NULL);
            CREATE UNIQUE INDEX event_aggregate_seq_idx ON event (aggregate_id, seq);"#).unwrap();
        let mut fixture = Self {
            _directory: directory,
            path,
            connection,
            sequence: -1,
        };
        fixture.event(
            "session.created.1",
            json!({"sessionID":"native-session","info":{"id":"native-session"}}),
        );
        fixture.event(
            "message.updated.1",
            json!({"sessionID":"native-session","info":{"id":"request","role":"user"}}),
        );
        fixture.event("message.part.updated.1", json!({"sessionID":"native-session","part":{"id":"request-text","messageID":"request","type":"text","text":"x".repeat(request_bytes)}}));
        fixture.event("message.updated.1", json!({"sessionID":"native-session","info":{"id":"assistant","role":"assistant","parentID":"request"}}));
        fixture
    }

    pub(crate) fn event(&mut self, kind: &str, value: Value) {
        self.sequence += 1;
        self.connection
            .execute(
                "INSERT INTO event VALUES (?1, 'native-session', ?2, ?3, ?4)",
                params![
                    format!("event-{}", self.sequence),
                    self.sequence,
                    kind,
                    value.to_string()
                ],
            )
            .unwrap();
        self.connection
            .execute(
                "INSERT INTO event_sequence VALUES ('native-session', ?1, 'owner')
            ON CONFLICT(aggregate_id) DO UPDATE SET seq = excluded.seq",
                [self.sequence],
            )
            .unwrap();
    }

    pub(crate) fn result(&mut self, index: usize) {
        self.event("message.part.updated.1", json!({"sessionID":"native-session","part":{
            "id":format!("part-{index}"),"messageID":"assistant","sessionID":"native-session",
            "type":"tool","callID":format!("call-{index}"),"tool":"bash",
            "state":{"status":"error","input":{"command":"cargo test parser"},"error":format!("Parser failure {index}")}
        }}));
    }

    pub(crate) fn results(&mut self, range: std::ops::Range<usize>) {
        self.connection.execute_batch("BEGIN").unwrap();
        for index in range {
            self.result(index);
        }
        self.connection.execute_batch("COMMIT").unwrap();
    }

    pub(crate) fn source(&self) -> SessionSource {
        SessionSource::ProviderDb {
            agent: antiburn_local::model::AgentKind::OpenCode,
            db_path: self.path.clone(),
            session_id: "native-session".into(),
        }
    }

    pub(crate) fn record(&self, cwd: &str) -> SessionRecord {
        SessionRecord {
            key: SessionKey::new("native", "opencode", "native-session"),
            source_kind: "providerDb".into(),
            source_label: "opencode:native-session".into(),
            wsl_distro: None,
            title: Some("Synthetic journal".into()),
            title_source: Some("explicit".into()),
            cwd: Some(cwd.into()),
            surface: "cli".into(),
            updated_at_epoch: Some(time::OffsetDateTime::now_utc().unix_timestamp()),
            activity_cursor: format!("event:{}", self.sequence),
            activity_source: "event".into(),
            subagent_count: 0,
            fork_parent_session_id: None,
            source_fingerprint: Some(format!("journal:{}", self.sequence)),
        }
    }

    pub(crate) fn index(&self, store: &Store, cwd: &str) {
        store
            .upsert_sessions(&[self.record(cwd)], &crate::agents::evidence_cohort())
            .unwrap();
    }
}

pub(crate) async fn run_page(store: &Store, source: SessionSource, now: i64) -> bool {
    process_next_with(store, &|| now, &|_| {}, &|| {}, &|_| {
        let source = source.clone();
        Box::pin(async move { Some(source) })
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn empty_pages_advance_and_schedule_continuation_without_idle_spin() {
    let state = tempfile::tempdir().unwrap();
    let store = Store::open(state.path()).unwrap();
    let mut fixture = SyntheticJournal::new(16);
    fixture.connection.execute_batch("BEGIN").unwrap();
    for _ in 4..4100 {
        fixture.event(
            "session.updated.1",
            json!({"sessionID":"native-session","info":{"id":"native-session"}}),
        );
    }
    fixture.result(0);
    fixture.connection.execute_batch("COMMIT").unwrap();
    fixture.index(&store, "/synthetic");
    assert!(run_page(&store, fixture.source(), 100).await);
    assert_eq!(store.next_opencode_journal_attempt(100).unwrap(), Some(101));
    assert!(!run_page(&store, fixture.source(), 100).await);
    assert!(run_page(&store, fixture.source(), 101).await);
    assert_eq!(store.next_opencode_journal_attempt(101).unwrap(), None);
    assert!(!run_page(&store, fixture.source(), 102).await);
    let publications = store.native_tool_result_publications_after(0, 100).unwrap();
    assert_eq!(
        publications
            .iter()
            .filter(|event| event.payload.is_some())
            .count(),
        1
    );
    assert_eq!(publications.last().unwrap().live_eligible, Some(false));
    assert!(publications.iter().all(|event| event.outcome
        != crate::store::native_tool_results::ToolResultPublicationOutcome::CoverageGap));
}

#[tokio::test]
async fn projection_publication_does_not_publish_a_native_journal_tail() {
    let state = tempfile::tempdir().unwrap();
    let store = Store::open(state.path()).unwrap();
    let mut fixture = SyntheticJournal::new(16);
    fixture.result(0);
    fixture.index(&store, "/synthetic");
    let claim = store
        .claim_next_evidence(&["opencode"], 100, LEASE_SECS)
        .unwrap()
        .unwrap();
    let located = fixture.source();
    let analyzer = |agent, session: String, _wsl, claimed, signal, rows, parent| {
        let source = located.clone();
        Box::pin(async move {
            analysis::analyze_located_for_evidence(
                agent,
                &session,
                claimed,
                signal,
                rows,
                parent,
                analysis::LocatedTranscripts {
                    source,
                    children: Vec::new(),
                },
            )
            .await
        }) as crate::insights_worker::PassFuture
    };
    let mut pass = crate::insights_worker::run_record_pass_with(
        &fixture.record("/synthetic"),
        PassSignal::new(),
        claim.claim_fence,
        store.clone(),
        &analyzer,
    )
    .await;
    assert_eq!(pass.outcome, analysis::PassOutcome::Published);
    assert!(pass.tool_result_observations.is_empty());
    assert!(!pass.tool_result_observations_complete);
    pass.analysis.analyzed_generation = claim.source_generation;
    assert!(
        crate::insights_worker::apply_outcome(&store, &claim, &pass, 100)
            .unwrap()
            .applied
    );
    assert!(
        store
            .native_tool_result_publications_after(0, 10)
            .unwrap()
            .is_empty()
    );
    assert!(run_page(&store, fixture.source(), 101).await);
    let publications = store.native_tool_result_publications_after(0, 10).unwrap();
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0].live_eligible, Some(false));
}

#[tokio::test]
async fn worker_persists_prefix_changes_as_blocked_gaps_without_replaying_history() {
    let state = tempfile::tempdir().unwrap();
    let store = Store::open(state.path()).unwrap();
    let mut source = SyntheticJournal::new(16);
    source.result(0);
    source.index(&store, "/synthetic");
    assert!(run_page(&store, source.source(), 100).await);
    source.result(1);
    source.index(&store, "/synthetic");
    source.connection.execute("UPDATE event SET data = replace(data, 'Parser failure 0', 'Changed failure') WHERE seq = 4", []).unwrap();
    assert!(run_page(&store, source.source(), 101).await);
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT last_error FROM opencode_journal_delivery",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "journal-prefix-changed"
    );
    assert!(!run_page(&store, source.source(), 1_000).await);
    let events = store.native_tool_result_publications_after(0, 10).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.payload.is_some())
            .count(),
        1
    );
    assert_eq!(
        events.last().unwrap().gap_reason.as_deref(),
        Some("journal-prefix-changed")
    );
}

#[tokio::test]
async fn a_reserved_journal_sequence_does_not_stall_or_replay_the_materialized_prefix() {
    let state = tempfile::tempdir().unwrap();
    let store = Store::open(state.path()).unwrap();
    let mut source = SyntheticJournal::new(16);
    source.result(0);
    source.index(&store, "/synthetic");
    assert!(run_page(&store, source.source(), 100).await);
    source
        .connection
        .execute("UPDATE event_sequence SET seq = 100000", [])
        .unwrap();
    let mut record = source.record("/synthetic");
    record.source_fingerprint = Some("unsettled".into());
    store
        .upsert_sessions(&[record], &crate::agents::evidence_cohort())
        .unwrap();
    assert!(run_page(&store, source.source(), 101).await);
    assert!(!run_page(&store, source.source(), 102).await);
    assert!(
        store
            .lock()
            .query_row(
                "SELECT last_error IS NULL FROM opencode_journal_delivery",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    source.result(1);
    source.index(&store, "/synthetic");
    assert!(run_page(&store, source.source(), 103).await);
    let events = store.native_tool_result_publications_after(0, 10).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.payload.is_some())
            .count(),
        2
    );
    assert_eq!(events.last().unwrap().live_eligible, Some(true));
    assert!(events.iter().all(|event| event.outcome
        != crate::store::native_tool_results::ToolResultPublicationOutcome::CoverageGap));
    assert!(
        store
            .lock()
            .query_row(
                "SELECT last_error IS NULL FROM opencode_journal_delivery",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
}
