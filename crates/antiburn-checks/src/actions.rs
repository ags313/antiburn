//! Pure ordered planning for independently gated status actions.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::compiler::{StatusAction, StatusResult, StatusResultTemplate};
use crate::diagnostics::DiagnosticCode;
use crate::expression_eval::{
    EvaluationBindings, EvaluationBudget, EvaluationError, EvaluationTrace, EvaluationValue,
    evaluate_expression_with_budget, is_truthy,
};
use crate::preparation::{enforce_selected_membership, evaluate_condition, prepare_template};

const PROBABILITY_SUM_TOLERANCE: f64 = 0.02;

/// A validated result for one declared Choice question.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChoiceResult {
    decision: String,
    question: String,
    output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    probabilities: Option<BTreeMap<String, f64>>,
}

impl<'de> Deserialize<'de> for ChoiceResult {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct SavedChoice {
            decision: String,
            question: String,
            output: String,
            #[serde(default, deserialize_with = "deserialize_present_probabilities")]
            probabilities: Option<BTreeMap<String, f64>>,
        }
        let saved = SavedChoice::deserialize(deserializer)?;
        match saved.probabilities {
            Some(probabilities) => {
                let labels = probabilities.keys().cloned().collect::<Vec<_>>();
                Self::validate(
                    saved.decision,
                    saved.question,
                    &labels,
                    saved.output,
                    probabilities,
                )
                .map_err(|error| serde::de::Error::custom(error.message))
            }
            None => {
                if saved.decision.is_empty() || saved.question.is_empty() || saved.output.is_empty()
                {
                    return Err(serde::de::Error::custom(
                        "Choice decision, question, and output must be nonempty.",
                    ));
                }
                Ok(Self {
                    decision: saved.decision,
                    question: saved.question,
                    output: saved.output,
                    probabilities: None,
                })
            }
        }
    }
}

fn deserialize_present_probabilities<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BTreeMap<String, f64>>, D::Error> {
    BTreeMap::deserialize(deserializer).map(Some)
}

impl ChoiceResult {
    /// Validates a selected label and its complete probability map.
    ///
    /// Every declared label must have one finite probability in `[0, 1]`, and
    /// the absolute error of their sum from one must not exceed `0.02` under
    /// `f64` arithmetic.
    /// Keep each probability unchanged; action thresholds read the raw values.
    /// The selected output must match a declared label exactly. Labels have no
    /// implicit behavior.
    ///
    /// # Errors
    /// Returns an error if identifiers, labels, probabilities, or their sum
    /// violate the Choice contract.
    pub fn validate(
        decision: impl Into<String>,
        question: impl Into<String>,
        labels: &[String],
        output: impl Into<String>,
        probabilities: BTreeMap<String, f64>,
    ) -> Result<Self, ChoiceResultError> {
        let mut result = Self::validate_label_only(decision, question, labels, output)?;
        let expected = labels.iter().collect::<BTreeSet<_>>();
        if probabilities.len() != expected.len()
            || probabilities.keys().any(|label| !expected.contains(label))
        {
            return Err(choice_error(
                "Choice probabilities must contain exactly the declared labels.",
            ));
        }
        let mut sum = 0.0;
        for probability in probabilities.values() {
            if !probability.is_finite() || !(0.0..=1.0).contains(probability) {
                return Err(choice_error(
                    "Choice probabilities must be finite values from zero to one.",
                ));
            }
            sum += probability;
        }
        if (sum - 1.0).abs() > PROBABILITY_SUM_TOLERANCE {
            return Err(choice_error(
                "Choice probability sum must differ from one by no more than 0.02.",
            ));
        }
        result.probabilities = Some(probabilities);
        Ok(result)
    }

    /// Validates a label from an adapter without label probabilities.
    ///
    /// The caller must verify that the adapter does not declare probabilities.
    /// Do not use this method to accept missing or invalid probability data from
    /// an adapter that declares it. Missing details remain unavailable to actions.
    ///
    /// # Errors
    /// Returns an error for invalid identifiers, labels, or an undeclared output.
    pub fn validate_label_only(
        decision: impl Into<String>,
        question: impl Into<String>,
        labels: &[String],
        output: impl Into<String>,
    ) -> Result<Self, ChoiceResultError> {
        let decision = decision.into();
        let question = question.into();
        let output = output.into();
        let expected = labels.iter().collect::<BTreeSet<_>>();
        if decision.is_empty() || question.is_empty() || output.is_empty() {
            return Err(choice_error(
                "Choice decision, question, and output must be nonempty.",
            ));
        }
        if labels.len() < 2 || expected.len() != labels.len() {
            return Err(choice_error("Choice requires at least two unique labels."));
        }
        if !expected.contains(&output) {
            return Err(choice_error("Choice output is not a declared label."));
        }
        Ok(Self {
            decision,
            question,
            output,
            probabilities: None,
        })
    }

