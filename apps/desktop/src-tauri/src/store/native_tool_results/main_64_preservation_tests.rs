use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use antiburn_checks::actions::plan_status_actions;
use antiburn_checks::check_store::{
    AcceptanceOutcome, AssessmentAcceptance, AttemptFailure, CandidateClaim, CheckStore,
    ClaimOutcome, ClaimedCandidate, EvaluatedAssessment, RecoveryWork,
};
use antiburn_checks::compiler::compile;
use antiburn_checks::decision::{DecisionAttempt, DecisionInput};
use antiburn_checks::definition_lifecycle::{EffectiveActivation, LifecycleTransition};
use antiburn_checks::discovery::DefinitionScope;
use antiburn_checks::domain::{
    ActivationId, ActivationScope, FrozenCandidate, Generation, ObservationPosition, RunScope,
};
use antiburn_checks::engine::{
    CheckEngine, DecisionEvaluator, DecisionFuture, EvaluationDiagnostic, EvaluationFailureCode,
    PreparationValidationPurpose,
};
use antiburn_checks::expression_eval::EvaluationBudget;
use antiburn_checks::fit::{PreparationFailure, PreparationRecord};
use antiburn_checks::prepared_assessment::{PreparedAssessment, PreparedBlock};
use antiburn_checks::results::{ChoiceResult, DecisionResult, NamedAnswer, QuestionResult};
use antiburn_checks::source::{PublicationCursor, PublishedEvidenceSource};
use serde_json::json;

use crate::check_store::StoreCheckStore;
use crate::published_evidence_reader::StorePublishedEvidenceReader;
use crate::store::Store;

struct NoProvider;

impl DecisionEvaluator for NoProvider {
    fn prepare(
        &self,
        _: &EffectiveActivation,
        _: Vec<DecisionInput>,
        _: EvaluationBudget,
    ) -> Result<PreparedAssessment, PreparationFailure> {
        panic!("receipt tests must not prepare provider requests")
    }

    fn validate_prepared(
        &self,
        _: &EffectiveActivation,
        _: &PreparedAssessment,
        _: PreparationValidationPurpose,
    ) -> Result<(), PreparationFailure> {
        panic!("receipt tests must not validate provider requests")
    }

    fn evaluate_prepared<'a>(
        &'a self,
        _: &'a EffectiveActivation,
        _: &'a FrozenCandidate,
        _: &'a PreparedAssessment,
        _: &'a DecisionAttempt,
    ) -> DecisionFuture<'a> {
        panic!("receipt tests must not send provider requests")
    }
}

pub(super) struct SavedWork {
    activations: Vec<EffectiveActivation>,
    accepted: AssessmentAcceptance,
    answered: AssessmentAcceptance,
    unknown: ClaimedCandidate,
    rows: Vec<String>,
}

