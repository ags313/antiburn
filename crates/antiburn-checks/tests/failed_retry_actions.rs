use std::collections::BTreeMap;

use antiburn_checks::actions::{ActionOutcomeKind, ChoiceResult, plan_status_actions};
use antiburn_checks::compiler::StatusResult;
use serde_json::json;

#[test]
fn retry_loop_writes_success_for_uncertainty_or_below_threshold_answers() {
    let check = antiburn_checks::compile(include_str!(
        "../../../docs/burn-check/examples/failed-retry-loop.yml"
    ))
    .unwrap();
    let labels = check
        .decision
        .as_ref()
        .expect("model fixture")
        .criteria
        .iter()
        .map(|(label, _)| label.clone())
        .collect::<Vec<_>>();
    for (output, stuck, clear, uncertain) in [
        ("uncertain", 0.01, 0.01, 0.98),
        ("stuck", 0.94, 0.03, 0.03),
        ("clear", 0.03, 0.94, 0.03),
    ] {
        let choice = ChoiceResult::validate(
            "review",
            "verdict",
            &labels,
            output,
            BTreeMap::from([
                ("stuck".to_owned(), stuck),
                ("clear".to_owned(), clear),
                ("uncertain".to_owned(), uncertain),
            ]),
        )
        .unwrap();
        for prior in [
            None,
            Some(StatusResult::Failure),
            Some(StatusResult::Success),
        ] {
            let plan = plan_status_actions(
                &check.actions,
                &json!({}),
                std::slice::from_ref(&choice),
                prior,
            )
            .unwrap();
            assert_eq!(plan.current_status, Some(StatusResult::Success));
            assert_eq!(plan.outcomes.len(), 2);
            assert_eq!(
                plan.outcomes[0].outcome,
                ActionOutcomeKind::SkippedConditionFalse
            );
            assert_eq!(
                plan.outcomes[1].outcome,
                ActionOutcomeKind::Applied {
                    result: StatusResult::Success,
                    message: None,
                }
            );
        }
    }
}

#[test]
fn retry_loop_recovery_clears_a_confident_failure() {
    let check = antiburn_checks::compile(include_str!(
        "../../../docs/burn-check/examples/failed-retry-loop.yml"
    ))
    .unwrap();
    let labels = check
        .decision
        .as_ref()
        .expect("model fixture")
        .criteria
        .iter()
        .map(|(label, _)| label.clone())
        .collect::<Vec<_>>();
    let mut status = None;
    for (output, stuck, clear, expected) in [
        ("stuck", 0.98, 0.01, StatusResult::Failure),
        ("clear", 0.01, 0.98, StatusResult::Success),
    ] {
        let choice = ChoiceResult::validate(
            "review",
            "verdict",
            &labels,
            output,
            BTreeMap::from([
                ("stuck".to_owned(), stuck),
                ("clear".to_owned(), clear),
                ("uncertain".to_owned(), 0.01),
            ]),
        )
        .unwrap();
        let plan = plan_status_actions(&check.actions, &json!({}), &[choice], status).unwrap();
        assert_eq!(plan.current_status, Some(expected));
        status = plan.current_status;
    }
}