    pub(crate) fn has_probabilities(&self) -> bool {
        self.probabilities.is_some()
    }

    /// Checks recovered results against the current compiled declaration.
    pub fn validate_declaration(
        &self,
        decision: &crate::compiler::ChoiceDecision,
    ) -> Result<(), ChoiceResultError> {
        if self.decision != decision.name || self.question != decision.question_id {
            return Err(choice_error(
                "Choice result does not match the declared question.",
            ));
        }
        let labels = decision
            .criteria
            .iter()
            .map(|(label, _)| label.clone())
            .collect::<Vec<_>>();
        match &self.probabilities {
            Some(probabilities) => Self::validate(
                self.decision.clone(),
                self.question.clone(),
                &labels,
                self.output.clone(),
                probabilities.clone(),
            ),
            None => Self::validate_label_only(
                self.decision.clone(),
                self.question.clone(),
                &labels,
                self.output.clone(),
            ),
        }
        .map(|_| ())
    }
}

/// A validation failure for a provider Choice result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceResultError {
    /// The stable diagnostic code for invalid decision evidence.
    pub code: DiagnosticCode,
    /// A short explanation of the invalid result.
    pub message: String,
}

/// A failure that prevents the whole action plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionPlanningError {
    /// The stable code for invalid context or exhausted shared limits.
    pub code: DiagnosticCode,
    /// A short explanation of why planning cannot complete.
    pub message: String,
}

/// The result of one configured status action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionOutcome {
    /// The zero-based position of this action in the authored list.
    pub index: usize,
    /// The action's applied, skipped, or failed result.
    pub outcome: ActionOutcomeKind,
}

/// The applied, skipped, or failed state for one status action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum ActionOutcomeKind {
    /// The condition passed and the action wrote this status result.
    Applied {
        /// The status written by this action.
        result: StatusResult,
        /// The evaluated explanatory text, when configured.
        message: Option<String>,
    },
    /// A condition evaluated to a false-like available value.
    SkippedConditionFalse,
    /// A condition depended on an unavailable context value.
    SkippedConditionUnavailable,
    /// A result or message depended on unavailable evidence.
    SkippedInputUnavailable,
    /// An ordinary expression or input error invalidated only this action.
    Failed(EvaluationError),
}

/// The complete ordered output of planning a status-action list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionPlan {
    /// The status after applying all valid writes, or the prior status if none wrote.
    pub current_status: Option<StatusResult>,
    /// One outcome for each action, in authored order.
    pub outcomes: Vec<ActionOutcome>,
}

/// Evaluates all actions independently against one frozen context and decision set.
///
/// Conditions read the same cloned context with validated Choice results exposed
/// under `decisions`. A skipped or failed action does not block later actions.
/// Valid status writes apply in order, so the last valid write sets the planned
/// status. This function does not persist results or send user-facing messages.
///
/// # Errors
/// Returns an error if the frozen root is not an object, it already contains a
/// `decisions` root, Choice results repeat a decision name, or a shared limit
/// stops evaluation. A shared limit error returns no partial plan.
pub fn plan_status_actions(
    actions: &[StatusAction],
    frozen_context: &Value,
    choices: &[ChoiceResult],
    prior_status: Option<StatusResult>,
) -> Result<ActionPlan, ActionPlanningError> {
    plan_status_actions_with_budget(
        actions,
        frozen_context,
        choices,
        prior_status,
        &mut EvaluationBudget::default(),
    )
}

/// Plans all status actions against the assessment's shared evaluation budget.
///
/// # Errors
/// Returns an invalid-context error or a whole-assessment budget error. An error
/// returns no action plan, so earlier tentative writes cannot be accepted.
pub fn plan_status_actions_with_budget(
    actions: &[StatusAction],
    frozen_context: &Value,
    choices: &[ChoiceResult],
    prior_status: Option<StatusResult>,
    budget: &mut EvaluationBudget,
) -> Result<ActionPlan, ActionPlanningError> {
    plan_status_actions_traced(actions, frozen_context, choices, prior_status, budget)
        .map(|(plan, _)| plan)
}

