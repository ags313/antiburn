use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use antiburn_checks::check_store::{CandidateClaim, CheckStore, ClaimOutcome};
use antiburn_checks::compiler::compile;
use antiburn_checks::decision::{DecisionAttempt, DecisionInput};
use antiburn_checks::definition_lifecycle::{EffectiveActivation, LifecycleTransition};
use antiburn_checks::discovery::DefinitionScope;
use antiburn_checks::domain::{
    ActivationId, ActivationScope, FrozenCandidate, Generation, ObservationPosition, RunScope,
};
use antiburn_checks::engine::{
    CheckEngine, DecisionEvaluator, DecisionFuture, PreparationValidationPurpose, RunOutcome,
    StartOutcome,
};
use antiburn_checks::expression_eval::EvaluationBudget;
use antiburn_checks::fit::PreparationFailure;
use antiburn_checks::prepared_assessment::PreparedAssessment;
use antiburn_checks::source::{PublicationCursor, PublishedEvidenceSource};
use antiburn_local::analysis::opencode_v2_capture::read_capture_reconciliation_for_scope;
use antiburn_local::analysis::*;

use super::*;
use crate::check_store::StoreCheckStore;
use crate::published_evidence_reader::StorePublishedEvidenceReader;
use crate::store::native_tool_results::{evidence, observers};

struct NoProvider;

impl DecisionEvaluator for NoProvider {
    fn prepare(
        &self,
        _: &EffectiveActivation,
        _: Vec<DecisionInput>,
        _: EvaluationBudget,
    ) -> Result<PreparedAssessment, PreparationFailure> {
        panic!("capture-gap tests must not prepare provider requests")
    }

    fn validate_prepared(
        &self,
        _: &EffectiveActivation,
        _: &PreparedAssessment,
        _: PreparationValidationPurpose,
    ) -> Result<(), PreparationFailure> {
        panic!("capture-gap tests must not validate provider requests")
    }

    fn evaluate_prepared<'a>(
        &'a self,
        _: &'a EffectiveActivation,
        _: &'a FrozenCandidate,
        _: &'a PreparedAssessment,
        _: &'a DecisionAttempt,
    ) -> DecisionFuture<'a> {
        panic!("capture-gap tests must not send provider requests")
    }
}

struct Runtime {
    adapter: Arc<StoreCheckStore>,
    active: EffectiveActivation,
    engine: CheckEngine<StoreCheckStore, NoProvider>,
    cursor: PublicationCursor,
}

impl Runtime {
    fn new(store: Arc<Store>, selector: &str, model: bool) -> Self {
        store
            .set_internal_value_checked("internal:yamlBurnChecksDiscoveryEnabledV1", "true")
            .unwrap();
        let scope = ActivationScope {
            environment: "native".into(),
            session_cwd: "/synthetic".into(),
            run: RunScope::Live,
        };
        let adapter = Arc::new(
            StoreCheckStore::new(store, "capture-gap-test".into(), scope.clone()).unwrap(),
        );
        let decisions = if model {
            "decisions:\n  review:\n    input: {session: '${{ session.id }}'}\n    output:\n      verdict:\n        type: choice\n        question: Is this useful?\n        criteria: {yes: Yes., no: No.}\n"
        } else {
            ""
        };
        let source = format!(
            "name: Capture gap\non:\n  {selector}:\n  cooldown: 0s\n{decisions}actions:\n  - type: status\n    input: {{result: failure}}\n"
        );
        let active = EffectiveActivation {
            id: ActivationId {
                scope,
                definition_id: "capture-gap".into(),
                generation: Generation(1),
            },
            revision: [7; 32],
            definition_scope: DefinitionScope::Global,
            source_paths: Vec::new(),
            project_root: None,
            compiled: Arc::new(compile(&source).unwrap()),
            bootstrap_observation_position: ObservationPosition(0),
            bootstrap_publication: PublicationCursor(0),
        };
        adapter
            .activate(
                &active.id.definition_id,
                &LifecycleTransition::Activated(active.clone()),
                Some(&active),
            )
            .unwrap();
        Self {
            engine: CheckEngine::new(adapter.clone(), Arc::new(NoProvider)),
            adapter,
            active,
            cursor: PublicationCursor(0),
        }
    }

