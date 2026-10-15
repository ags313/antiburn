use std::path::Path;
use std::sync::Arc;

use antiburn_checks::actions::{ActionOutcome, ActionOutcomeKind, ActionPlan, ChoiceResult};
use antiburn_checks::check_store::{AcceptanceOutcome, CheckStore, EvaluatedCandidate};
use antiburn_checks::compiler::{StatusResult, compile};
use antiburn_checks::definition_lifecycle::{EffectiveActivation, LifecycleTransition};
use antiburn_checks::discovery::DefinitionScope;
use antiburn_checks::domain::{
    ActivationId, BranchId, FrozenContext, Generation, Message, ObservationPosition,
};
use serde_json::json;

use super::*;
use crate::store::burn_checks::{BurnCheckCandidateClaim, StoreCheckStore};

fn definition(project: &str) -> InstalledCheckDefinition {
    InstalledCheckDefinition {
        effective_id: format!("private:{project}:retry"),
        check_id: "failed-retry-loop".into(),
        activation_scope_key: serde_json::to_string(&scope(project)).unwrap(),
        generation: Some(1),
        definition_revision: "01".repeat(32),
        name: "Failed Retry Loop".into(),
        aliases: vec!["repeated failures".into()],
        description: Some("Detects repeated failures without recovery.".into()),
        source: InstalledCheckSource {
            scope: InstalledCheckScope::Project,
            path: Some(".antiburn/checks/retries.yaml".into()),
        },
        availability: InstalledCheckAvailability::Available,
        diagnostic: None,
    }
}

fn scope(project: &str) -> ActivationScope {
    ActivationScope {
        environment: "native".into(),
        session_cwd: project.into(),
        run: RunScope::Live,
    }
}

fn request() -> ReportRequest {
    ReportRequest {
        environment_key: "native".into(),
        window: antiburn_local::insights::ReportWindow {
            start_epoch: 90,
            end_epoch: 200,
        },
        computed_at_epoch: 300,
    }
}

fn store() -> Arc<Store> {
    Arc::new(Store::open_in_memory(Path::new("/tmp/check-facade-test")).unwrap())
}

fn activate(store: &Arc<Store>, definition: &InstalledCheckDefinition) {
    let scope: ActivationScope = serde_json::from_str(&definition.activation_scope_key).unwrap();
    let adapter = StoreCheckStore::new(store.clone(), "test".into(), scope.clone()).unwrap();
    let activation = EffectiveActivation {
        id: ActivationId {
            scope: scope.clone(),
            definition_id: definition.check_id.clone(),
            generation: Generation(1),
        },
        revision: [1; 32],
        definition_scope: DefinitionScope::Project,
        source_paths: vec![],
        project_root: Some(scope.session_cwd.into()),
        compiled: Arc::new(compile("name: Report fixture\non:\n  tool_result:\ndecisions:\n  review:\n    model: typesafe/jev-1.13.0\n    input: {activity: \"${{ message.tool.result.status }}\"}\n    output:\n      verdict:\n        type: choice\n        question: Is this useful?\n        criteria: {clear: Clear., concern: Concern.}\nactions:\n  - type: status\n    input: {result: failure}\n").unwrap()),
        bootstrap_observation_position: ObservationPosition(0),
        bootstrap_publication: Default::default(),
    };
    adapter
        .activate(
            &definition.check_id,
            &LifecycleTransition::Activated(activation.clone()),
            Some(&activation),
        )
        .unwrap();
}

fn session(store: &Store, id: &str, epoch: i64) {
    store.lock().execute(
        "INSERT OR IGNORE INTO session (environment_key, agent, session_id, source_kind, source_label,
          cwd, updated_at_epoch, started_at_epoch, first_seen_at, last_seen_at, incarnation)
          VALUES ('native', 'claude-code', ?1, 'local', 'test', '/workspace/demo', ?2, ?2, '', '', 7)",
        params![id, epoch],
    ).unwrap();
    store
        .lock()
        .execute(
            "INSERT OR IGNORE INTO session_evidence (environment_key, agent, session_id, status,
          analyzed_generation, parser_revision, analyzer_revision, evidence_schema_revision)
          VALUES ('native', 'claude-code', ?1, 'ready', 0, ?2, ?3, ?4)",
            params![
                id,
                PARSER_REVISION,
                ANALYZER_REVISION,
                EVIDENCE_SCHEMA_REVISION
            ],
        )
        .unwrap();
}

