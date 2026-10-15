use std::path::Path;

use antiburn_checks::actions::{ChoiceResult, plan_status_actions};
use antiburn_checks::compiler::compile;
use antiburn_checks::domain::{
    BranchId, EvidenceVersion, FrozenContext, Message, NativeOrderKey, ObservationDraft,
    ObservationKind,
};
use antiburn_checks::engine::{CheckEngine, DecisionEvaluator, DecisionFuture};
use antiburn_checks::source::SourceEvent;
use serde_json::json;

use super::*;

const CHECK: &str = "name: Retry\non:\n  tool_result:\ndecisions:\n  review:\n    model: typesafe/jev-1.13.0\n    input: {activity: \"${{ message.tool.result.status }}\"}\n    output:\n      verdict:\n        type: choice\n        question: Is this useful?\n        criteria: {yes: Yes., no: No.}\nactions:\n  - type: status\n    if: ${{ decisions.review.details.verdict.probabilities.yes > 0.9 }}\n    input: {result: failure}\n";

struct NoEvaluator;
impl DecisionEvaluator for NoEvaluator {
    fn evaluate<'a>(
        &'a self,
        _: &'a EffectiveActivation,
        _: &'a FrozenCandidate,
        _: &'a serde_json::Value,
        _: u8,
    ) -> DecisionFuture<'a> {
        panic!("store tests do not send provider requests")
    }
}

fn fixture() -> (Arc<Store>, Arc<StoreCheckStore>, EffectiveActivation) {
    let store = Arc::new(Store::open_in_memory(Path::new("/tmp/antiburn-runtime-test")).unwrap());
    fixture_with_store(store)
}

fn fixture_with_store(
    store: Arc<Store>,
) -> (Arc<Store>, Arc<StoreCheckStore>, EffectiveActivation) {
    let scope = ActivationScope {
        environment: "native".into(),
        project: "/repo".into(),
        run: RunScope::Live,
    };
    let adapter =
        Arc::new(StoreCheckStore::new(store.clone(), "owner-1".into(), scope.clone()).unwrap());
    let active = EffectiveActivation {
        id: ActivationId {
            scope,
            definition_id: "retry".into(),
            generation: Generation(1),
        },
        revision: [1; 32],
        scope: DefinitionScope::Global,
        source_paths: vec![],
        project_root: Some(PathBuf::from("/repo")),
        compiled: Arc::new(compile(CHECK).unwrap()),
        bootstrap: ObservationPosition(50),
        bootstrap_publication: PublicationCursor(0),
    };
    adapter
        .activate(
            "retry",
            &LifecycleTransition::Activated(active.clone()),
            Some(&active),
        )
        .unwrap();
    (store, adapter, active)
}

fn event(store: &Store, sequence: u64, id: &str, branch: &str, revision: u64) -> SourceBatch {
    store.lock().execute("INSERT INTO session (environment_key, agent, session_id, source_kind, source_label, first_seen_at, last_seen_at, cwd)
        VALUES ('native', 'fixture', ?1, 'file', 'fixture', 'now', 'now', '/repo') ON CONFLICT DO NOTHING", [branch]).unwrap();
    store.lock().execute("INSERT INTO native_tool_result_publication (publication_sequence, outcome, environment_key, agent, session_id, source_format, native_identity, native_position_json, native_revision, payload_json, complete, source_generation, claim_fence)
        VALUES (?1, 'update', 'native', 'fixture', ?2, 'fixture', ?3, '{}', ?4, '{}', 1, 1, 1)", params![sequence, branch, id, revision]).unwrap();
    SourceBatch {
        events: vec![SourceEvent::Observation {
            cursor: PublicationCursor(sequence),
            identity: id.into(),
            revision,
            observation: Box::new(ObservationDraft {
                branch: BranchId(branch.into()),
                native_order: NativeOrderKey(id.into()),
                version: EvidenceVersion(format!("{id}:{revision}")),
                kind: ObservationKind::ToolResult {
                    message: Message {
                        id: id.into(),
                        value: json!({"tool": {"result": {"status": "error"}}}),
                    },
                    category: "shell".into(),
                    status: "error".into(),
                },
                context: FrozenContext {
                    value: json!({"session": {"requests": []}}),
                    dependencies: vec![],
                },
                complete: true,
            }),
        }],
        next: PublicationCursor(sequence),
        has_more: false,
    }
}

fn apply(
    adapter: &Arc<StoreCheckStore>,
    active: &EffectiveActivation,
    page: &SourceBatch,
) -> FrozenCandidate {
    let engine = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator));
    engine.apply_source_page(active, page).unwrap();
    let position = adapter
        .load_source_progress(active)
        .unwrap()
        .observation_position;
    adapter
        .load_recovery(None, 10)
        .unwrap()
        .into_iter()
        .find_map(|record| match record.work {
            RecoveryWork::Pending(candidate) if candidate.trigger_position == position => {
                Some(candidate)
            }
            _ => None,
        })
        .unwrap()
}