    fn sync(&mut self, store: &Store) -> usize {
        let mut candidates = 0;
        for _ in 0..8 {
            let page = PublishedEvidenceSource::new(
                StorePublishedEvidenceReader::for_project(store, "native", "/synthetic")
                    .with_activation(&self.active.id),
                100,
            )
            .unwrap()
            .read_batch(self.cursor)
            .unwrap();
            candidates += self
                .engine
                .apply_source_page(&self.active, &page)
                .unwrap()
                .candidates;
            self.cursor = page.next;
            if !page.has_more {
                return candidates;
            }
        }
        panic!("capture-gap fixture exceeded its source-page bound")
    }

    fn accept(&self, now: u64) {
        let StartOutcome::Started(mut task) = self.engine.start_one_due(&self.active, now).unwrap()
        else {
            panic!("the captured event must have a due candidate")
        };
        for _ in 0..8 {
            if let Poll::Ready(settlement) =
                task.as_mut().poll(&mut Context::from_waker(Waker::noop()))
            {
                assert_eq!(settlement.unwrap().outcome, RunOutcome::Accepted);
                return;
            }
        }
        panic!("local status work must settle without I/O")
    }
}

fn position(sequence: u64) -> ToolResultPosition {
    ToolResultPosition::OpenCodeV2Event {
        sequence,
        item_index: 0,
    }
}

fn context(sequence: u64) -> NativeEventContext {
    NativeEventContext {
        first_eligible_position: position(sequence),
        main_session: true,
        complete_through_event: false,
        user_messages: Vec::new(),
    }
}

fn public_messages(session_id: &str) -> Vec<NativeMessageObservation> {
    vec![
        NativeMessageObservation::User(UserMessageObservation {
            revision: 5,
            context: context(5),
            message: UserMessage {
                source_format: SourceFormat::OpenCodeSqliteV2,
                session_id: session_id.into(),
                message_id: "public-user".into(),
                position: UserMessagePosition::OpenCodeV2Event { sequence: 5 },
                origin: UserMessageOrigin::OpenCodeUserText,
                scope: UserMessageScope::MainSession,
                change: UserMessageChange::Present,
                completeness: UserMessageCompleteness::Complete,
                proof_revision: 1,
                parts: vec![UserMessagePart {
                    item_index: 0,
                    native_part_id: None,
                    text: "Inspect the synthetic input.".into(),
                }],
            },
        }),
        NativeMessageObservation::Public(PublicMessageObservation {
            source_format: SourceFormat::OpenCodeSqliteV2,
            session_id: session_id.into(),
            record_id: "public-assistant".into(),
            part_id: Some("text".into()),
            call_id: None,
            position: position(6),
            revision: 6,
            change: ToolResultChange::Update,
            eligible: true,
            completeness: ToolResultCompleteness::Complete,
            context: Some(context(6)),
            payload: PublicMessagePayload::AssistantText {
                parts: vec![UserMessagePart {
                    item_index: 0,
                    native_part_id: Some("text".into()),
                    text: "The synthetic input is ready.".into(),
                }],
                generator: NativeRequestAttribution::default(),
            },
        }),
        NativeMessageObservation::Public(PublicMessageObservation {
            source_format: SourceFormat::OpenCodeSqliteV2,
            session_id: session_id.into(),
            record_id: "public-call".into(),
            part_id: Some("call".into()),
            call_id: Some("call".into()),
            position: position(7),
            revision: 7,
            change: ToolResultChange::Update,
            eligible: true,
            completeness: ToolResultCompleteness::Complete,
            context: Some(context(7)),
            payload: PublicMessagePayload::ToolCall {
                tool_name: "read".into(),
                input: Some(NativeToolInput::Text("synthetic.txt".into())),
                facts: None,
                provider_executed: Some(false),
            },
        }),
    ]
}

