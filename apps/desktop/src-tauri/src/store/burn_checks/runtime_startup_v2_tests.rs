use antiburn_checks::check_store::CheckStore;
use antiburn_checks::compiler::compile;
use antiburn_checks::engine::{CheckEngine, DecisionEvaluator, DecisionFuture};
use antiburn_checks::source::PublishedEvidenceSource;
use antiburn_local::analysis::{
    SourceFormat, ToolCategory, ToolResultChange, ToolResultCompleteness, ToolResultContext,
    ToolResultObservation, ToolResultPosition, ToolResultStatus,
};

use super::*;
use crate::published_evidence_reader::StorePublishedEvidenceReader;
use crate::store::Incarnation;
use crate::store::native_tool_results::source_delivery::opencode_v2::{
    OpenCodeV2AdmittedSource, OpenCodeV2CaptureClaim, OpenCodeV2CaptureStart,
};

fn admitted_source(session_id: &str) -> OpenCodeV2AdmittedSource {
    OpenCodeV2AdmittedSource {
        database_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("synthetic/opencode.db"),
        file_identity: format!("synthetic-{session_id}"),
    }
}

fn admission_cursor(session_id: &str) -> String {
    let source = admitted_source(session_id);
    format!(
        "opencode-admission-v2:{}",
        serde_json::to_string(&(
            &source.database_path,
            session_id,
            &source.file_identity,
            "synthetic-scope"
        ))
        .unwrap()
    )
}

const CHECK: &str = "name: Retry\non:\n  tool_result:\ndecisions:\n  review:\n    model: typesafe/jev-1.13.0\n    input: {activity: '${{ message }}'}\n    output:\n      verdict:\n        type: choice\n        question: Is this useful?\n        criteria: {yes: Yes., no: No.}\nactions:\n  - type: status\n    input: {result: failure}\n";

struct NoEvaluator;

impl DecisionEvaluator for NoEvaluator {
    fn evaluate<'a>(
        &'a self,
        _: &'a EffectiveActivation,
        _: &'a FrozenCandidate,
        _: &'a serde_json::Value,
        _: u8,
    ) -> DecisionFuture<'a> {
        panic!("startup tests do not send provider requests")
    }
}

fn fixture() -> (
    tempfile::TempDir,
    Arc<Store>,
    Arc<StoreCheckStore>,
    EffectiveActivation,
    OpenCodeV2CaptureClaim,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory(directory.path()).unwrap());
    let scope = ActivationScope {
        environment: "native".into(),
        session_cwd: "/repo".into(),
        run: RunScope::Live,
    };
    let adapter = Arc::new(
        StoreCheckStore::new(store.clone(), "startup-owner".into(), scope.clone()).unwrap(),
    );
    let active = EffectiveActivation {
        id: ActivationId {
            scope,
            definition_id: "retry".into(),
            generation: Generation(1),
        },
        revision: [1; 32],
        definition_scope: DefinitionScope::Global,
        source_paths: vec![],
        project_root: Some(PathBuf::from("/repo")),
        compiled: Arc::new(compile(CHECK).unwrap()),
        bootstrap_observation_position: ObservationPosition(0),
        bootstrap_publication: PublicationCursor(0),
    };
    adapter
        .activate(
            "retry",
            &LifecycleTransition::Activated(active.clone()),
            Some(&active),
        )
        .unwrap();
    store
        .lock()
        .execute(
            "INSERT INTO session (environment_key, agent, session_id, source_kind, source_label,
            cwd, updated_at_epoch, first_seen_at, last_seen_at)
         VALUES ('native', 'opencode', 'v2-session', 'providerDb',
            'opencode:v2-session:open_code_v2_sqlite', '/repo', unixepoch(), 'now', 'now')",
            [],
        )
        .unwrap();
    store
        .lock()
        .execute(
            "UPDATE session SET activity_cursor=?1, source_format=?2 WHERE session_id='v2-session'",
            params![
                admission_cursor("v2-session"),
                serde_json::to_string(&SourceFormat::OpenCodeSqliteV2).unwrap()
            ],
        )
        .unwrap();
    let (incarnation, source_generation) = store
        .lock()
        .query_row(
            "SELECT incarnation, source_generation FROM session",
            [],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    let start = OpenCodeV2CaptureStart {
        key: SessionKey::new("native", "opencode", "v2-session"),
        incarnation: Incarnation(incarnation),
        source_generation,
        source_label: "opencode:v2-session:open_code_v2_sqlite".into(),
        cwd: "/repo".into(),
        admitted_source: admitted_source("v2-session"),
        baseline_sequence: Some(4),
    };
    let claim = store
        .start_opencode_v2_capture_epoch(&[start], "startup-epoch", now_epoch(), 600)
        .unwrap()
        .unwrap()
        .remove(0);
    (directory, store, adapter, active, claim)
}

