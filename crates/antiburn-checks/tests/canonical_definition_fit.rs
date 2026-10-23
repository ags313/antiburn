use antiburn_checks::fit::{
    FitStrategy, FitValidationError, OMISSION_MARKER, PreparationBudget, PreparationBudgetKind,
    PreparationFailureCode, fit_input,
};
use serde_json::json;

mod canonical_support;
use canonical_support::{DEFINITIONS, check, context, input};

fn over_budget() -> FitValidationError {
    FitValidationError::OverBudget(PreparationBudget {
        kind: PreparationBudgetKind::EstimatedTokens,
        observed: 2,
        maximum: 1,
    })
}

#[test]
fn every_definition_fits_only_result_text_without_dropping_evidence_records() {
    for (id, _) in DEFINITIONS {
        let check = check(id);
        let fit = &check.decisions[0].fit;
        assert_eq!(fit.len(), 1, "{id}");
        assert_eq!(fit[0].path, "activity[*].tool.result.content", "{id}");
        assert_eq!(fit[0].strategy, FitStrategy::Truncate, "{id}");
        let context = context(
            &check,
            "Fix the parser. Keep the exception: do not change generated files.",
            &"result output ".repeat(400),
        );
        let original = input(&check, &context);
        let (mut fitted, report) = fit_input(original.clone(), fit, |_, report| {
            if report.omissions.len() == original["activity"].as_array().unwrap().len() {
                Ok(())
            } else {
                Err(over_budget())
            }
        })
        .unwrap();
        assert_eq!(fitted["requests"], original["requests"], "{id}");
        for (index, omission) in report.omissions.iter().enumerate() {
            assert_eq!(
                omission.path,
                format!("activity[{index}].tool.result.content")
            );
            assert!(omission.retained_count < omission.original_count);
            assert!(
                fitted["activity"][index]["tool"]["result"]["content"]
                    .as_str()
                    .unwrap()
                    .contains(OMISSION_MARKER)
            );
            fitted["activity"][index]["tool"]["result"]["content"] =
                original["activity"][index]["tool"]["result"]["content"].clone();
        }
        assert_eq!(fitted, original, "only result text may change: {id}");
    }
}

#[test]
fn oversized_requests_reject_instead_of_losing_corrections_or_approvals() {
    for (id, _) in DEFINITIONS {
        let check = check(id);
        let context = context(
            &check,
            &format!(
                "{} Keep the correction and approval exception.",
                "request ".repeat(500)
            ),
            &"output ".repeat(600),
        );
        let original = input(&check, &context);
        let failure = fit_input(original.clone(), &check.decisions[0].fit, |fitted, _| {
            assert_eq!(fitted["requests"], original["requests"], "{id}");
            assert_eq!(
                fitted["activity"].as_array().unwrap().len(),
                check.trigger.window.unwrap().minimum as usize
            );
            if fitted["requests"][0]["text"].as_str().unwrap().len() < 512 {
                Ok(())
            } else {
                Err(over_budget())
            }
        })
        .unwrap_err();
        assert_eq!(failure.code, PreparationFailureCode::NoFit, "{id}");
        assert!(
            failure
                .report
                .unwrap()
                .omissions
                .iter()
                .all(|entry| entry.path.starts_with("activity["))
        );
    }
}

#[test]
fn oversized_inventories_reject_without_shortening_rules_or_descriptions() {
    for id in ["ignored-instructions", "skill-opportunities"] {
        let check = check(id);
        let mut context = context(&check, "Review the parser.", "read output");
        context["repo"]["instructions"] = json!([{
            "path": "/repo/AGENTS.md", "content": "rule and exception ".repeat(400),
            "scope": {"root": "/repo", "globs": ["src/**"]}, "observed_at": "2026-10-23T12:00:00Z"
        }]);
        context["global"]["skills"] = json!([{
            "name": "review", "description": "review step ".repeat(400), "path": "/config/review/SKILL.md"
        }]);
        let original = input(&check, &context);
        let failure = fit_input(original.clone(), &check.decisions[0].fit, |fitted, _| {
            assert_eq!(
                fitted["current_global_scope"],
                original["current_global_scope"]
            );
            assert_eq!(fitted["current_repo_scope"], original["current_repo_scope"]);
            Err(over_budget())
        })
        .unwrap_err();
        assert_eq!(failure.code, PreparationFailureCode::NoFit, "{id}");
        assert!(failure.report.unwrap().omissions.is_empty());
    }
}

#[test]
fn missing_required_requests_do_not_become_an_empty_history() {
    for (id, _) in DEFINITIONS {
        let check = check(id);
        let mut context = context(&check, "Review the parser.", "read output");
        context["session"]
            .as_object_mut()
            .unwrap()
            .remove("requests");
        let evaluated = antiburn_checks::preparation::evaluate_decision_input_with_budget(
            &check.decisions[0],
            &context,
            &mut antiburn_checks::expression_eval::EvaluationBudget::default(),
        )
        .unwrap();
        assert_eq!(
            evaluated.value,
            antiburn_checks::expression_eval::EvaluationValue::Unavailable,
            "{id}"
        );
    }
}