fn accept(
    store: &Store,
    definition: &InstalledCheckDefinition,
    branch: &str,
    position: i64,
    status: Option<StatusResult>,
) {
    session(store, branch, 100);
    let id = format!("{}:{branch}:{position}", definition.activation_scope_key);
    let label = if status == Some(StatusResult::Success) {
        "clear"
    } else {
        "concern"
    };
    let evaluated = EvaluatedCandidate {
        choices: vec![
            ChoiceResult::validate(
                "review",
                "verdict",
                &["clear".into(), "concern".into()],
                label,
                BTreeMap::from([
                    ("clear".into(), if label == "clear" { 0.99 } else { 0.01 }),
                    (
                        "concern".into(),
                        if label == "concern" { 0.99 } else { 0.01 },
                    ),
                ]),
            )
            .unwrap(),
        ],
        attempts: 1,
    };
    let plan = ActionPlan {
        current_status: status,
        outcomes: vec![ActionOutcome {
            index: 0,
            outcome: match status {
                Some(result) => ActionOutcomeKind::Applied {
                    result,
                    message: Some("Review the recorded work.".into()),
                },
                None => ActionOutcomeKind::SkippedConditionFalse,
            },
        }],
    };
    let frozen = FrozenCandidate {
        activation: ActivationId {
            scope: serde_json::from_str(&definition.activation_scope_key).unwrap(),
            definition_id: definition.check_id.clone(),
            generation: Generation(1),
        },
        branch: BranchId(branch.into()),
        trigger_position: ObservationPosition(position as u64),
        messages: vec![Message {
            id: "tool-1".into(),
            value: json!({"tool":{"result":{"status":"error","content":"test failed"}}}),
        }],
        context: FrozenContext {
            value: json!({"session":{"requests":[{"content":"Run the tests"}]}}),
            dependencies: vec![],
        },
        dependencies: vec![],
        fingerprint: [0; 32],
    };
    store.lock().execute(
        "INSERT INTO burn_check_candidate (candidate_id, check_id, activation_scope_key, generation, branch_key,
           session_key_json, trigger_position, selected_publication_head, dependencies_json, frozen_input_json,
           answer_json, input_fingerprint, state, claim_owner, claim_fence, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, 0, '{}', ?7, ?8, ?1, 'answered', 'test', 1, 100, 100)",
        params![id, definition.check_id, definition.activation_scope_key, branch, serde_json::to_string(&("native", "claude-code", branch)).unwrap(), position, serde_json::to_string(&frozen).unwrap(), serde_json::to_string(&evaluated).unwrap()],
    ).unwrap();
    assert_eq!(
        store
            .try_accept_burn_check_answer(
                &BurnCheckCandidateClaim {
                    candidate_id: id,
                    owner: "test".into(),
                    fence: 1
                },
                &format!("assessment-{position}"),
                &serde_json::to_value(&evaluated).unwrap(),
                &serde_json::to_value(&plan).unwrap(),
                status.map(|value| match value {
                    StatusResult::Failure => "failure",
                    StatusResult::Success => "success",
                }),
                101 + position
            )
            .unwrap(),
        AcceptanceOutcome::Accepted
    );
}

fn populated() -> (Arc<Store>, InstalledCheckDefinition) {
    let store = store();
    let definition = definition("/workspace/demo");
    activate(&store, &definition);
    accept(
        &store,
        &definition,
        "branch-a",
        1,
        Some(StatusResult::Failure),
    );
    accept(&store, &definition, "branch-a", 2, None);
    accept(
        &store,
        &definition,
        "branch-b",
        3,
        Some(StatusResult::Success),
    );
    (store, definition)
}