fn claim(adapter: &StoreCheckStore, candidate: &FrozenCandidate) -> ClaimedCandidate {
    let CandidateClaim::Claimed(claimed) = adapter.claim_due(candidate, 100_000, true).unwrap()
    else {
        panic!("candidate must be claimable");
    };
    *claimed
}

fn answer(
    adapter: &StoreCheckStore,
    active: &EffectiveActivation,
    claimed: ClaimedCandidate,
    yes: f64,
) -> AcceptedAssessment {
    adapter
        .record_dispatch_intent(
            &claimed,
            100_000,
            Duration::from_secs(60),
            claimed.next_attempt,
        )
        .unwrap();
    let evaluated = EvaluatedCandidate {
        choices: vec![
            ChoiceResult::validate(
                "review",
                "verdict",
                &["yes".into(), "no".into()],
                if yes > 0.5 { "yes" } else { "no" },
                BTreeMap::from([("yes".into(), yes), ("no".into(), 1.0 - yes)]),
            )
            .unwrap(),
        ],
        attempts: claimed.next_attempt,
    };
    adapter.record_answer(&claimed, &evaluated).unwrap();
    let actions = plan_status_actions(
        &active.compiled.actions,
        &claimed.candidate.context.value,
        &evaluated.choices,
        claimed.prior_status,
    )
    .unwrap();
    AcceptedAssessment {
        claimed,
        evaluated,
        actions,
    }
}