fn publish_public_cycle(store: &Store, claim: &OpenCodeV2CaptureClaim) {
    let messages = public_messages(&claim.key.session_id);
    for (index, message) in messages.iter().enumerate() {
        assert!(
            store
                .advance_opencode_v2_evidence(
                    claim,
                    5 + index as u64,
                    &NativeEvidenceBatch {
                        source_format: SourceFormat::OpenCodeSqliteV2,
                        history_complete: false,
                        events: vec![NativeEvidenceEvent {
                            live_eligible: true,
                            fact: NativeEvidenceFact::Message(message.clone()),
                        }],
                    },
                    101,
                    60,
                )
                .unwrap()
        );
    }
    assert!(
        store
            .advance_opencode_v2_evidence(
                claim,
                8,
                &NativeEvidenceBatch {
                    source_format: SourceFormat::OpenCodeSqliteV2,
                    history_complete: false,
                    events: vec![NativeEvidenceEvent {
                        live_eligible: true,
                        fact: NativeEvidenceFact::ResponseCycle(ResponseCycleObservation {
                            source_format: SourceFormat::OpenCodeSqliteV2,
                            session_id: claim.key.session_id.clone(),
                            key: NativeCycleKey::OpenCodeV2Execution {
                                start_event_id: "execution-start".into(),
                                start_sequence: 5,
                            },
                            start_position: position(5),
                            position: position(8),
                            revision: 8,
                            change: ToolResultChange::Update,
                            status: ResponseCycleStatus::Completed,
                            native_reason: None,
                            native_error_code: None,
                            started_at_ms: Some(100_000),
                            ended_at_ms: Some(101_000),
                            requester: NativeRequestAttribution::default(),
                            context: context(8),
                            messages,
                        }),
                    }],
                },
                101,
                60,
            )
            .unwrap()
    );
}