fn rows(
    store: &Store,
    definitions: &[InstalledCheckDefinition],
) -> Vec<InstalledCheckReportPayload> {
    read_installed_checks_in_window(store, definitions, &request()).unwrap()
}

#[test]
fn rust_facade_matches_frontend_fixture_and_retains_the_actual_failure_write() {
    let (store, definition) = populated();
    let before = store.lock().total_changes();
    let rows = rows(&store, &[definition]);
    assert_eq!(store.lock().total_changes(), before);
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../src/views/main-window/burn-checks/tests/fixtures/installed-check-report.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(&rows[0]).unwrap(), expected);
    let evidence = read_evidence(
        &store,
        &[self::definition("/workspace/demo")],
        &request(),
        &rows[0].targets[0].id,
    )
    .unwrap();
    assert_eq!(evidence.status, BurnCheckEvidenceStatus::Available);
    assert_eq!(evidence.items.len(), 2);
    assert!(evidence.items[0].excerpt.contains("Run the tests"));
}

#[test]
fn ready_session_without_a_candidate_waits_for_results_without_checking() {
    let store = store();
    let definition = definition("/workspace/demo");
    activate(&store, &definition);
    session(&store, "active-session", 190);
    let report = rows(&store, std::slice::from_ref(&definition));
    assert_eq!(report[0].checking_count, 0);
    assert_eq!(report[0].waiting_for_results_count, 1);
    assert_eq!(report[0].waiting_for_data_count, 0);
    assert_eq!(
        (report[0].finding, report[0].clean, report[0].review_failed),
        (0, 0, 0)
    );
}

#[test]
fn exposes_a_candidate_that_failed_without_a_status_write() {
    let store = store();
    let definition = definition("/workspace/demo");
    activate(&store, &definition);
    accept(&store, &definition, "session-failed", 1, None);
    let reviewed = rows(&store, std::slice::from_ref(&definition));
    assert_eq!(reviewed[0].reviewed_without_status, 1);
    store
        .lock()
        .execute(
            "UPDATE burn_check_candidate SET state = 'execution_failed' WHERE branch_key = 'session-failed'",
            [],
        )
        .unwrap();
    store.lock().execute("UPDATE burn_check_assessment_history SET outcome = 'execution_failed', evidence_valid = 0 WHERE branch_key = 'session-failed'", []).unwrap();
    let report = rows(&store, std::slice::from_ref(&definition));
    assert_eq!(report[0].review_failed, 1);
    assert_eq!(report[0].reviewed_without_status, 0);
    assert_eq!(report[0].checking_count, 0);
    assert_eq!(report[0].waiting_for_results_count, 1);
}

#[test]
fn retained_status_and_latest_assessment_are_independent() {
    for status in [StatusResult::Failure, StatusResult::Success] {
        let store = store();
        let definition = definition("/workspace/demo");
        activate(&store, &definition);
        accept(&store, &definition, "retained", 1, Some(status));
        accept(&store, &definition, "retained", 2, None);
        let no_write = rows(&store, std::slice::from_ref(&definition));
        assert_eq!(no_write[0].reviewed_without_status, 1);
        assert_eq!(
            no_write[0].finding,
            u64::from(status == StatusResult::Failure)
        );
        assert_eq!(
            no_write[0].clean,
            u64::from(status == StatusResult::Success)
        );
        assert_eq!(
            no_write[0].explanation.as_deref(),
            Some("Review the recorded work.")
        );
        store.lock().execute("UPDATE burn_check_candidate SET state = 'execution_failed' WHERE trigger_position = 2", []).unwrap();
        store.lock().execute("UPDATE burn_check_assessment_history SET outcome = 'execution_failed', evidence_valid = 0, actions_json = '{\"detail\":\"failed\"}' WHERE trigger_position = 2", []).unwrap();
        let failed = rows(&store, std::slice::from_ref(&definition));
        assert_eq!(failed[0].review_failed, 1);
        assert_eq!(failed[0].reviewed_without_status, 0);
        assert_eq!(failed[0].finding, no_write[0].finding);
        assert_eq!(failed[0].clean, no_write[0].clean);
        assert_eq!(failed[0].checking_count, 0);
    }
}