#[test]
fn page_checkpoint_coalescing_and_generation_are_durable_and_atomic() {
    let (store, adapter, active) = fixture();
    let page = event(&store, 11, "one", "branch", 1);
    store.lock().execute_batch("CREATE TRIGGER reject_candidate BEFORE INSERT ON burn_check_candidate BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    let engine = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator));
    assert!(engine.apply_source_page(&active, &page).is_err());
    assert_eq!(
        adapter
            .load_source_progress(&active)
            .unwrap()
            .observation_position,
        ObservationPosition(50)
    );
    assert_eq!(
        store
            .lock()
            .query_row("SELECT COUNT(*) FROM burn_check_observation", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    store
        .lock()
        .execute_batch("DROP TRIGGER reject_candidate")
        .unwrap();
    let first = apply(&adapter, &active, &page);
    let second = apply(&adapter, &active, &event(&store, 29, "two", "branch", 1));
    assert_eq!(second.trigger_position, ObservationPosition(52));
    assert_eq!(
        adapter.claim_due(&first, 100_000, false).unwrap(),
        CandidateClaim::Stale
    );
    assert_eq!(adapter.load_recovery(None, 10).unwrap().len(), 1);
    let restarted =
        StoreCheckStore::new(store.clone(), "owner-2".into(), active.id.scope.clone()).unwrap();
    assert_eq!(
        restarted
            .load_lifecycle("retry", active.compiled.clone())
            .unwrap(),
        LifecycleState::Active(active.clone())
    );
    restarted.register_activation(&active).unwrap();
    assert_eq!(
        restarted.load_source_progress(&active).unwrap(),
        adapter.load_source_progress(&active).unwrap()
    );
    let mut wrong = active.clone();
    wrong.id.generation = Generation(7);
    assert!(
        restarted
            .activate(
                "retry",
                &LifecycleTransition::Activated(wrong.clone()),
                Some(&wrong)
            )
            .is_err()
    );
    assert_eq!(claim(&restarted, &second).candidate, second);
}

#[test]
fn answered_recovery_defers_without_resend_and_preserves_last_status_write() {
    let (store, adapter, active) = fixture();
    let first = apply(&adapter, &active, &event(&store, 1, "one", "branch", 1));
    let assessment = answer(&adapter, &active, claim(&adapter, &first), 0.99);
    let next_page = event(&store, 2, "two", "branch", 1);
    assert_eq!(
        adapter.accept(&assessment).unwrap(),
        AcceptanceOutcome::Deferred
    );
    let restarted = Arc::new(
        StoreCheckStore::new(store.clone(), "owner-2".into(), active.id.scope.clone()).unwrap(),
    );
    restarted.register_activation(&active).unwrap();
    let RecoveryWork::Answered { claimed, evaluated } =
        restarted.load_recovery(None, 10).unwrap().remove(0).work
    else {
        panic!("saved answer must recover");
    };
    assert_eq!(evaluated, assessment.evaluated);
    let second = apply(&restarted, &active, &next_page);
    let recovered = AcceptedAssessment {
        claimed,
        evaluated,
        actions: assessment.actions,
    };
    store.lock().execute_batch("CREATE TRIGGER reject_status BEFORE INSERT ON burn_check_current_status BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    assert!(restarted.accept(&recovered).is_err());
    for table in [
        "burn_check_assessment_history",
        "burn_check_acceptance_watermark",
    ] {
        let count: i64 = store
            .lock()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "failed acceptance must roll back {table}");
    }
    store
        .lock()
        .execute_batch("DROP TRIGGER reject_status")
        .unwrap();
    assert_eq!(
        restarted.accept(&recovered).unwrap(),
        AcceptanceOutcome::Accepted
    );
    assert_eq!(
        restarted.accept(&recovered).unwrap(),
        AcceptanceOutcome::AlreadyAccepted
    );
    let original: (String, i64) = store
        .lock()
        .query_row(
            "SELECT assessment_id, updated_at_epoch FROM burn_check_current_status",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let claimed = claim(&restarted, &second);
    assert_eq!(claimed.prior_status, Some(StatusResult::Failure));
    let no_write = answer(&restarted, &active, claimed, 0.4);
    assert_eq!(no_write.status_write(), None);
    assert_eq!(
        restarted.accept(&no_write).unwrap(),
        AcceptanceOutcome::Accepted
    );
    let retained: (String, i64) = store
        .lock()
        .query_row(
            "SELECT assessment_id, updated_at_epoch FROM burn_check_current_status",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(original, retained);
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_evaluation_attempt",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn safe_retry_number_survives_restart_and_unknown_dispatch_never_resends() {
    let (store, adapter, active) = fixture();
    let candidate = apply(&adapter, &active, &event(&store, 1, "one", "branch", 1));
    let first = claim(&adapter, &candidate);
    adapter
        .record_dispatch_intent(&first, 100_000, Duration::from_secs(60), 1)
        .unwrap();
    adapter
        .record_attempt_failure(&first, 1, AttemptFailure::SafeTransient)
        .unwrap();
    let restarted =
        StoreCheckStore::new(store.clone(), "owner-2".into(), active.id.scope.clone()).unwrap();
    restarted.register_activation(&active).unwrap();
    assert!(matches!(
        restarted.claim_due(&candidate, 120_000, false).unwrap(),
        CandidateClaim::NotDue {
            retry_at_unix_ms: 160_000
        }
    ));
    let retry = claim(&restarted, &candidate);
    assert_eq!(retry.next_attempt, 2);
    assert!(
        restarted
            .record_dispatch_intent(&first, 100_000, Duration::ZERO, 2)
            .is_err()
    );
    restarted
        .record_dispatch_intent(&retry, 100_000, Duration::ZERO, 2)
        .unwrap();
    let recovered =
        StoreCheckStore::new(store.clone(), "owner-3".into(), active.id.scope.clone()).unwrap();
    recovered.register_activation(&active).unwrap();
    assert!(recovered.load_recovery(None, 10).unwrap().is_empty());
    assert_eq!(
        recovered.claim_due(&candidate, 200_000, true).unwrap(),
        CandidateClaim::Stale
    );
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT outcome FROM burn_check_evaluation_attempt WHERE attempt_number = 2",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "unknown"
    );
}

#[test]
fn correction_revokes_answer_and_exact_claim_does_not_substitute_branches() {
    let (store, adapter, active) = fixture();
    let first = apply(&adapter, &active, &event(&store, 1, "one", "branch-a", 1));
    let second = apply(&adapter, &active, &event(&store, 2, "two", "branch-b", 1));
    let second_claim = claim(&adapter, &second);
    assert_eq!(second_claim.id, CandidateId::from(&second));
    let assessment = answer(&adapter, &active, claim(&adapter, &first), 0.99);
    let mut correction = event(&store, 3, "one", "branch-a", 1);
    let SourceEvent::Observation { observation, .. } = &mut correction.events[0] else {
        unreachable!()
    };
    observation.version = EvidenceVersion("same-revision-correction".into());
    let engine = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator));
    assert_eq!(
        engine
            .apply_source_page(&active, &correction)
            .unwrap()
            .candidates,
        0
    );
    assert_eq!(
        adapter.accept(&assessment).unwrap(),
        AcceptanceOutcome::Stale
    );
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_current_status",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn same_revision_correction_or_removal_invalidates_accepted_evidence() {
    for removed in [false, true] {
        let (store, adapter, active) = fixture();
        let candidate = apply(&adapter, &active, &event(&store, 1, "one", "branch", 1));
        let assessment = answer(&adapter, &active, claim(&adapter, &candidate), 0.99);
        assert_eq!(
            adapter.accept(&assessment).unwrap(),
            AcceptanceOutcome::Accepted
        );
        let page = if removed {
            store.lock().execute("INSERT INTO native_tool_result_publication (publication_sequence, outcome, environment_key, agent, session_id, source_format, native_identity, native_position_json, native_revision, payload_json, complete, source_generation, claim_fence)
                SELECT 2, 'removal', environment_key, agent, session_id, source_format, native_identity, native_position_json, native_revision, '{}', 1, source_generation, claim_fence
                FROM native_tool_result_publication WHERE publication_sequence = 1", []).unwrap();
            SourceBatch {
                events: vec![SourceEvent::Removal {
                    cursor: PublicationCursor(2),
                    identity: "one".into(),
                    revision: 1,
                    branch: BranchId("branch".into()),
                }],
                next: PublicationCursor(2),
                has_more: false,
            }
        } else {
            let mut page = event(&store, 2, "one", "branch", 1);
            let SourceEvent::Observation { observation, .. } = &mut page.events[0] else {
                unreachable!()
            };
            let ObservationKind::ToolResult { message, .. } = &mut observation.kind else {
                unreachable!()
            };
            message.value["tool"]["result"]["status"] = json!("corrected");
            page
        };
        CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator))
            .apply_source_page(&active, &page)
            .unwrap();
        let valid: (bool, bool) = store.lock().query_row(
            "SELECT s.evidence_valid, h.evidence_valid FROM burn_check_current_status s JOIN burn_check_assessment_history h USING(assessment_id)",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(valid, (false, false));
        let mut next = active.clone();
        next.id.generation = Generation(2);
        next.revision = [2; 32];
        adapter
            .activate(
                "retry",
                &LifecycleTransition::Activated(next.clone()),
                Some(&next),
            )
            .unwrap();
        assert_eq!(
            adapter.claim_due(&candidate, 200_000, true).unwrap(),
            CandidateClaim::Stale
        );
        assert!(adapter.load_recovery(None, 10).unwrap().is_empty());
        assert_eq!(
            store
                .lock()
                .query_row(
                    "SELECT COUNT(*) FROM burn_check_current_status",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[test]
fn later_correction_in_one_page_prevents_storing_the_earlier_candidate() {
    let (store, adapter, active) = fixture();
    let mut page = event(&store, 1, "one", "branch", 1);
    let mut correction = event(&store, 2, "one", "branch", 1);
    let SourceEvent::Observation { observation, .. } = &mut correction.events[0] else {
        unreachable!()
    };
    observation.version = EvidenceVersion("corrected".into());
    page.events.extend(correction.events);
    page.next = PublicationCursor(2);
    let outcome = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator))
        .apply_source_page(&active, &page)
        .unwrap();
    assert_eq!(outcome.candidates, 0);
    assert!(adapter.load_recovery(None, 10).unwrap().is_empty());
    assert_eq!(
        store
            .lock()
            .query_row("SELECT COUNT(*) FROM burn_check_candidate", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}

#[test]
fn unapplied_publication_for_another_session_does_not_block_acceptance() {
    let (store, adapter, active) = fixture();
    let candidate = apply(&adapter, &active, &event(&store, 1, "one", "branch", 1));
    let assessment = answer(&adapter, &active, claim(&adapter, &candidate), 0.99);
    event(&store, 2, "two", "unrelated-session", 1);
    assert_eq!(
        adapter.accept(&assessment).unwrap(),
        AcceptanceOutcome::Accepted
    );
}

#[test]
fn overflow_and_failure_keep_history_without_status_or_watermark() {
    let (store, adapter, active) = fixture();
    let overflow = apply(&adapter, &active, &event(&store, 1, "one", "a", 1));
    adapter.record_overflow(&overflow).unwrap();
    let failed = apply(&adapter, &active, &event(&store, 2, "two", "b", 1));
    adapter
        .record_failure(&claim(&adapter, &failed), "provider failed")
        .unwrap();
    let outcomes = store
        .lock()
        .prepare("SELECT outcome FROM burn_check_assessment_history ORDER BY trigger_position")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(outcomes, ["not_assessed", "execution_failed"]);
    for table in [
        "burn_check_current_status",
        "burn_check_acceptance_watermark",
        "burn_check_evaluation_attempt",
    ] {
        assert_eq!(
            store
                .lock()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn retention_prunes_unused_frozen_inputs_and_preserves_current_status_evidence() {
    let (store, adapter, active) = fixture();
    for (sequence, branch, yes) in [(1, "no-status", 0.4), (2, "failure", 0.99)] {
        let candidate = apply(
            &adapter,
            &active,
            &event(&store, sequence, "one", branch, 1),
        );
        let assessment = answer(&adapter, &active, claim(&adapter, &candidate), yes);
        assert_eq!(
            adapter.accept(&assessment).unwrap(),
            AcceptanceOutcome::Accepted
        );
    }
    Store::prune_burn_check_candidate_payloads_in(&store.lock(), now_epoch() + 1).unwrap();
    let payloads = store.lock().prepare("SELECT branch_key, frozen_input_json <> '{}' FROM burn_check_candidate ORDER BY branch_key").unwrap()
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(
        payloads,
        [("failure".into(), true), ("no-status".into(), false)]
    );
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_assessment_history",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn source_page_read_before_deletion_cannot_restore_deleted_check_data() {
    let (store, adapter, active) = fixture();
    let page = event(&store, 1, "one", "branch", 1);
    store
        .delete_session(&crate::store::SessionKey::new(
            "native", "fixture", "branch",
        ))
        .unwrap()
        .unwrap();
    assert!(
        CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator))
            .apply_source_page(&active, &page)
            .is_err()
    );
    assert_eq!(
        adapter
            .load_source_progress(&active)
            .unwrap()
            .publication_cursor,
        PublicationCursor(0)
    );
    for table in ["burn_check_observation", "burn_check_candidate"] {
        assert_eq!(
            store
                .lock()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn v75_migration_preserves_existing_data_and_supports_runtime_acceptance() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for &migration in &crate::store::schema::MIGRATIONS[..75] {
        connection.execute_batch(migration).unwrap();
    }
    connection.pragma_update(None, "user_version", 75).unwrap();
    connection.execute_batch(
        "INSERT INTO setting (key, value) VALUES ('legacy-setting', 'preserved');
         INSERT INTO session (environment_key, agent, session_id, source_kind, source_label, first_seen_at, last_seen_at)
           VALUES ('native', 'claude-code', 'migration-session', 'file', 'fixture', 'now', 'now');
         INSERT INTO burn_check_assessment (environment_key, agent, session_id, check_id, incarnation, created_at_epoch, updated_at_epoch)
           VALUES ('native', 'claude-code', 'migration-session', 'existing-detector', 1, 1, 1);",
    ).unwrap();
    let store =
        Store::from_connection(connection, PathBuf::from("/tmp/antiburn-runtime-migration"))
            .unwrap();
    assert_eq!(
        store.schema_version().unwrap(),
        i64::try_from(crate::store::schema::MIGRATIONS.len()).unwrap()
    );
    let (store, adapter, active) = fixture_with_store(Arc::new(store));
    let candidate = apply(&adapter, &active, &event(&store, 1, "one", "branch", 1));
    let assessment = answer(&adapter, &active, claim(&adapter, &candidate), 0.99);
    assert_eq!(
        adapter.accept(&assessment).unwrap(),
        AcceptanceOutcome::Accepted
    );
    let connection = store.lock();
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM setting WHERE key = 'legacy-setting'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "preserved"
    );
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM burn_check_assessment WHERE check_id = 'existing-detector' AND session_id = 'migration-session'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert!(
        connection
            .execute(
                "UPDATE native_tool_result_publication SET source_generation = 2",
                []
            )
            .is_err()
    );
    assert!(
        connection
            .execute("DELETE FROM native_tool_result_publication", [])
            .is_err()
    );
    drop(connection);
    store
        .delete_session(&crate::store::SessionKey::new(
            "native", "fixture", "branch",
        ))
        .unwrap()
        .unwrap();
    assert!(
        store
            .native_tool_result_publications_after(0, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn recovery_uses_keyset_pages_and_replay_has_an_independent_scope() {
    let (store, adapter, active) = fixture();
    for (sequence, branch) in [(1, "a"), (2, "b"), (3, "c")] {
        apply(
            &adapter,
            &active,
            &event(&store, sequence, "same-native-id", branch, 1),
        );
    }
    let mut after = None;
    let mut ids = Vec::new();
    loop {
        let records = adapter.load_recovery(after.as_ref(), 1).unwrap();
        if records.is_empty() {
            break;
        }
        let RecoveryWork::Pending(candidate) = &records[0].work else {
            panic!("pending candidate expected");
        };
        let id = CandidateId::from(candidate);
        assert!(after.as_ref().is_none_or(|after| after < &id));
        after = Some(id.clone());
        ids.push(id);
    }
    assert_eq!(ids.len(), 3);
    let mut replay = active.clone();
    replay.id.scope.run = RunScope::Replay("isolated-run".into());
    replay.bootstrap = ObservationPosition(0);
    let replay_store = Arc::new(
        StoreCheckStore::new(
            store.clone(),
            "replay-owner".into(),
            replay.id.scope.clone(),
        )
        .unwrap(),
    );
    replay_store
        .activate(
            "retry",
            &LifecycleTransition::Activated(replay.clone()),
            Some(&replay),
        )
        .unwrap();
    let page = SourceBatch {
        events: vec![SourceEvent::Observation {
            cursor: PublicationCursor(1),
            identity: "same-native-id".into(),
            revision: 1,
            observation: Box::new(ObservationDraft {
                branch: BranchId("a".into()),
                native_order: NativeOrderKey("one".into()),
                version: EvidenceVersion("one".into()),
                kind: ObservationKind::ToolResult {
                    message: Message {
                        id: "same-native-id".into(),
                        value: json!({"tool": {"result": {"status": "error"}}}),
                    },
                    category: "shell".into(),
                    status: "error".into(),
                },
                context: FrozenContext {
                    value: json!({"session": {"requests": []}}),
                    dependencies: vec![],
                },
                complete: true,
            }),
        }],
        next: PublicationCursor(1),
        has_more: false,
    };
    let candidate = apply(&replay_store, &replay, &page);
    let assessment = answer(
        &replay_store,
        &replay,
        claim(&replay_store, &candidate),
        0.99,
    );
    assert_eq!(
        replay_store.accept(&assessment).unwrap(),
        AcceptanceOutcome::Accepted
    );
    assert_ne!(
        StoreCheckStore::project_key(&replay.id.scope),
        StoreCheckStore::project_key(&active.id.scope)
    );
    assert_eq!(adapter.load_recovery(None, 10).unwrap().len(), 3);
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_current_status WHERE project_key = ?1",
                [&adapter.project_key],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn clear_and_session_deletion_remove_frozen_selector_payloads() {
    let (store, adapter, active) = fixture();
    let engine = CheckEngine::new(adapter.clone(), Arc::new(NoEvaluator));
    engine
        .apply_source_page(&active, &event(&store, 1, "one", "branch", 1))
        .unwrap();
    let RecoveryWork::Pending(candidate) = adapter.load_recovery(None, 10).unwrap().remove(0).work
    else {
        panic!("pending candidate expected");
    };
    let assessment = answer(&adapter, &active, claim(&adapter, &candidate), 0.99);
    assert_eq!(
        adapter.accept(&assessment).unwrap(),
        AcceptanceOutcome::Accepted
    );
    store
        .delete_session(&crate::store::SessionKey::new(
            "native", "fixture", "branch",
        ))
        .unwrap();
    let progress = adapter.load_source_progress(&active).unwrap();
    assert!(progress.selection.branches.is_empty());
    assert!(progress.selection.seen_results.is_empty());
    for table in [
        "burn_check_observation",
        "burn_check_candidate",
        "burn_check_candidate_dependency",
        "burn_check_evaluation_attempt",
        "burn_check_assessment_history",
        "burn_check_current_status",
        "burn_check_acceptance_watermark",
        "burn_check_cooldown",
    ] {
        let count: i64 = store
            .lock()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "session deletion must clear {table}");
    }
    let next_page = event(&store, 2, "two", "new-branch", 1);
    assert!(engine.apply_source_page(&active, &next_page).is_err());
    engine.apply_source_page(&active, &next_page).unwrap();
    assert_eq!(
        adapter
            .load_source_progress(&active)
            .unwrap()
            .selection
            .branches
            .len(),
        1
    );
    store.clear_local_session_data().unwrap();
    assert!(adapter.register_activation(&active).is_err());
    assert_eq!(
        adapter.load_lifecycle("retry", None).unwrap(),
        LifecycleState::Removed {
            generation: Generation(1)
        }
    );
    let mut next = active.clone();
    next.id.generation = Generation(2);
    adapter
        .activate(
            "retry",
            &LifecycleTransition::Activated(next.clone()),
            Some(&next),
        )
        .unwrap();
    assert_eq!(
        adapter.load_lifecycle("retry", None).unwrap(),
        LifecycleState::Uncompiled {
            generation: Generation(2)
        }
    );
}

struct RecordingEvaluator(Mutex<Vec<u8>>);

impl DecisionEvaluator for RecordingEvaluator {
    fn evaluate<'a>(
        &'a self,
        _: &'a EffectiveActivation,
        _: &'a FrozenCandidate,
        _: &'a serde_json::Value,
        number: u8,
    ) -> DecisionFuture<'a> {
        self.0.lock().unwrap().push(number);
        Box::pin(async move {
            Ok(EvaluatedCandidate {
                attempts: 1,
                choices: vec![
                    ChoiceResult::validate(
                        "review",
                        "verdict",
                        &["yes".into(), "no".into()],
                        "yes",
                        BTreeMap::from([("yes".into(), 0.99), ("no".into(), 0.01)]),
                    )
                    .unwrap(),
                ],
            })
        })
    }
}

#[test]
fn recovered_safe_retry_passes_the_durable_number_to_the_evaluator() {
    use antiburn_checks::engine::{ExecutionPolicy, RunOutcome, StartOutcome};
    use std::task::{Context, Poll, Waker};

    let (store, adapter, active) = fixture();
    let candidate = apply(&adapter, &active, &event(&store, 1, "one", "branch", 1));
    let claimed = claim(&adapter, &candidate);
    adapter
        .record_dispatch_intent(&claimed, 0, Duration::ZERO, 1)
        .unwrap();
    adapter
        .record_attempt_failure(&claimed, 1, AttemptFailure::SafeTransient)
        .unwrap();
    let restarted =
        Arc::new(StoreCheckStore::new(store, "owner-2".into(), active.id.scope.clone()).unwrap());
    restarted.register_activation(&active).unwrap();
    let evaluator = Arc::new(RecordingEvaluator(Mutex::new(vec![])));
    let engine = CheckEngine::with_policy(
        restarted,
        evaluator.clone(),
        ExecutionPolicy::SequentialReplay,
    );
    assert_eq!(engine.recover_pending().unwrap().candidates, 1);
    let StartOutcome::Started(mut task) = engine.start_one_due(&active, 1).unwrap() else {
        panic!("recovered retry must start");
    };
    let Poll::Ready(settlement) = task.as_mut().poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("scripted evaluator settles immediately");
    };
    assert_eq!(settlement.unwrap().outcome, RunOutcome::Accepted);
    assert_eq!(*evaluator.0.lock().unwrap(), [2]);
}