/// Plans actions and retains every evaluated source dependency.
///
/// # Errors
/// Returns an invalid-context error or a whole-assessment budget error.
pub fn plan_status_actions_traced(
    actions: &[StatusAction],
    frozen_context: &Value,
    choices: &[ChoiceResult],
    prior_status: Option<StatusResult>,
    budget: &mut EvaluationBudget,
) -> Result<(ActionPlan, EvaluationTrace), ActionPlanningError> {
    budget
        .step(1)
        .map_err(|error| planning_error(error.code, &error.message))?;
    let context = context_with_choices(frozen_context, choices)?;
    let mut current_status = prior_status;
    let mut outcomes = Vec::new();
    let mut trace = EvaluationTrace::default();
    for (index, action) in actions.iter().enumerate() {
        let result = plan_action(action, &context, budget, &mut trace);
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error)
                if error.code == DiagnosticCode::EvaluationLimitExceeded || budget.exhausted() =>
            {
                return Err(planning_error(
                    DiagnosticCode::EvaluationLimitExceeded,
                    &error.message,
                ));
            }
            Err(error) => ActionOutcomeKind::Failed(error),
        };
        if let ActionOutcomeKind::Applied { result, .. } = &outcome {
            current_status = Some(*result);
        }
        outcomes.push(ActionOutcome { index, outcome });
    }
    Ok((
        ActionPlan {
            current_status,
            outcomes,
        },
        trace,
    ))
}

fn plan_action(
    action: &StatusAction,
    context: &Value,
    budget: &mut EvaluationBudget,
    trace: &mut EvaluationTrace,
) -> Result<ActionOutcomeKind, EvaluationError> {
    budget.step(1)?;
    let mut bindings = EvaluationBindings::new();
    if let Some(condition) = &action.condition {
        let evaluated = evaluate_condition(condition, context, &bindings, budget, 1)?;
        trace.merge(&evaluated.trace);
        match evaluated.value {
            EvaluationValue::Unavailable => {
                return Ok(ActionOutcomeKind::SkippedConditionUnavailable);
            }
            EvaluationValue::Available(value) if !is_truthy(&value) => {
                return Ok(ActionOutcomeKind::SkippedConditionFalse);
            }
            EvaluationValue::Available(_) => {}
        }
    }
    let result = match &action.result {
        StatusResultTemplate::Literal(result) => *result,
        StatusResultTemplate::Expression(expression) => {
            let evaluated =
                evaluate_expression_with_budget(expression, context, &bindings, budget, 1)?;
            let evaluated = enforce_selected_membership(evaluated, context);
            trace.merge(&evaluated.trace);
            match evaluated.value {
                EvaluationValue::Unavailable => {
                    return Ok(ActionOutcomeKind::SkippedInputUnavailable);
                }
                EvaluationValue::Available(Value::String(result)) if result == "success" => {
                    StatusResult::Success
                }
                EvaluationValue::Available(Value::String(result)) if result == "failure" => {
                    StatusResult::Failure
                }
                _ => {
                    return Err(invalid_action_input(
                        "Status result must be exactly `success` or `failure`.",
                    ));
                }
            }
        }
    };
    let message = match &action.message {
        None => None,
        Some(template) => {
            let evaluated = prepare_template(template, context, &mut bindings, budget, 1)?;
            trace.merge(&evaluated.trace);
            match evaluated.value {
                EvaluationValue::Unavailable => {
                    return Ok(ActionOutcomeKind::SkippedInputUnavailable);
                }
                EvaluationValue::Available(Value::String(message))
                    if !message.trim().is_empty() =>
                {
                    Some(message)
                }
                _ => {
                    return Err(invalid_action_input(
                        "Status message must be a nonblank string.",
                    ));
                }
            }
        }
    };
    Ok(ActionOutcomeKind::Applied { result, message })
}

fn invalid_action_input(message: &str) -> EvaluationError {
    EvaluationError {
        code: DiagnosticCode::RuntimeInvalidContext,
        message: message.to_owned(),
    }
}

fn context_with_choices(
    frozen_context: &Value,
    choices: &[ChoiceResult],
) -> Result<Value, ActionPlanningError> {
    let Some(context) = frozen_context.as_object() else {
        return Err(ActionPlanningError {
            code: DiagnosticCode::RuntimeInvalidContext,
            message: "Frozen expression context must be a JSON object.".to_owned(),
        });
    };
    if context.contains_key("decisions") {
        return Err(ActionPlanningError {
            code: DiagnosticCode::RuntimeInvalidContext,
            message: "Frozen context must not contain a `decisions` root.".to_owned(),
        });
    }
    let mut root = context.clone();
    if choices.is_empty() {
        return Ok(Value::Object(root));
    }
    let mut decisions = Map::new();
    for choice in choices {
        if decisions.contains_key(&choice.decision) {
            return Err(planning_error(
                DiagnosticCode::RuntimeInvalidDecision,
                "Choice results must have unique decision names.",
            ));
        }
        let mut question_details = Map::new();
        if let Some(probabilities) = &choice.probabilities {
            let probabilities = probabilities
                .iter()
                .map(|(label, probability)| {
                    let number = Number::from_f64(*probability)
                        .expect("validated Choice probabilities are finite");
                    (label.clone(), Value::Number(number))
                })
                .collect::<Map<_, _>>();
            question_details.insert("probabilities".to_owned(), Value::Object(probabilities));
        }
        let mut details = Map::new();
        details.insert(choice.question.clone(), Value::Object(question_details));
        let mut outputs = Map::new();
        outputs.insert(
            choice.question.clone(),
            Value::String(choice.output.clone()),
        );
        decisions.insert(
            choice.decision.clone(),
            Value::Object(Map::from_iter([
                ("outputs".to_owned(), Value::Object(outputs)),
                ("details".to_owned(), Value::Object(details)),
            ])),
        );
    }
    root.insert("decisions".to_owned(), Value::Object(decisions));
    Ok(Value::Object(root))
}