#[test]
fn checking_uses_any_queued_or_in_flight_candidate_and_stops_at_terminal_states() {
    let store = store();
    let definition = definition("/workspace/demo");
    activate(&store, &definition);
    accept(&store, &definition, "active", 1, None);
    accept(&store, &definition, "active", 2, None);
    for state in ["pending", "claimed", "dispatching", "answered"] {
        store
            .lock()
            .execute(
                "UPDATE burn_check_candidate SET state = ?1 WHERE trigger_position = 1",
                [state],
            )
            .unwrap();
        let report = rows(&store, std::slice::from_ref(&definition));
        assert_eq!(report[0].checking_count, 1, "{state}");
        assert_eq!(report[0].waiting_for_results_count, 0);
    }
    store
        .lock()
        .execute(
            "UPDATE session SET source_generation = source_generation + 1",
            [],
        )
        .unwrap();
    let stale_but_running = rows(&store, std::slice::from_ref(&definition));
    assert_eq!(stale_but_running[0].checking_count, 1);
    assert_eq!(stale_but_running[0].waiting_for_data_count, 0);
    store
        .lock()
        .execute(
            "UPDATE session SET source_generation = source_generation - 1",
            [],
        )
        .unwrap();
    for state in [
        "accepted",
        "execution_failed",
        "not_assessed",
        "cancelled",
        "stale",
        "coalesced",
    ] {
        store
            .lock()
            .execute(
                "UPDATE burn_check_candidate SET state = ?1 WHERE trigger_position = 1",
                [state],
            )
            .unwrap();
        assert_eq!(
            rows(&store, std::slice::from_ref(&definition))[0].checking_count,
            0,
            "{state}"
        );
    }
}

#[test]
fn stale_session_evidence_waits_for_data_and_cannot_expose_retained_status_or_evidence() {
    let (store, definition) = populated();
    let target = rows(&store, std::slice::from_ref(&definition))[0].targets[0]
        .id
        .clone();
    store.lock().execute("UPDATE session SET source_generation = source_generation + 1 WHERE session_id = 'branch-a'", []).unwrap();
    let report = rows(&store, std::slice::from_ref(&definition));
    assert_eq!((report[0].finding, report[0].clean), (0, 1));
    assert_eq!(report[0].checking_count, 0);
    assert_eq!(report[0].waiting_for_data_count, 1);
    assert_eq!(report[0].waiting_for_results_count, 1);
    assert_eq!(report[0].reviewed_without_status, 0);
    assert_eq!(
        read_evidence(&store, &[definition], &request(), &target)
            .unwrap()
            .status,
        BurnCheckEvidenceStatus::Unavailable
    );
}

#[test]
fn disabled_activation_exposes_no_status_checking_or_waiting_counts_with_old_metadata() {
    let (store, definition) = populated();
    store
        .lock()
        .execute("UPDATE burn_check_activation SET enabled = 0", [])
        .unwrap();
    let report = rows(&store, &[definition]);
    let row = &report[0];
    assert_eq!(
        (
            row.finding,
            row.clean,
            row.checking_count,
            row.waiting_for_data_count,
            row.waiting_for_results_count
        ),
        (0, 0, 0, 0, 0)
    );
    assert!(row.targets.is_empty());
}

#[test]
fn aggregates_project_winners_once_and_keeps_failures_across_projects() {
    let (store, first) = populated();
    let mut second = definition("/workspace/other");
    second.source.scope = InstalledCheckScope::Global;
    activate(&store, &second);
    accept(
        &store,
        &second,
        "other-session",
        4,
        Some(StatusResult::Success),
    );
    let rows = rows(&store, &[first.clone(), second.clone()]);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].finding, rows[0].clean), (1, 2));
    assert_eq!(availability_rows(&[first, second]).len(), 1);
}

