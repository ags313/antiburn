use super::*;

struct LabelOnlyEvaluator(&'static str);

impl DecisionEvaluator for LabelOnlyEvaluator {
    fn evaluate<'a>(
        &'a self,
        activation: &'a EffectiveActivation,
        _: &'a crate::domain::FrozenCandidate,
        _: &'a serde_json::Value,
        _: u8,
    ) -> DecisionFuture<'a> {
        Box::pin(async move {
            let decision = activation
                .compiled
                .decision
                .as_ref()
                .expect("model fixture");
            let labels = decision
                .criteria
                .iter()
                .map(|(label, _)| label.clone())
                .collect::<Vec<_>>();
            Ok(EvaluatedCandidate {
                choices: vec![
                    ChoiceResult::validate_label_only(
                        &decision.name,
                        &decision.question_id,
                        &labels,
                        self.0,
                    )
                    .unwrap(),
                ],
                attempts: 1,
            })
        })
    }
}

#[test]
fn label_only_replay_persists_answers_and_only_explicit_label_status() {
    let mut activation = activation();
    let check = Arc::make_mut(&mut activation.compiled);
    check.actions = vec![crate::compiler::StatusAction {
        condition: Some(crate::compiler::Condition::Expression(
            crate::expression::parse_expression("decisions.review.outputs.verdict == 'stuck'")
                .unwrap(),
        )),
        result: crate::compiler::StatusResult::Failure.into(),
        message: None,
    }];
    for (output, expected) in [
        ("stuck", Some(crate::compiler::StatusResult::Failure)),
        ("clear", None),
        ("uncertain", None),
    ] {
        let store = Arc::new(MemoryStore::default());
        let report = block_on(replay(
            ReplayRunId("label-only".into()),
            activation.clone(),
            store.clone(),
            Arc::new(LabelOnlyEvaluator(output)),
            &mut Events((1..=3).map(event).collect::<Vec<_>>().into_iter()),
            small_limits(),
            || false,
        ))
        .unwrap();
        assert_eq!(report.end, ReplayEnd::Complete);
        assert_eq!((report.evaluations, report.attempts), (1, 1));
        let answers = store.answers.lock().unwrap();
        assert_eq!(answers.len(), 1);
        let saved = serde_json::to_value(&answers[0]).unwrap();
        assert!(
            !saved["choices"][0]
                .as_object()
                .unwrap()
                .contains_key("probabilities")
        );
        let restored: EvaluatedCandidate = serde_json::from_value(saved).unwrap();
        restored.validate(&activation.compiled).unwrap();
        assert_eq!(restored, answers[0]);
        let accepted = store.accepted.lock().unwrap();
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].status_write(), expected);
        assert_eq!(accepted[0].actions.current_status, expected);
    }
}

#[test]
fn probability_checks_reject_label_only_replay_before_answer_or_success() {
    for output in ["stuck", "clear", "uncertain"] {
        let store = Arc::new(MemoryStore::default());
        let report = block_on(replay(
            ReplayRunId("missing-probabilities".into()),
            activation(),
            store.clone(),
            Arc::new(LabelOnlyEvaluator(output)),
            &mut Events((1..=3).map(event).collect::<Vec<_>>().into_iter()),
            small_limits(),
            || false,
        ))
        .unwrap();
        assert!(matches!(report.end, ReplayEnd::ExecutionError(_)));
        assert_eq!((report.evaluations, report.attempts), (1, 1));
        assert!(matches!(
            report.steps.last().unwrap().outcome,
            Some(RunOutcome::Failed(_))
        ));
        assert_eq!(*store.failures.lock().unwrap(), [AttemptFailure::Permanent]);
        assert!(store.answers.lock().unwrap().is_empty());
        assert!(store.accepted.lock().unwrap().is_empty());
    }
}

#[test]
fn stored_label_only_recovery_keeps_prior_failure_and_never_resends() {
    for reads_probabilities in [false, true] {
        let mut activation = activation();
        if !reads_probabilities {
            Arc::make_mut(&mut activation.compiled).actions = vec![crate::compiler::StatusAction {
                condition: Some(crate::compiler::Condition::Expression(
                    crate::expression::parse_expression(
                        "decisions.review.outputs.verdict == 'stuck'",
                    )
                    .unwrap(),
                )),
                result: crate::compiler::StatusResult::Failure.into(),
                message: None,
            }];
        }
        let saved = json!({
            "choices": [{"decision": "review", "question": "verdict", "output": "clear"}],
            "attempts": 1,
        });
        let evaluated: EvaluatedCandidate = serde_json::from_value(saved).unwrap();
        let candidate = crate::domain::FrozenCandidate {
            activation: activation.id.clone(),
            branch: BranchId("main".into()),
            trigger_position: ObservationPosition(3),
            messages: Vec::new(),
            context: FrozenContext {
                value: json!({
                    "session": {"requests": [], "requests_complete": true, "source_coverage": "complete"},
                    "window": {"messages": [
                        {"tool": {"result": {"status": "error"}}},
                        {"tool": {"result": {"status": "error"}}},
                        {"tool": {"result": {"status": "error"}}}
                    ]},
                }),
                dependencies: Vec::new(),
            },
            dependencies: Vec::new(),
            fingerprint: [3; 32],
        };
        let store = Arc::new(MemoryStore::default());
        store.recovery.lock().unwrap().push(RecoveryRecord {
            activation,
            work: crate::check_store::RecoveryWork::Answered {
                claimed: ClaimedCandidate {
                    id: CandidateId::from(&candidate),
                    candidate,
                    prior_status: Some(crate::compiler::StatusResult::Failure),
                    fence: 1,
                    next_attempt: 1,
                },
                evaluated: evaluated.clone(),
            },
        });
        let evaluator = Arc::new(ScriptedEvaluator {
            inputs: Mutex::new(Vec::new()),
        });
        let engine = CheckEngine::new(store.clone(), evaluator.clone());
        let recovered = engine.recover_pending();
        if reads_probabilities {
            assert_eq!(recovered.unwrap().candidates, 0);
            let failures = store.evaluation_failures.lock().unwrap();
            assert_eq!(failures.len(), 1);
            assert_eq!(
                failures[0].code,
                crate::engine::EvaluationFailureCode::InvalidDecision
            );
            assert!(failures[0].detail.contains("probabilities"));
            assert!(store.accepted.lock().unwrap().is_empty());
        } else {
            assert_eq!(recovered.unwrap().candidates, 0);
            let accepted = store.accepted.lock().unwrap();
            assert_eq!(accepted.len(), 1);
            assert_eq!(accepted[0].evaluated, evaluated);
            assert_eq!(accepted[0].status_write(), None);
            assert_eq!(
                accepted[0].actions.current_status,
                Some(crate::compiler::StatusResult::Failure),
            );
        }
        assert!(evaluator.inputs.lock().unwrap().is_empty());
        assert!(store.dispatches.lock().unwrap().is_empty());
        assert!(store.answers.lock().unwrap().is_empty());
    }
}