fn choice_error(message: &str) -> ChoiceResultError {
    ChoiceResultError {
        code: DiagnosticCode::RuntimeInvalidDecision,
        message: message.to_owned(),
    }
}

fn planning_error(code: DiagnosticCode, message: &str) -> ActionPlanningError {
    ActionPlanningError {
        code,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod probability_tests {
    use super::*;
    use crate::expression_eval::evaluate_expression;
    use serde_json::json;

    fn choice(probabilities: BTreeMap<String, f64>) -> Result<ChoiceResult, ChoiceResultError> {
        ChoiceResult::validate(
            "review",
            "verdict",
            &["stuck".into(), "clear".into(), "uncertain".into()],
            "stuck",
            probabilities,
        )
    }

    #[test]
    fn label_only_context_keeps_probabilities_and_confidence_unavailable() {
        let choice = ChoiceResult::validate_label_only(
            "review",
            "verdict",
            &["stuck".into(), "clear".into()],
            "stuck",
        )
        .unwrap();
        let context = context_with_choices(&json!({}), &[choice]).unwrap();
        assert_eq!(
            context["decisions"]["review"]["outputs"]["verdict"],
            "stuck"
        );
        let details = context["decisions"]["review"]["details"]["verdict"]
            .as_object()
            .unwrap();
        assert!(!details.contains_key("probabilities"));
        assert!(!details.contains_key("confidence"));
        for path in [
            "decisions.review.details.verdict.probabilities",
            "decisions.review.details.verdict.probabilities.stuck",
            "decisions.review.details.verdict.confidence",
        ] {
            assert_eq!(
                evaluate_expression(
                    &crate::expression::parse_expression(path).unwrap(),
                    &context
                )
                .unwrap(),
                EvaluationValue::Unavailable,
            );
        }
    }

    #[test]
    fn rounded_probability_sums_are_accepted_without_changing_action_thresholds() {
        let check = crate::compiler::compile(include_str!(
            "../../../docs/burn-check/examples/failed-retry-loop.yml"
        ))
        .unwrap();
        for (probability, expected_status) in [
            (0.949, Some(StatusResult::Success)),
            (0.95, Some(StatusResult::Failure)),
        ] {
            let probabilities = BTreeMap::from([
                ("stuck".into(), probability),
                ("clear".into(), 0.03),
                (
                    "uncertain".into(),
                    if probability == 0.95 { 0.01 } else { 0.011 },
                ),
            ]);
            let answer =
                choice(probabilities.clone()).expect("a 0.99 sum meets the provider contract");
            assert_eq!(answer.probabilities, Some(probabilities));
            let plan = plan_status_actions(&check.actions, &json!({}), &[answer], None).unwrap();
            assert_eq!(plan.current_status, expected_status);
        }
        let above_one = BTreeMap::from([
            ("stuck".into(), 0.96),
            ("clear".into(), 0.03),
            ("uncertain".into(), 0.02),
        ]);
        assert_eq!(
            choice(above_one.clone()).unwrap().probabilities,
            Some(above_one)
        );
    }

    #[test]
    fn sums_outside_provider_tolerance_and_invalid_individual_values_are_rejected() {
        for probabilities in [
            BTreeMap::from([
                ("stuck".into(), 0.90),
                ("clear".into(), 0.04),
                ("uncertain".into(), 0.01),
            ]),
            BTreeMap::from([
                ("stuck".into(), 0.99),
                ("clear".into(), 0.03),
                ("uncertain".into(), 0.01),
            ]),
            BTreeMap::from([
                ("stuck".into(), f64::NAN),
                ("clear".into(), 0.0),
                ("uncertain".into(), 0.0),
            ]),
            BTreeMap::from([
                ("stuck".into(), 1.01),
                ("clear".into(), 0.0),
                ("uncertain".into(), 0.0),
            ]),
            BTreeMap::from([
                ("stuck".into(), 1.0),
                ("clear".into(), -0.01),
                ("uncertain".into(), 0.01),
            ]),
        ] {
            assert!(choice(probabilities).is_err());
        }
    }
}