fn evidence_validity(store: &Store) -> (i64, i64) {
    store
        .lock()
        .query_row(
            "SELECT h.evidence_valid, s.evidence_valid FROM burn_check_assessment_history h
              JOIN burn_check_current_status s USING(assessment_id) WHERE h.check_id = 'capture-gap'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn source_database(path: &std::path::Path) -> rusqlite::Connection {
    let source = rusqlite::Connection::open(path).unwrap();
    source.execute_batch(
        r#"PRAGMA journal_mode = WAL;
         CREATE TABLE session_v2 (id TEXT PRIMARY KEY, parent_id TEXT, fork_session_id TEXT,
           directory TEXT, title TEXT, time_created INTEGER, time_updated INTEGER);
         CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT,
           seq INTEGER, time_created INTEGER, time_updated INTEGER, data TEXT);
         CREATE UNIQUE INDEX native_message_seq ON session_message(session_id, seq);
         CREATE TABLE event_sequence (aggregate_id TEXT PRIMARY KEY NOT NULL, seq INTEGER NOT NULL);
         INSERT INTO session_v2 VALUES ('native-session', NULL, NULL, '/synthetic', NULL, 1, 1);
         INSERT INTO event_sequence VALUES ('native-session', 20);
         INSERT INTO session_message VALUES ('public-assistant', 'native-session', 'assistant', 6, 1, 1,
           '{"content":[{"type":"text","text":"The synthetic input is ready."}]}');"#,
    ).unwrap();
    source
}

fn reconnect(
    store: &Store,
    start: &mut OpenCodeV2CaptureStart,
    path: &std::path::Path,
) -> OpenCodeV2CaptureClaim {
    let records = store.opencode_v2_live_candidates(103, 10).unwrap();
    let targets = store.opencode_v2_reconciliation_targets(&records).unwrap();
    let snapshot = read_capture_reconciliation_for_scope(
        path,
        SourceFormat::OpenCodeSqliteV2,
        &[(start.key.session_id.clone(), start.cwd.clone())],
        &targets,
        &|| false,
    )
    .unwrap();
    start.baseline_sequence = snapshot.baseline[&start.key.session_id];
    store
        .start_opencode_v2_capture_epoch_reconciled(
            std::slice::from_ref(start),
            &snapshot,
            "epoch-b",
            103,
            60,
        )
        .unwrap()
        .unwrap()
        .remove(0)
}

#[test]
fn disconnect_and_source_edit_or_deletion_invalidates_accepted_public_and_cycle_evidence() {
    for selector in ["user_message", "assistant_message", "tool_call", "turn_end"] {
        for mutation in ["edit", "delete"] {
            let (directory, store, _journal, mut start) = setup();
            let store = Arc::new(store);
            let mut runtime = Runtime::new(store.clone(), selector, false);
            let claim = started(&store, &start);
            publish_public_cycle(&store, &claim);
            assert_eq!(runtime.sync(&store), 1, "{selector}/{mutation}");
            runtime.accept(101_000);
            assert_eq!(evidence_validity(&store), (1, 1));
            let frozen: String = store.lock().query_row(
                "SELECT frozen_input_json FROM burn_check_candidate WHERE check_id = 'capture-gap'", [], |row| row.get(0),
            ).unwrap();
            let publications = store.native_evidence_publications_after(0, 100).unwrap();
            let path = directory.path().join("source.sqlite");
            let source = source_database(&path);

            store
                .mark_opencode_v2_capture_gap(&[claim], "capture-lost", 102)
                .unwrap();
            assert_eq!(evidence_validity(&store), (0, 0));
            source.execute(if mutation == "delete" {
                "DELETE FROM session_message WHERE id = 'public-assistant'"
            } else {
                "UPDATE session_message SET data = '{\"content\":[]}' WHERE id = 'public-assistant'"
            }, []).unwrap();
            let next = reconnect(&store, &mut start, &path);
            assert_eq!(runtime.sync(&store), 0);
            assert_eq!(evidence_validity(&store), (0, 0));
            assert_eq!(store.lock().query_row(
                "SELECT frozen_input_json FROM burn_check_candidate WHERE check_id = 'capture-gap'", [], |row| row.get::<_, String>(0),
            ).unwrap(), frozen);
            assert_eq!(
                &store.native_evidence_publications_after(0, 100).unwrap()[..publications.len()],
                publications.as_slice()
            );
            assert_eq!(
                store
                    .lock()
                    .query_row(
                        "SELECT COUNT(*) FROM native_tool_result_current WHERE is_removed = 1",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                4
            );

            let mut replay = public_messages(&start.key.session_id).remove(1);
            let NativeMessageObservation::Public(message) = &mut replay else {
                unreachable!()
            };
            message.position = position(21);
            message.revision = 21;
            assert!(
                store
                    .advance_opencode_v2_evidence(
                        &next,
                        21,
                        &NativeEvidenceBatch {
                            source_format: SourceFormat::OpenCodeSqliteV2,
                            history_complete: false,
                            events: vec![NativeEvidenceEvent {
                                live_eligible: false,
                                fact: NativeEvidenceFact::Message(replay)
                            }],
                        },
                        104,
                        60
                    )
                    .unwrap()
            );
            assert_eq!(runtime.sync(&store), 0);
            assert_eq!(evidence_validity(&store), (0, 0));
        }
    }
}

#[test]
fn capture_gap_stales_queued_or_claimed_public_work_before_provider_dispatch() {
    for selector in ["assistant_message", "turn_end"] {
        for claimed in [false, true] {
            let (directory, store, _journal, mut start) = setup();
            let store = Arc::new(store);
            let mut runtime = Runtime::new(store.clone(), selector, true);
            let claim = started(&store, &start);
            publish_public_cycle(&store, &claim);
            assert_eq!(runtime.sync(&store), 1);
            let frozen: String = store.lock().query_row(
                "SELECT frozen_input_json FROM burn_check_candidate WHERE check_id = 'capture-gap'", [], |row| row.get(0),
            ).unwrap();
            let candidate: FrozenCandidate = serde_json::from_str(&frozen).unwrap();
            let pending_claim = claimed.then(|| {
                let CandidateClaim::Claimed(claim) = runtime
                    .adapter
                    .claim_due(&candidate, 101_000, false)
                    .unwrap()
                else {
                    panic!("the candidate must be claimable before the gap")
                };
                claim
            });
            store
                .mark_opencode_v2_capture_gap(&[claim], "capture-lost", 102)
                .unwrap();
            let path = directory.path().join("source.sqlite");
            let source = source_database(&path);
            source.execute("DELETE FROM session_message", []).unwrap();
            reconnect(&store, &mut start, &path);
            if let Some(claim) = pending_claim {
                assert_eq!(
                    runtime
                        .adapter
                        .record_dispatch_intent(
                            &claim,
                            104_000,
                            Duration::ZERO,
                            &DecisionAttempt::new("review", 1).unwrap(),
                        )
                        .unwrap(),
                    ClaimOutcome::Stale
                );
            } else {
                assert!(matches!(
                    runtime
                        .adapter
                        .claim_due(&candidate, 104_000, false)
                        .unwrap(),
                    CandidateClaim::Stale
                ));
            }
            assert!(matches!(
                runtime
                    .engine
                    .start_one_due(&runtime.active, 104_000)
                    .unwrap(),
                StartOutcome::Stale | StartOutcome::NotDue
            ));
            assert_eq!(
                store
                    .lock()
                    .query_row(
                        "SELECT COUNT(*) FROM burn_check_evaluation_attempt",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
        }
    }
}

#[test]
fn capture_gap_keeps_independent_results_for_exact_reconciliation() {
    for retained in [true, false] {
        let (directory, store, _journal, mut start) = setup();
        let store = Arc::new(store);
        let mut runtime = Runtime::new(store.clone(), "tool_result", false);
        let claim = started(&store, &start);
        publish_public_cycle(&store, &claim);
        assert!(
            store
                .advance_opencode_v2_capture(&claim, 9, Some(&result(9, false)), 101, 60)
                .unwrap()
        );
        assert_eq!(runtime.sync(&store), 1);
        runtime.accept(101_000);
        let before = store
            .native_tool_results_current(&start.key)
            .unwrap()
            .remove(0);
        assert_eq!(evidence_validity(&store), (1, 1));
        store
            .mark_opencode_v2_capture_gap(&[claim], "capture-lost", 102)
            .unwrap();
        assert_eq!(evidence_validity(&store), (1, 1));
        assert_eq!(
            store.native_tool_results_current(&start.key).unwrap(),
            vec![before.clone()]
        );
        let path = directory.path().join("source.sqlite");
        let source = source_database(&path);
        if retained {
            source.execute(
                "INSERT INTO session_message VALUES ('event-9', 'native-session', 'assistant', 9, 1, 1, ?1)",
                [json!({"content":[{"type":"tool","id":"call-9","name":"bash",
                    "state":{"status":"completed","input":{"command":"true"},
                        "content":[{"type":"text","text":"ok"}]}}]}).to_string()],
            ).unwrap();
        }
        reconnect(&store, &mut start, &path);
        runtime.sync(&store);
        let after = store
            .native_tool_results_current(&start.key)
            .unwrap()
            .remove(0);
        assert_eq!(after.removed, !retained);
        assert_eq!(after.result.context, before.result.context);
        assert_eq!(
            evidence_validity(&store),
            if retained { (1, 1) } else { (0, 0) }
        );
    }
}

#[test]
fn capture_gap_preserves_an_accepted_idle_boundary() {
    let (_directory, store, _journal, start) = setup();
    let store = Arc::new(store);
    let mut runtime = Runtime::new(store.clone(), "session_idle", false);
    let claim = started(&store, &start);
    publish_public_cycle(&store, &claim);
    {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        tx.execute(
            "INSERT INTO burn_check_session_attachment(check_id, activation_scope_key, generation, branch_key, incarnation)
              VALUES (?1, ?2, 1, ?3, ?4)",
            params![runtime.active.id.definition_id, StoreCheckStore::activation_scope_key(&runtime.active.id.scope),
                start.key.to_identity_json(), start.incarnation.0],
        ).unwrap();
        observers::attach_in(
            &tx,
            &runtime.active.id,
            &start.key,
            start.incarnation.0 as i64,
            "receipt",
            1000,
        )
        .unwrap();
        observers::frontier_in(&tx, &start.key, "receipt", true, 121_000).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(runtime.sync(&store), 1);
    runtime.accept(121_000);
    assert_eq!(evidence_validity(&store), (1, 1));
    store
        .mark_opencode_v2_capture_gap(&[claim], "capture-lost", 122)
        .unwrap();
    runtime.sync(&store);
    assert_eq!(evidence_validity(&store), (1, 1));
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT state FROM burn_check_candidate WHERE check_id = 'capture-gap'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "accepted"
    );
}

#[test]
fn capture_gap_fences_stale_claims_and_preserves_another_sessions_bindings() {
    let (_directory, store, journal, start) = setup();
    let claim = started(&store, &start);
    let mut record = journal.record("/synthetic");
    record.key.session_id = "healthy-session".into();
    record.source_label = "opencode:healthy-session:open_code_v2_sqlite".into();
    record.activity_cursor = test_admission_cursor(&start.admitted_source, "healthy-session");
    store
        .upsert_sessions(&[record.clone()], &crate::agents::evidence_cohort())
        .unwrap();
    let mut other = start.clone();
    other.key = record.key;
    other.source_label = record.source_label;
    (other.incarnation, other.source_generation) = store.lock().query_row(
        "SELECT incarnation, source_generation FROM session WHERE session_id = 'healthy-session'", [],
        |row| Ok((Incarnation(row.get(0)?), row.get(1)?)),
    ).unwrap();
    let other_claim = store
        .start_opencode_v2_capture_epoch(&[other.clone()], "healthy", 100, 60)
        .unwrap()
        .unwrap()
        .remove(0);
    publish_public_cycle(&store, &claim);
    publish_public_cycle(&store, &other_claim);
    let identity = evidence::message_payload(&public_messages("native-session").remove(1))
        .identity()
        .unwrap();
    let other_identity = evidence::message_payload(&public_messages("healthy-session").remove(1))
        .identity()
        .unwrap();
    let healthy = evidence::current_binding_in(&store.lock(), &other.key, &other_identity)
        .unwrap()
        .unwrap();
    let mut stale = claim.clone();
    stale.claim_fence -= 1;
    assert_eq!(
        store
            .mark_opencode_v2_capture_gap(&[stale], "capture-lost", 102)
            .unwrap(),
        0
    );
    assert!(
        evidence::current_binding_in(&store.lock(), &start.key, &identity)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .mark_opencode_v2_capture_gap(&[claim], "capture-lost", 102)
            .unwrap(),
        1
    );
    assert!(
        evidence::current_binding_in(&store.lock(), &start.key, &identity)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        evidence::current_binding_in(&store.lock(), &other.key, &other_identity).unwrap(),
        Some(healthy)
    );
    assert!(
        store
            .renew_opencode_v2_capture_epoch(&[other_claim], 103, 60)
            .unwrap()
    );
}

#[test]
fn recovered_or_expired_capture_cannot_keep_old_generic_bindings() {
    for recovered in [true, false] {
        let (_directory, store, _journal, mut start) = setup();
        let claim = started(&store, &start);
        publish_public_cycle(&store, &claim);
        if recovered {
            assert_eq!(store.recover_opencode_v2_capture_claims(102).unwrap(), 1);
        } else {
            start.baseline_sequence = Some(20);
            assert!(
                store
                    .start_opencode_v2_capture_epoch(&[start.clone()], "expired", 200, 60)
                    .unwrap()
                    .is_some()
            );
        }
        assert_eq!(
            store
                .lock()
                .query_row(
                    "SELECT COUNT(*) FROM native_tool_result_current WHERE is_removed = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            4
        );
    }
}

#[test]
fn gap_publication_failure_rolls_back_generic_invalidation() {
    let (_directory, store, _journal, start) = setup();
    let claim = started(&store, &start);
    publish_public_cycle(&store, &claim);
    store
        .lock()
        .execute_batch(
            "CREATE TRIGGER reject_gap BEFORE INSERT ON native_tool_result_publication
          WHEN NEW.outcome = 'coverage_gap' AND NEW.gap_reason = 'capture-lost'
          BEGIN SELECT RAISE(ABORT, 'synthetic gap failure'); END;",
        )
        .unwrap();
    assert!(
        store
            .mark_opencode_v2_capture_gap(std::slice::from_ref(&claim), "capture-lost", 102)
            .is_err()
    );
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM native_tool_result_current WHERE is_removed = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert!(
        store
            .renew_opencode_v2_capture_epoch(&[claim], 103, 60)
            .unwrap()
    );
}

#[test]
fn a_new_public_identity_can_be_assessed_without_restoring_old_evidence() {
    let (directory, store, _journal, mut start) = setup();
    let store = Arc::new(store);
    let mut runtime = Runtime::new(store.clone(), "assistant_message", false);
    let claim = started(&store, &start);
    publish_public_cycle(&store, &claim);
    assert_eq!(runtime.sync(&store), 1);
    runtime.accept(101_000);
    store
        .mark_opencode_v2_capture_gap(&[claim], "capture-lost", 102)
        .unwrap();
    let path = directory.path().join("source.sqlite");
    let _source = source_database(&path);
    let next = reconnect(&store, &mut start, &path);
    runtime.sync(&store);
    let NativeMessageObservation::Public(mut message) =
        public_messages(&start.key.session_id).remove(1)
    else {
        unreachable!()
    };
    message.record_id = "new-public-message".into();
    message.position = position(21);
    message.revision = 21;
    message.context = Some(context(21));
    assert!(
        store
            .advance_opencode_v2_evidence(
                &next,
                21,
                &NativeEvidenceBatch {
                    source_format: SourceFormat::OpenCodeSqliteV2,
                    history_complete: false,
                    events: vec![NativeEvidenceEvent {
                        live_eligible: true,
                        fact: NativeEvidenceFact::Message(NativeMessageObservation::Public(
                            message
                        )),
                    }],
                },
                104,
                60,
            )
            .unwrap()
    );
    assert_eq!(runtime.sync(&store), 1);
    runtime.accept(104_000);
    let validity = store
        .lock()
        .prepare(
            "SELECT evidence_valid FROM burn_check_assessment_history WHERE check_id = 'capture-gap'
              ORDER BY trigger_position",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(validity, [0, 1]);
}