fn unattached_fixture() -> (
    tempfile::TempDir,
    Arc<Store>,
    Arc<StoreCheckStore>,
    EffectiveActivation,
    OpenCodeV2CaptureClaim,
) {
    let fixture = fixture();
    fixture
        .1
        .lock()
        .execute("DELETE FROM burn_check_session_attachment", [])
        .unwrap();
    fixture
}

fn result(sequence: u64) -> ToolResultObservation {
    let position = ToolResultPosition::OpenCodeV2Event {
        sequence,
        item_index: 0,
    };
    ToolResultObservation {
        outcome: None,
        provider_executed: None,
        source_format: SourceFormat::OpenCodeSqliteV2,
        session_id: "v2-session".into(),
        record_id: Some(format!("message-{sequence}")),
        call_id: Some(format!("call-{sequence}")),
        result_id: None,
        position,
        revision: 1,
        change: ToolResultChange::Update,
        tool_name: Some("shell".into()),
        tool_category: Some(ToolCategory::Bash),
        arguments: Some(serde_json::json!({"command": "true"})),
        content: Some("ok".into()),
        status: ToolResultStatus::Success,
        finality: true,
        completeness: ToolResultCompleteness::Complete,
        context: Some(ToolResultContext {
            result_position: position,
            main_session: true,
            complete_through_event: false,
            user_messages: vec![],
        }),
    }
}

fn publish(store: &Store, claim: &OpenCodeV2CaptureClaim, sequence: u64) {
    let mut observation = result(sequence);
    observation.session_id.clone_from(&claim.key.session_id);
    assert!(
        store
            .advance_opencode_v2_capture(claim, sequence, Some(&observation), now_epoch(), 600)
            .unwrap()
    );
}

