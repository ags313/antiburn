use antiburn_checks::actions::plan_status_actions;
use antiburn_checks::compile;
use antiburn_checks::compiler::StatusResult;
use antiburn_checks::results::EvaluatedAssessment;
use serde_json::json;

#[test]
fn installed_skill_example_compiles_and_keeps_its_stated_result_policy() {
    let skill = include_str!("../../../.agents/skills/antiburn-checks/SKILL.md");
    let (_, example) = skill.split_once("```yaml\n").unwrap();
    let (source, _) = example.split_once("\n```").unwrap();
    let check = compile(source).unwrap();
    assert!(check.is_local());
    assert!(check.question_requirements().is_empty());
    for (status, prior, expected) in [
        ("cancelled", None, Some(StatusResult::Failure)),
        ("completed", Some(StatusResult::Failure), Some(StatusResult::Success)),
        ("error", None, None),
        ("unknown", Some(StatusResult::Failure), Some(StatusResult::Failure)),
    ] {
        let plan = plan_status_actions(
            &check.actions,
            &json!({"message": {"tool": {"result": {"status": status}}}}),
            &EvaluatedAssessment::local(),
            prior,
        )
        .unwrap();
        assert_eq!(plan.current_status, expected, "{status}");
    }
}