fn rows(store: &Store, table: &str, columns: &str) -> String {
    store
        .lock()
        .query_row(
            &format!("SELECT json_group_array(json(row)) FROM (SELECT json_array({columns}) AS row FROM {table} ORDER BY row)"),
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn saved_rows(store: &Store) -> Vec<String> {
    let mut saved: Vec<_> = [
        ("burn_check_candidate", "candidate_id,state,claim_owner,claim_fence,frozen_input_json,input_fingerprint,preparation_json,dependencies_json,answer_json,diagnostic_json,started_cooldown_until_epoch"),
        ("burn_check_evaluation_attempt", "attempt_id,candidate_id,decision_name,attempt_number,state,outcome,response_json,updated_at_epoch"),
        ("burn_check_current_status", "check_id,branch_key,status,assessment_id,position,evidence_valid,updated_at_epoch"),
        ("burn_check_assessment_history", "assessment_id,candidate_id,outcome,answer_json,actions_json,evidence_valid,accepted_at_epoch"),
        ("burn_check_acceptance_watermark", "check_id,branch_key,generation,position"),
        ("burn_check_cooldown", "check_id,branch_key,generation,cooldown_until_epoch"),
        ("burn_check_decision_usage", "attempt_id,usage_known,served_model"),
        ("burn_check_activation", "check_id,generation,applied_sequence,checkpoint_json"),
    ]
    .map(|(table, columns)| rows(store, table, columns))
    .into();
    saved.push(serde_json::to_string(&store.burn_check_usage_summary().unwrap()).unwrap());
    saved
}

pub(super) fn seed(store: &Arc<Store>) -> SavedWork {
    store
        .set_internal_value_checked("internal:yamlBurnChecksDiscoveryEnabledV1", "true")
        .unwrap();
    let scope = ActivationScope {
        environment: "native".into(),
        session_cwd: "/synthetic".into(),
        run: RunScope::Live,
    };
    let adapter = Arc::new(
        StoreCheckStore::new(store.clone(), "main-64-paid".into(), scope.clone()).unwrap(),
    );
    let mut activations = Vec::new();
    let mut assessments = Vec::new();
    let mut unknown = None;
    for state in ["accepted", "answered", "unknown"] {
        let active = EffectiveActivation {
            id: ActivationId {
                scope: scope.clone(),
                definition_id: format!("main-64-{state}"),
                generation: Generation(1),
            },
            revision: [7; 32],
            definition_scope: DefinitionScope::Global,
            source_paths: Vec::new(),
            project_root: None,
            compiled: Arc::new(compile("name: Receipt preservation\non:\n  tool_result:\ndecisions:\n  review:\n    input: {session: '${{ session.id }}'}\n    output:\n      verdict:\n        type: choice\n        question: Is this useful?\n        criteria: {yes: Yes., no: No.}\nactions:\n  - type: status\n    input: {result: failure}\n").unwrap()),
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
        let page = PublishedEvidenceSource::new(
            StorePublishedEvidenceReader::for_project(store, "native", "/synthetic")
                .with_activation(&active.id),
            100,
        )
        .unwrap()
        .read_batch(PublicationCursor(0))
        .unwrap();
        assert!(!page.has_more);
        assert_eq!(
            CheckEngine::new(adapter.clone(), Arc::new(NoProvider))
                .apply_source_page(&active, &page)
                .unwrap()
                .candidates,
            1
        );
        let candidate = adapter
            .load_recovery(None, 10)
            .unwrap()
            .into_iter()
            .find_map(|record| match record.work {
                RecoveryWork::Pending { candidate, .. } if candidate.activation == active.id => {
                    Some(candidate)
                }
                _ => None,
            })
            .unwrap();
        let CandidateClaim::Claimed(claimed) =
            adapter.claim_due(&candidate, 100_000, true).unwrap()
        else {
            panic!("the saved result must create claimable synthetic work")
        };
        adapter
            .record_preparation(
                &claimed,
                &PreparationRecord::Ready(
                    PreparedAssessment::new(
                        "synthetic-v1".into(),
                        EvaluationBudget::default(),
                        0,
                        vec![PreparedBlock {
                            decision: "review".into(),
                            original_input: json!({"session":candidate.branch.0}),
                            input: json!({"session":candidate.branch.0}),
                            request: json!({"exact":"saved synthetic request"}),
                            report: Default::default(),
                        }],
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
        let attempt = DecisionAttempt::new("review", 1).unwrap();
        assert_eq!(
            adapter
                .record_dispatch_intent(&claimed, 100_000, Duration::from_secs(60), &attempt)
                .unwrap(),
            ClaimOutcome::Current
        );
        if state == "unknown" {
            adapter
                .record_attempt_failure(
                    &claimed,
                    &attempt,
                    AttemptFailure::Unknown,
                    &EvaluationDiagnostic {
                        code: EvaluationFailureCode::DeliveryUnknown,
                        detail: "Synthetic unknown delivery.".into(),
                    },
                )
                .unwrap();
            unknown = Some(*claimed);
        } else {
            let declaration = &active.compiled.decisions[0];
            let result = DecisionResult::new(
                declaration,
                &active.compiled.question_requirements(),
                vec![
                    NamedAnswer::new(
                        "verdict",
                        QuestionResult::Choice(
                            ChoiceResult::new(
                                &declaration.outputs[0].question,
                                "yes",
                                Some(BTreeMap::from([("yes".into(), 0.99), ("no".into(), 0.01)])),
                                None,
                            )
                            .unwrap(),
                        ),
                    )
                    .unwrap(),
                ],
                1,
            )
            .unwrap();
            adapter
                .record_decision_answer(&claimed, &attempt, &result)
                .unwrap();
            let evaluated = EvaluatedAssessment::new(
                &active.compiled.decisions,
                &active.compiled.question_requirements(),
                vec![result],
            )
            .unwrap();
            let actions = plan_status_actions(
                &active.compiled.actions,
                &claimed.candidate.context.value,
                &evaluated,
                claimed.prior_status,
            )
            .unwrap();
            let assessment = AssessmentAcceptance {
                claimed: *claimed,
                evaluated,
                actions,
            };
            if state == "accepted" {
                assert_eq!(
                    adapter.accept(&assessment).unwrap(),
                    AcceptanceOutcome::Accepted
                );
            }
            assessments.push(assessment);
        }
        let candidate_id: String = store
            .lock()
            .query_row(
                "SELECT candidate_id FROM burn_check_candidate WHERE check_id=?1",
                [&active.id.definition_id],
                |row| row.get(0),
            )
            .unwrap();
        store
            .record_burn_check_decision_usage(
                &candidate_id,
                &attempt,
                &active.id.definition_id,
                ("custom-systemone", "synthetic-model"),
                (state != "unknown").then_some(antiburn_local::analysis::jev::JevUsage {
                    input_tokens: 7,
                    output_tokens: 3,
                }),
                100,
            )
            .unwrap();
        activations.push(active);
    }
    drop(adapter);
    // Startup settles the unknown attempt before the converter begins.
    let adapter = StoreCheckStore::new(store.clone(), "main-64-paid".into(), scope).unwrap();
    drop(adapter);
    let usage = store.burn_check_usage_summary().unwrap();
    assert_eq!((usage.confirmed_calls, usage.unknown_outcomes), (2, 1));
    assert_eq!(
        rows(store, "burn_check_current_status", "evidence_valid"),
        "[[1]]"
    );
    SavedWork {
        activations,
        accepted: assessments.remove(0),
        answered: assessments.remove(0),
        unknown: unknown.unwrap(),
        rows: saved_rows(store),
    }
}

impl SavedWork {
    pub(super) fn assert_preserved(&self, store: &Store) {
        assert_eq!(saved_rows(store), self.rows);
    }

    pub(super) fn assert_paid_fences(self, store: &Arc<Store>, unproved_claude: bool) {
        self.assert_preserved(store);
        let (_, agent, _): (String, String, String) =
            serde_json::from_str(&self.answered.claimed.candidate.branch.0).unwrap();
        assert_eq!(agent == "claude-code", unproved_claude);
        let immutable_inputs = rows(
            store,
            "burn_check_candidate",
            "candidate_id,frozen_input_json,input_fingerprint,preparation_json,dependencies_json,answer_json,diagnostic_json,started_cooldown_until_epoch",
        );
        let attempts = self.rows[1].clone();
        let usage = store.burn_check_usage_summary().unwrap();
        let adapter = StoreCheckStore::new(
            store.clone(),
            "main-64-paid-restart".into(),
            self.activations[0].id.scope.clone(),
        )
        .unwrap();
        for active in &self.activations {
            adapter.register_activation(active).unwrap();
        }
        assert_eq!(
            adapter.accept(&self.accepted).unwrap(),
            AcceptanceOutcome::AlreadyAccepted
        );
        for saved in [&self.accepted.claimed, &self.unknown] {
            assert_eq!(
                adapter.claim_due(&saved.candidate, 200_000, true).unwrap(),
                CandidateClaim::Stale
            );
            assert_eq!(
                adapter
                    .record_dispatch_intent(
                        saved,
                        200_000,
                        Duration::ZERO,
                        &DecisionAttempt::new("review", 2).unwrap()
                    )
                    .unwrap(),
                ClaimOutcome::Stale
            );
        }
        let mut recovery = adapter.load_recovery(None, 10).unwrap();
        assert_eq!(recovery.len(), 1);
        let RecoveryWork::Answered { claimed, evaluated } = recovery.remove(0).work else {
            panic!("the paid answer must recover without a new attempt")
        };
        assert_eq!(evaluated, self.answered.evaluated);
        assert_eq!(claimed.candidate, self.answered.claimed.candidate);
        if unproved_claude {
            let candidate_id: String = store.lock().query_row("SELECT candidate_id FROM burn_check_candidate WHERE check_id='main-64-answered'", [], |row| row.get(0)).unwrap();
            assert!(!crate::store::native_tool_results::request_origin::new_candidate_request_origin_current_in(&store.lock(), &candidate_id).unwrap());
        }
        let dispatch = adapter.record_dispatch_intent(
            &claimed,
            200_000,
            Duration::ZERO,
            &DecisionAttempt::new("review", 2).unwrap(),
        );
        if unproved_claude {
            assert_eq!(dispatch.unwrap(), ClaimOutcome::Stale);
        } else {
            assert!(dispatch.is_err());
        }
        assert_eq!(
            adapter
                .accept(&AssessmentAcceptance {
                    claimed,
                    evaluated,
                    actions: self.answered.actions
                })
                .unwrap(),
            if unproved_claude {
                AcceptanceOutcome::Stale
            } else {
                AcceptanceOutcome::Deferred
            }
        );
        assert_eq!(
            adapter.load_recovery(None, 10).unwrap().len(),
            usize::from(!unproved_claude)
        );
        assert_eq!(
            rows(
                store,
                "burn_check_evaluation_attempt",
                "attempt_id,candidate_id,decision_name,attempt_number,state,outcome,response_json,updated_at_epoch"
            ),
            attempts
        );
        assert_eq!(store.burn_check_usage_summary().unwrap(), usage);
        assert_eq!(
            rows(
                store,
                "burn_check_candidate",
                "candidate_id,frozen_input_json,input_fingerprint,preparation_json,dependencies_json,answer_json,diagnostic_json,started_cooldown_until_epoch"
            ),
            immutable_inputs
        );
        if unproved_claude {
            assert_eq!(&saved_rows(store)[1..], &self.rows[1..]);
        }
    }
}