#[test]
fn v2_startup_writes_empty_attachment_and_never_seeds_current_terminals() {
    let (_directory, store, adapter, active, claim) = unattached_fixture();
    publish(&store, &claim, 5);
    let session = adapter
        .startup_sessions(&active, now_epoch())
        .unwrap()
        .remove(0);
    assert!(session.v2);
    let events = StorePublishedEvidenceReader::for_project(&store, "native", "/repo")
        .recent_results(&claim.key)
        .unwrap();
    assert!(events.is_empty());
    let before = adapter.load_source_progress(&active).unwrap();
    let attached = adapter
        .initialize_recent(&active, &session, &events)
        .unwrap()
        .unwrap();
    assert_eq!(attached.observations, 0);
    assert_eq!(attached.candidates, 0);
    let after = adapter.load_source_progress(&active).unwrap();
    assert_eq!(after.publication_cursor, before.publication_cursor);
    assert_eq!(after.observation_position, before.observation_position);
    assert!(after.selection.seen_results.is_empty());
    let branch = &after.selection.branches[&BranchId(claim.key.to_identity_json())];
    assert!(branch.results.is_empty());
    assert!(!branch.coverage_complete);
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_session_attachment",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(
        adapter
            .initialize_recent(&active, &session, &events)
            .unwrap()
            .is_none()
    );
    assert!(
        adapter
            .startup_sessions(&active, now_epoch())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn capture_attaches_live_checks_before_the_first_publication() {
    let (_directory, store, adapter, active, claim) = fixture();
    publish(&store, &claim, 5);
    let first_update = store
        .native_tool_result_publications_after(0, 20)
        .unwrap()
        .into_iter()
        .find(|event| event.payload.is_some())
        .unwrap()
        .publication_sequence as u64;
    let source = PublishedEvidenceSource::new(
        StorePublishedEvidenceReader::for_live_project(&store, "native", "/repo")
            .with_activation(&active.id),
        20,
    )
    .unwrap();
    let engine = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator));
    assert!(
        adapter
            .startup_sessions(&active, now_epoch())
            .unwrap()
            .is_empty()
    );
    let progress = adapter.load_source_progress(&active).unwrap();
    let ready = source.read_batch(progress.publication_cursor).unwrap();
    assert!(ready.events.iter().any(
        |event| matches!(event, SourceEvent::Observation { cursor, .. } if cursor.0 == first_update)
    ));
    assert_eq!(
        engine
            .apply_source_page(&active, &ready)
            .unwrap()
            .candidates,
        1
    );
    assert!(
        adapter
            .initialized_branches(&active)
            .unwrap()
            .contains(&BranchId(claim.key.to_identity_json()))
    );
    let restart = Arc::new(
        StoreCheckStore::new(store.clone(), "next-owner".into(), active.id.scope.clone()).unwrap(),
    );
    restart.register_activation(&active).unwrap();
    assert!(
        restart
            .startup_sessions(&active, now_epoch())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        restart
            .load_source_progress(&active)
            .unwrap()
            .observation_position,
        ObservationPosition(1)
    );
    assert!(source.read_batch(ready.next).unwrap().events.is_empty());
    publish(&store, &claim, 6);
    let engine = CheckEngine::new(restart.clone(), Arc::new(NoEvaluator));
    let future = source.read_batch(ready.next).unwrap();
    assert_eq!(
        engine
            .apply_source_page(&active, &future)
            .unwrap()
            .candidates,
        1
    );
    assert_eq!(restart.load_recovery(None, 10).unwrap().len(), 2);
}