#[test]
fn invalidation_and_revision_changes_remove_status_without_manufacturing_success() {
    let (store, mut definition) = populated();
    let target = rows(&store, &[definition.clone()])[0].targets[0].id.clone();
    store
        .lock()
        .execute(
            "UPDATE burn_check_current_status SET evidence_valid = 0 WHERE branch_key = 'branch-a'",
            [],
        )
        .unwrap();
    let current = rows(&store, std::slice::from_ref(&definition));
    assert_eq!(
        (current[0].finding, current[0].clean, current[0].unavailable),
        (0, 1, 1)
    );
    assert_eq!(
        read_evidence(&store, &[definition.clone()], &request(), &target)
            .unwrap()
            .status,
        BurnCheckEvidenceStatus::Unavailable
    );
    definition.definition_revision = "changed".into();
    let current = rows(&store, &[definition]);
    assert_eq!((current[0].finding, current[0].clean), (0, 0));
}

#[test]
fn native_window_excludes_old_and_deleted_sessions_and_replay_or_remote_definitions() {
    let (store, definition) = populated();
    store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = 89 WHERE session_id = 'branch-a'",
            [],
        )
        .unwrap();
    let current = rows(&store, std::slice::from_ref(&definition));
    assert_eq!((current[0].finding, current[0].clean), (0, 1));
    store
        .lock()
        .execute("DELETE FROM session WHERE session_id = 'branch-b'", [])
        .unwrap();
    assert_eq!(rows(&store, std::slice::from_ref(&definition))[0].clean, 0);
    for scope in [
        ActivationScope {
            environment: "ssh:test".into(),
            ..self::scope("/workspace/demo")
        },
        ActivationScope {
            run: RunScope::Replay("test".into()),
            ..self::scope("/workspace/demo")
        },
    ] {
        let definition = InstalledCheckDefinition {
            activation_scope_key: serde_json::to_string(&scope).unwrap(),
            ..definition.clone()
        };
        assert!(rows(&store, std::slice::from_ref(&definition)).is_empty());
        assert!(availability_rows(&[definition]).is_empty());
    }
}

#[test]
fn exact_counts_do_not_depend_on_the_target_display_cap() {
    let store = store();
    let definition = definition("/workspace/demo");
    activate(&store, &definition);
    for position in 1..=(MAX_TARGETS + 2) {
        accept(
            &store,
            &definition,
            &format!("session-{position}"),
            position as i64,
            Some(StatusResult::Failure),
        );
    }
    let current = rows(&store, &[definition]);
    assert_eq!(current[0].finding, (MAX_TARGETS + 2) as u64);
    assert_eq!(current[0].targets.len(), MAX_TARGETS);
    assert!(current[0].targets_truncated);
}

#[test]
fn unavailable_winners_remain_searchable_without_current_results() {
    let (store, mut definition) = populated();
    definition.availability = InstalledCheckAvailability::Invalid {
        reason: Some("Invalid YAML.".into()),
    };
    let current = rows(&store, &[definition.clone()]);
    assert_eq!(current[0].finding, 0);
    assert_eq!(current[0].clean, 0);
    assert_eq!(current[0].diagnostic.as_deref(), Some("Invalid YAML."));
    assert_eq!(
        availability_rows(&[definition])[0].effective_id,
        "installed:failed-retry-loop"
    );
}

#[test]
fn installed_snoozes_preserve_native_records_and_never_enter_native_masks() {
    use crate::dto::{CheckSnoozeId, CheckSnoozePayload};
    let store = store();
    let native: CheckSnoozePayload =
        serde_json::from_value(json!({"detector":"oldModelUsage","scope":"check","until":null}))
            .unwrap();
    let installed: CheckSnoozePayload = serde_json::from_value(
        json!({"detector":"installed:failed-retry-loop","scope":"check","until":123000}),
    )
    .unwrap();
    store.set_check_snooze(&native).unwrap();
    store.set_check_snooze(&installed).unwrap();
    assert_eq!(store.burn_check_snoozes().unwrap().len(), 1);
    assert_eq!(store.check_snoozes().unwrap().len(), 2);
    store.clear_check_snooze(&installed.detector).unwrap();
    assert_eq!(store.check_snoozes().unwrap(), vec![native]);
    assert!(serde_json::from_value::<CheckSnoozeId>(json!("installed:../other")).is_err());
}
