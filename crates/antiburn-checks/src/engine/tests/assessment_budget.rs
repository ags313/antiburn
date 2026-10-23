use super::*;

fn budget_activation() -> EffectiveActivation {
    let mut activation = activation();
    activation.compiled = Arc::new(
        compile(
            "name: Shared budget\non:\n  tool_result:\ndecisions:\n  review:\n    input:\n      selected: ${{ message.tool.arguments.items.*.value }}\n    output:\n      verdict:\n        type: choice\n        question: Is the work useful?\n        criteria: {yes: Useful., no: Not useful.}\nactions:\n  - type: status\n    input: {result: failure}\n  - type: status\n    if: message.tool.arguments.items.*.value\n    input: {result: success}\n",
        )
        .unwrap(),
    );
    activation
}

fn budget_candidate(activation: &EffectiveActivation) -> FrozenCandidate {
    let mut candidate = candidate(activation, 1);
    candidate.context.value["message"]["tool"]["arguments"] = json!({
        "items": (0..600).map(|_| json!({"value": "selected"})).collect::<Vec<_>>()
    });
    candidate
}

#[test]
fn preparation_and_actions_share_the_iteration_budget_before_acceptance() {
    let store = Arc::new(FakeStore::default());
    let engine = CheckEngine::new(store.clone(), Arc::new(FakeEvaluator));
    let activation = budget_activation();
    engine
        .enqueue(&activation, budget_candidate(&activation))
        .unwrap();
    let StartOutcome::Started(task) = engine.start_one_due(&activation, 0).unwrap() else {
        panic!("the complete candidate must start");
    };
    let settlement = run_task(task);
    assert!(matches!(settlement.outcome, RunOutcome::Failed(_)));
    assert_eq!(settlement.attempts, 1);
    assert_eq!(*store.dispatches.lock().unwrap(), 1);
    assert!(store.accepted.lock().unwrap().is_empty());
}

#[test]
fn saved_and_legacy_recovery_keep_preparation_costs_in_the_action_budget() {
    let activation = budget_activation();
    let candidate = budget_candidate(&activation);
    assert!(
        plan_status_actions_with_budget(
            &activation.compiled.actions,
            &candidate.context.value,
            &[],
            None,
            &mut EvaluationBudget::default(),
        )
        .is_ok()
    );
    let mut budget = EvaluationBudget::default();
    let PreparationOutcome::Ready(input) =
        prepare_decision_input_with_budget(&activation.compiled, &candidate, &mut budget)
    else {
        panic!("the input alone fits the run budget");
    };
    let prepared = PreparationRecord::Ready(PreparedDecision {
        profile: "test".into(),
        input,
        request: None,
        report: FitReport::default(),
        evaluation_budget: Some(budget),
    });
    let saved: PreparationRecord =
        serde_json::from_str(&serde_json::to_string(&prepared).unwrap()).unwrap();
    for preparation in [Some(&saved), None] {
        let mut budget =
            restored_evaluation_budget(&activation.compiled, &candidate, preparation).unwrap();
        assert!(
            plan_status_actions_with_budget(
                &activation.compiled.actions,
                &candidate.context.value,
                &[],
                None,
                &mut budget,
            )
            .is_err()
        );
    }
}

#[test]
fn recovery_rejects_changed_or_exhausted_saved_evaluation_limits() {
    let activation = budget_activation();
    let candidate = budget_candidate(&activation);
    let changed = EvaluationBudget::new(
        crate::expression_eval::EvaluationLimits {
            max_steps: 1,
            ..Default::default()
        },
        1_000,
        8 * 1024 * 1024,
    );
    let mut exhausted = EvaluationBudget::default();
    assert!(exhausted.visit_items(1_001).is_err());
    for budget in [changed, exhausted] {
        let preparation = PreparationRecord::Ready(PreparedDecision {
            profile: "test".into(),
            input: json!({}),
            request: None,
            report: FitReport::default(),
            evaluation_budget: Some(budget),
        });
        let saved: PreparationRecord =
            serde_json::from_str(&serde_json::to_string(&preparation).unwrap()).unwrap();
        assert!(
            restored_evaluation_budget(&activation.compiled, &candidate, Some(&saved)).is_err()
        );
    }
}