#[test]
fn enabling_or_editing_a_check_during_capture_delivers_before_startup_exactly_once() {
    let (_directory, store, adapter, first, claim) = fixture();
    publish(&store, &claim, 5);
    for generation in 1..=2 {
        let mut active = first.clone();
        active.id.definition_id = "enabled-during-capture".into();
        active.id.generation = Generation(generation);
        active.revision = [generation as u8; 32];
        active.bootstrap_publication = adapter.publication_head().unwrap();
        adapter
            .activate(
                &active.id.definition_id,
                &LifecycleTransition::Activated(active.clone()),
                Some(&active),
            )
            .unwrap();
        publish(&store, &claim, 5 + generation);

        let source = PublishedEvidenceSource::new(
            StorePublishedEvidenceReader::for_live_project(&store, "native", "/repo")
                .with_activation(&active.id),
            100,
        )
        .unwrap();
        let progress = adapter.load_source_progress(&active).unwrap();
        assert_eq!(progress.publication_cursor, active.bootstrap_publication);
        assert!(
            !progress.selection.branches[&BranchId(claim.key.to_identity_json())].coverage_complete
        );
        let page = source.read_batch(progress.publication_cursor).unwrap();
        let observations = page
            .events
            .iter()
            .filter_map(|event| match event {
                SourceEvent::Observation { observation, .. } => Some(observation),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(observations.len(), 1, "generation {generation}");
        assert_eq!(
            observations[0].native_order.0,
            serde_json::to_string(&ToolResultPosition::OpenCodeV2Event {
                sequence: 5 + generation,
                item_index: 0
            })
            .unwrap()
        );
        let engine = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator));
        let outcome = engine.apply_source_page(&active, &page).unwrap();
        assert_eq!((outcome.observations, outcome.candidates), (1, 1));

        // The production loop runs startup only after it commits the source page.
        assert!(
            adapter
                .startup_sessions(&active, now_epoch())
                .unwrap()
                .is_empty()
        );
        let progress = adapter.load_source_progress(&active).unwrap();
        assert_eq!(progress.observation_position, ObservationPosition(1));
        assert!(
            source
                .read_batch(progress.publication_cursor)
                .unwrap()
                .events
                .is_empty()
        );
        let count: i64 = store.lock().query_row(
            "SELECT COUNT(*) FROM burn_check_candidate WHERE check_id = ?1 AND generation = ?2 AND state = 'pending'",
            params![active.id.definition_id, generation], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
        let current = store
            .opencode_v2_capture_claims(std::slice::from_ref(&claim.key))
            .unwrap();
        assert_eq!(current[0].capture_epoch, claim.capture_epoch);
        assert_eq!(current[0].claim_fence, claim.claim_fence);
    }
}

#[test]
fn activation_attaches_only_current_root_v2_captures_in_its_scope() {
    for mutation in [
        "UPDATE session SET incarnation = incarnation + 1",
        "UPDATE session SET source_format = '\"open_code_sqlite_v2\"'",
        "UPDATE session SET source_kind = 'file'",
        "UPDATE session SET source_label = 'other-source'",
        "UPDATE session SET cwd = '/other'",
        "UPDATE session SET wsl_distro = 'synthetic'",
        "INSERT INTO session_relation VALUES ('native', 'opencode', 'v2-session', 'forkParent', 'parent', NULL)",
        "INSERT INTO session_relation VALUES ('native', 'opencode', 'parent', 'subagent', 'v2-session', NULL)",
        "UPDATE opencode_v2_capture SET incarnation = incarnation + 1",
        "UPDATE opencode_v2_capture SET source_label = 'other-source'",
        "UPDATE opencode_v2_capture SET cwd = '/other'",
        "UPDATE opencode_v2_capture SET capture_epoch = ''",
        "UPDATE opencode_v2_capture SET lease_expires_at_epoch = NULL",
        "UPDATE opencode_v2_capture SET lease_expires_at_epoch = unixepoch()",
    ] {
        let (_directory, store, adapter, mut active, _claim) = fixture();
        store.lock().execute(mutation, []).unwrap();
        active.id.definition_id = "scope-test".into();
        active.bootstrap_publication = adapter.publication_head().unwrap();
        adapter
            .activate(
                &active.id.definition_id,
                &LifecycleTransition::Activated(active.clone()),
                Some(&active),
            )
            .unwrap();
        let count: i64 = store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_session_attachment WHERE check_id = ?1",
                [&active.id.definition_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{mutation}");
    }
}

#[test]
fn activation_does_not_attach_another_environment_cwd_or_replay() {
    for scope in [
        ActivationScope {
            environment: "wsl:synthetic".into(),
            session_cwd: "/repo".into(),
            run: RunScope::Live,
        },
        ActivationScope {
            environment: "native".into(),
            session_cwd: "/other".into(),
            run: RunScope::Live,
        },
        ActivationScope {
            environment: "native".into(),
            session_cwd: "/repo".into(),
            run: RunScope::Replay("fixture".into()),
        },
    ] {
        let (_directory, store, _adapter, mut active, _claim) = fixture();
        active.id.definition_id = "different-scope".into();
        active.id.scope = scope.clone();
        let adapter = StoreCheckStore::new(store.clone(), "scope-owner".into(), scope).unwrap();
        active.bootstrap_publication = adapter.publication_head().unwrap();
        adapter
            .activate(
                &active.id.definition_id,
                &LifecycleTransition::Activated(active.clone()),
                Some(&active),
            )
            .unwrap();
        let count: i64 = store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_session_attachment WHERE check_id = ?1",
                [&active.id.definition_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{:?}", active.id.scope);
    }
}

#[test]
fn attachment_failure_rolls_back_the_new_activation() {
    let (_directory, store, adapter, mut active, _claim) = fixture();
    active.id.definition_id = "attachment-failure".into();
    active.bootstrap_publication = adapter.publication_head().unwrap();
    store
        .lock()
        .execute_batch(
            "CREATE TRIGGER reject_new_attachment BEFORE INSERT ON burn_check_session_attachment
         WHEN NEW.check_id = 'attachment-failure'
         BEGIN SELECT RAISE(ABORT, 'synthetic attachment failure'); END;",
        )
        .unwrap();
    assert!(
        adapter
            .activate(
                &active.id.definition_id,
                &LifecycleTransition::Activated(active.clone()),
                Some(&active)
            )
            .is_err()
    );
    let count: i64 = store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM burn_check_activation WHERE check_id = ?1",
            [&active.id.definition_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn v2_startup_receipt_fences_capture_replacement_source_and_scope() {
    for sql in [
        "UPDATE opencode_v2_capture SET capture_epoch = 'replacement'",
        "UPDATE opencode_v2_capture SET claim_fence = claim_fence + 1",
        "UPDATE opencode_v2_capture SET incarnation = incarnation + 1",
        "UPDATE opencode_v2_capture SET source_label = 'other-source'",
        "UPDATE opencode_v2_capture SET cwd = '/other'",
        "UPDATE opencode_v2_capture SET baseline_sequence = baseline_sequence + 1",
        "UPDATE opencode_v2_capture SET capture_epoch = ''",
        "DELETE FROM opencode_v2_capture",
        "UPDATE session SET incarnation = incarnation + 1",
        "UPDATE session SET source_label = 'opencode:v2-session'",
        "UPDATE session SET cwd = '/other'",
        "UPDATE session SET source_kind = 'file'",
        "UPDATE session SET wsl_distro = 'synthetic-distro'",
    ] {
        let (_directory, store, adapter, active, _claim) = unattached_fixture();
        let receipt = adapter
            .startup_sessions(&active, now_epoch())
            .unwrap()
            .remove(0);
        store.lock().execute(sql, []).unwrap();
        assert!(
            adapter
                .initialize_recent(&active, &receipt, &[])
                .unwrap()
                .is_none(),
            "{sql}"
        );
        assert_eq!(
            store
                .lock()
                .query_row(
                    "SELECT COUNT(*) FROM burn_check_session_attachment",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0,
            "{sql}"
        );
    }
}

#[test]
fn ineligible_v2_branch_does_not_block_a_healthy_project_branch() {
    for relation in [
        "INSERT INTO session_relation VALUES ('native', 'opencode', 'v2-session', 'forkParent', 'parent', NULL)",
        "INSERT INTO session_relation VALUES ('native', 'opencode', 'parent', 'subagent', 'v2-session', NULL)",
    ] {
        let (_directory, store, adapter, active, claim) = fixture();
        publish(&store, &claim, 5);
        store.lock().execute(relation, []).unwrap();
        assert!(
            adapter
                .startup_sessions(&active, now_epoch())
                .unwrap()
                .is_empty()
        );

        store.lock().execute(
            "INSERT INTO session (environment_key, agent, session_id, source_kind, source_label,
              cwd, updated_at_epoch, first_seen_at, last_seen_at)
             VALUES ('native', 'opencode', 'healthy', 'providerDb',
              'opencode:healthy:open_code_v2_sqlite', '/repo', unixepoch(), 'now', 'now')", [],
        ).unwrap();
        store
            .lock()
            .execute(
                "UPDATE session SET activity_cursor=?1, source_format=?2 WHERE session_id='healthy'",
                params![admission_cursor("healthy"), serde_json::to_string(&SourceFormat::OpenCodeSqliteV2).unwrap()],
            )
            .unwrap();
        let (incarnation, source_generation) = store
            .lock()
            .query_row(
                "SELECT incarnation, source_generation FROM session WHERE session_id = 'healthy'",
                [],
                |row| Ok((Incarnation(row.get(0)?), row.get(1)?)),
            )
            .unwrap();
        let healthy = store
            .start_opencode_v2_capture_epoch(
                &[OpenCodeV2CaptureStart {
                    key: SessionKey::new("native", "opencode", "healthy"),
                    incarnation,
                    source_generation,
                    source_label: "opencode:healthy:open_code_v2_sqlite".into(),
                    cwd: "/repo".into(),
                    admitted_source: admitted_source("healthy"),
                    baseline_sequence: Some(4),
                }],
                "healthy-epoch",
                now_epoch(),
                600,
            )
            .unwrap()
            .unwrap()
            .remove(0);
        let startup = adapter.startup_sessions(&active, now_epoch()).unwrap();
        assert!(startup.is_empty());
        publish(&store, &healthy, 5);

        let source = PublishedEvidenceSource::new(
            StorePublishedEvidenceReader::for_live_project(&store, "native", "/repo")
                .with_activation(&active.id),
            100,
        )
        .unwrap();
        let page = source.read_batch(PublicationCursor(0)).unwrap();
        assert!(!page.has_more);
        assert!(
            page.events
                .iter()
                .any(|event| matches!(event, SourceEvent::Ignored { .. }))
        );
        let branches: Vec<_> = page
            .events
            .iter()
            .filter_map(|event| match event {
                SourceEvent::Observation { observation, .. } => Some(&observation.branch),
                _ => None,
            })
            .collect();
        assert_eq!(branches, [&BranchId(healthy.key.to_identity_json())]);
    }
}

#[test]
fn v2_startup_accepts_ordinary_generation_changes_and_capture_progress() {
    let (_directory, store, adapter, active, claim) = unattached_fixture();
    store
        .lock()
        .execute(
            "UPDATE session SET source_generation = source_generation + 1, updated_at_epoch = 0",
            [],
        )
        .unwrap();
    store
        .lock()
        .execute(
            "UPDATE session SET activity_cursor=?1 WHERE session_id='healthy-session'",
            [admission_cursor("healthy-session")],
        )
        .unwrap();
    let receipt = adapter
        .startup_sessions(&active, now_epoch())
        .unwrap()
        .remove(0);
    publish(&store, &claim, 5);
    store
        .lock()
        .execute(
            "UPDATE session SET source_generation = source_generation + 1",
            [],
        )
        .unwrap();
    assert!(
        adapter
            .initialize_recent(&active, &receipt, &[])
            .unwrap()
            .is_some()
    );
}

#[test]
fn v2_empty_attachment_accepts_unleased_receipt_without_live_report_readiness() {
    for lease in ["NULL", "unixepoch() - 1"] {
        let (_directory, store, adapter, active, claim) = unattached_fixture();
        let receipt = adapter
            .startup_sessions(&active, now_epoch())
            .unwrap()
            .remove(0);
        store
            .lock()
            .execute(
                &format!("UPDATE opencode_v2_capture SET lease_expires_at_epoch = {lease}"),
                [],
            )
            .unwrap();
        store.lock().execute("UPDATE session SET updated_at_epoch = 0, source_generation = source_generation + 1", []).unwrap();
        let ended = adapter
            .startup_sessions(&active, now_epoch())
            .unwrap()
            .remove(0);
        assert_eq!(receipt.receipt, ended.receipt);
        let outcome = adapter
            .initialize_recent(&active, &receipt, &[])
            .unwrap()
            .unwrap();
        assert_eq!(outcome.observations, 0);
        assert_eq!(outcome.candidates, 0);
        let source = PublishedEvidenceSource::new(
            StorePublishedEvidenceReader::for_live_project(&store, "native", "/repo")
                .with_activation(&active.id),
            20,
        )
        .unwrap();
        let engine = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator));
        let batch = source.read_batch(PublicationCursor(0)).unwrap();
        engine.apply_source_page(&active, &batch).unwrap();
        assert!(
            adapter
                .initialized_branches(&active)
                .unwrap()
                .contains(&BranchId(claim.key.to_identity_json()))
        );
        let ready = crate::insights_report::installed_checks::source_ready_sql(1, 2, 3);
        let live_ready: bool = store
            .lock()
            .query_row(
                &format!(
                    "SELECT ({ready}) FROM session s
                LEFT JOIN session_evidence e USING(environment_key, agent, session_id)"
                ),
                params![
                    antiburn_local::analysis::PARSER_REVISION,
                    antiburn_local::analysis::ANALYZER_REVISION,
                    antiburn_local::analysis::EVIDENCE_SCHEMA_REVISION,
                ],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!live_ready);
    }
}

#[test]
fn v2_disconnect_before_attachment_drains_publications_and_unblocks_healthy_session() {
    let (_directory, store, adapter, active, claim) = fixture();
    publish(&store, &claim, 5);
    assert_eq!(
        store
            .mark_opencode_v2_capture_gap(
                std::slice::from_ref(&claim),
                "event-feed-ended",
                now_epoch()
            )
            .unwrap(),
        1
    );
    store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = 0, source_generation = source_generation + 1",
            [],
        )
        .unwrap();
    store
        .lock()
        .execute(
            "INSERT INTO session (environment_key, agent, session_id, source_kind, source_label,
            cwd, updated_at_epoch, first_seen_at, last_seen_at)
         VALUES ('native', 'opencode', 'healthy-session', 'providerDb',
            'opencode:healthy-session:open_code_v2_sqlite', '/repo', unixepoch(), 'now', 'now')",
            [],
        )
        .unwrap();
    store
        .lock()
        .execute(
            "UPDATE session SET activity_cursor = ?1, source_format = ?2
              WHERE environment_key = 'native' AND agent = 'opencode'
                AND session_id = 'healthy-session'",
            params![
                admission_cursor("healthy-session"),
                serde_json::to_string(&SourceFormat::OpenCodeSqliteV2).unwrap()
            ],
        )
        .unwrap();
    let (healthy_incarnation, healthy_source_generation) = store
        .lock()
        .query_row(
            "SELECT incarnation, source_generation FROM session
              WHERE environment_key = 'native' AND agent = 'opencode'
                AND session_id = 'healthy-session'",
            [],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    let healthy = store
        .start_opencode_v2_capture_epoch(
            &[OpenCodeV2CaptureStart {
                key: SessionKey::new("native", "opencode", "healthy-session"),
                incarnation: Incarnation(healthy_incarnation),
                source_generation: healthy_source_generation,
                source_label: "opencode:healthy-session:open_code_v2_sqlite".into(),
                cwd: "/repo".into(),
                admitted_source: admitted_source("healthy-session"),
                baseline_sequence: Some(4),
            }],
            "healthy-epoch",
            now_epoch(),
            600,
        )
        .unwrap()
        .unwrap()
        .remove(0);
    publish(&store, &healthy, 5);
    let source = PublishedEvidenceSource::new(
        StorePublishedEvidenceReader::for_live_project(&store, "native", "/repo")
            .with_activation(&active.id),
        20,
    )
    .unwrap();
    let engine = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator));
    let sessions = adapter.startup_sessions(&active, now_epoch()).unwrap();
    assert!(sessions.is_empty());
    let ready = source.read_batch(PublicationCursor(0)).unwrap();
    let outcome = engine.apply_source_page(&active, &ready).unwrap();
    assert_eq!(outcome.observations, 2);
    assert_eq!(outcome.candidates, 1);
    assert!(source.read_batch(ready.next).unwrap().events.is_empty());
    let pending = adapter.load_recovery(None, 10).unwrap();
    assert_eq!(pending.len(), 1);
    let RecoveryWork::Pending(candidate) = &pending[0].work else {
        panic!("healthy evidence must retain one pending candidate")
    };
    assert_eq!(candidate.branch, BranchId(healthy.key.to_identity_json()));
    let initialized = adapter.initialized_branches(&active).unwrap();
    assert!(initialized.contains(&BranchId(claim.key.to_identity_json())));
    assert!(initialized.contains(&BranchId(healthy.key.to_identity_json())));
}
