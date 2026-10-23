use std::collections::BTreeMap;

use antiburn_checks::actions::{ActionOutcomeKind, ChoiceResult, plan_status_actions};
use antiburn_checks::check_store::EvaluatedCandidate;
use antiburn_checks::compiler::{Condition, StatusAction, StatusResult};
use antiburn_checks::diagnostics::DiagnosticCode;
use antiburn_checks::expression::parse_expression;
use serde_json::json;

fn labels() -> Vec<String> {
    vec!["stuck".into(), "clear".into(), "uncertain".into()]
}

fn label_only(output: &str) -> ChoiceResult {
    ChoiceResult::validate_label_only("review", "verdict", &labels(), output).unwrap()
}

fn check() -> antiburn_checks::compiler::CompiledCheck {
    antiburn_checks::compile(include_str!(
        "../../../docs/burn-check/examples/failed-retry-loop.yml"
    ))
    .unwrap()
}

#[test]
fn label_only_results_validate_labels_without_adding_details() {
    let choice = label_only("stuck");
    assert_eq!(
        serde_json::to_value(&choice).unwrap(),
        json!({"decision": "review", "question": "verdict", "output": "stuck"})
    );
    choice.validate_declaration(&check().decision).unwrap();
    for (decision, question, declared, output) in [
        ("", "verdict", labels(), "stuck"),
        ("review", "", labels(), "stuck"),
        ("review", "verdict", labels(), ""),
        ("review", "verdict", labels(), "other"),
        ("review", "verdict", vec!["stuck".into()], "stuck"),
        (
            "review",
            "verdict",
            vec!["stuck".into(), "stuck".into()],
            "stuck",
        ),
    ] {
        assert!(ChoiceResult::validate_label_only(decision, question, &declared, output).is_err());
    }
    for (decision, question) in [("other", "verdict"), ("review", "other")] {
        let choice =
            ChoiceResult::validate_label_only(decision, question, &labels(), "stuck").unwrap();
        assert!(choice.validate_declaration(&check().decision).is_err());
    }
}

#[test]
fn present_probability_maps_remain_strict() {
    for probabilities in [
        BTreeMap::new(),
        BTreeMap::from([("stuck".into(), 1.0)]),
        BTreeMap::from([("stuck".into(), 0.5), ("clear".into(), 0.5)]),
        BTreeMap::from([
            ("stuck".into(), f64::NAN),
            ("clear".into(), 0.0),
            ("uncertain".into(), 0.0),
        ]),
        BTreeMap::from([
            ("stuck".into(), 1.1),
            ("clear".into(), -0.1),
            ("uncertain".into(), 0.0),
        ]),
        BTreeMap::from([
            ("stuck".into(), 0.1),
            ("clear".into(), 0.1),
            ("uncertain".into(), 0.1),
        ]),
    ] {
        assert!(
            ChoiceResult::validate("review", "verdict", &labels(), "stuck", probabilities).is_err()
        );
    }
}

#[test]
fn absent_details_do_not_create_null_matches_or_fallback_success() {
    let conditions = [
        "decisions.review.details.verdict.probabilities.stuck >= 0.95",
        "!(decisions.review.details.verdict.probabilities.stuck >= 0.95)",
        "decisions.review.details.verdict.probabilities.stuck == null",
        "decisions.review.details.verdict.probabilities.stuck || true",
    ];
    let actions = conditions.map(|condition| StatusAction {
        condition: Some(Condition::Expression(parse_expression(condition).unwrap())),
        result: StatusResult::Success.into(),
        message: None,
    });
    for prior in [
        None,
        Some(StatusResult::Failure),
        Some(StatusResult::Success),
    ] {
        let plan =
            plan_status_actions(&actions, &json!({}), &[label_only("stuck")], prior).unwrap();
        assert_eq!(plan.current_status, prior);
        assert!(
            plan.outcomes
                .iter()
                .all(|action| { action.outcome == ActionOutcomeKind::SkippedConditionUnavailable })
        );
    }
}

#[test]
fn stored_label_only_answers_require_declaration_and_action_compatibility() {
    let probability_check = check();
    let mut label_check = check();
    label_check.actions = vec![StatusAction {
        condition: Some(Condition::Expression(
            parse_expression("decisions.review.outputs.verdict == 'stuck'").unwrap(),
        )),
        result: StatusResult::Failure.into(),
        message: None,
    }];
    assert!(!label_check.choice_requirements().probabilities);
    for output in labels() {
        let answer = EvaluatedCandidate {
            choices: vec![label_only(&output)],
            attempts: 1,
        };
        let bytes = serde_json::to_vec(&answer).unwrap();
        let restored: EvaluatedCandidate = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, answer);
        restored.validate(&label_check).unwrap();
        let error = restored.validate(&probability_check).unwrap_err();
        assert_eq!(error.code, DiagnosticCode::RuntimeInvalidDecision);
        assert!(error.message.contains("probabilities"));
        let plan =
            plan_status_actions(&label_check.actions, &json!({}), &restored.choices, None).unwrap();
        assert_eq!(
            plan.current_status,
            (output == "stuck").then_some(StatusResult::Failure),
        );
    }
    for field in ["decision", "question", "output"] {
        let mut saved = serde_json::to_value(EvaluatedCandidate {
            choices: vec![label_only("stuck")],
            attempts: 1,
        })
        .unwrap();
        saved["choices"][0][field] = json!("unknown");
        let restored: EvaluatedCandidate = serde_json::from_value(saved).unwrap();
        assert!(restored.validate(&label_check).is_err(), "{field}");
    }
}

#[test]
fn stored_absence_is_distinct_from_null_empty_or_partial_maps() {
    let base = json!({
        "choices": [{"decision": "review", "question": "verdict", "output": "stuck"}],
        "attempts": 1,
    });
    for probabilities in [
        json!(null),
        json!({}),
        json!({"stuck": 1.0}),
        json!({"stuck": 0.1, "clear": 0.1, "uncertain": 0.1}),
        json!({"stuck": 1.1, "clear": -0.1, "uncertain": 0.0}),
    ] {
        let mut saved = base.clone();
        saved["choices"][0]["probabilities"] = probabilities;
        assert!(serde_json::from_value::<EvaluatedCandidate>(saved).is_err());
    }
    for probabilities in [
        json!({"stuck": 0.5, "clear": 0.5}),
        json!({"stuck": 0.5, "clear": 0.2, "uncertain": 0.2, "extra": 0.1}),
    ] {
        let mut saved = base.clone();
        saved["choices"][0]["probabilities"] = probabilities;
        let restored: EvaluatedCandidate = serde_json::from_value(saved).unwrap();
        assert!(restored.validate(&check()).is_err());
    }
    for field in ["decision", "question", "output"] {
        let mut saved = base.clone();
        saved["choices"][0][field] = json!("");
        assert!(serde_json::from_value::<EvaluatedCandidate>(saved).is_err());
    }
    let mut confidence = base;
    confidence["choices"][0]["confidence"] = json!(0.99);
    assert!(serde_json::from_value::<EvaluatedCandidate>(confidence).is_err());
}
