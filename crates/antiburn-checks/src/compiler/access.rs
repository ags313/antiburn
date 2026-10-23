use crate::diagnostics::DiagnosticCode;
use crate::expression::{
    AccessSegment, BinaryOperator, Expression, ExpressionFunction, ExpressionLiteral, PathSegment,
};

use super::{ExpressionContext, Validator, reference_error, unsupported_reference};

type AccessResult = Result<ValueShape, (DiagnosticCode, String)>;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ValueShape {
    Scalar,
    Open,
    Array(Box<Self>),
    Record(Record),
    Alternatives(Vec<Self>),
}

impl ValueShape {
    pub(super) fn map_item(self) -> Option<Self> {
        match self {
            Self::Array(item) => Some(*item),
            Self::Open => Some(Self::Open),
            Self::Alternatives(items) => {
                let mut items: Vec<_> = items.into_iter().filter_map(Self::map_item).collect();
                match items.len() {
                    0 => None,
                    1 => items.pop(),
                    _ => Some(Self::Alternatives(items)),
                }
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Record {
    Session,
    Request,
    Window,
    Message,
    Quality,
    Tool,
    Result,
    Outcome,
    Agent,
    Repo,
    Decisions,
    Decision,
    Outputs,
    Details,
    ChoiceDetails,
    Probabilities,
}

impl Validator {
    pub(super) fn resolve_expression(
        &self,
        expression: &Expression,
        context: ExpressionContext,
    ) -> AccessResult {
        match expression {
            Expression::Literal(_) => Ok(ValueShape::Scalar),
            Expression::Path(path) => {
                let Some((PathSegment::Property(root), rest)) = path.split_first() else {
                    return Err(reference_error(
                        "Expression paths must start with a context root.",
                    ));
                };
                let mut shape = self.root_shape(root, context)?;
                for segment in rest {
                    shape = match segment {
                        PathSegment::Property(key) => self.property_shape(shape, key)?,
                        PathSegment::Index(_) => self.index_shape(shape)?,
                    };
                }
                Ok(shape)
            }
            Expression::Access { base, segments } => {
                let shape = self.resolve_expression(base, context)?;
                self.access_shape(shape, segments, context)
            }
            Expression::Call {
                function,
                arguments,
            } => {
                for argument in arguments {
                    self.resolve_expression(argument, context)?;
                }
                Ok(match function {
                    ExpressionFunction::FromJSON => ValueShape::Open,
                    _ => ValueShape::Scalar,
                })
            }
            Expression::Not(value) => {
                self.resolve_expression(value, context)?;
                Ok(ValueShape::Scalar)
            }
            Expression::Binary {
                operator,
                left,
                right,
            } => {
                let known_truth = match left.as_ref() {
                    Expression::Literal(ExpressionLiteral::Null) => Some(false),
                    Expression::Literal(ExpressionLiteral::Boolean(value)) => Some(*value),
                    Expression::Literal(ExpressionLiteral::String(value)) => {
                        Some(!value.is_empty())
                    }
                    Expression::Literal(ExpressionLiteral::Number(value)) => {
                        value.as_f64().map(|value| value != 0.0)
                    }
                    _ => None,
                };
                let left = self.resolve_expression(left, context)?;
                let right = self.resolve_expression(right, context)?;
                if let Some(truth) = known_truth
                    && matches!(operator, BinaryOperator::And | BinaryOperator::Or)
                {
                    return Ok(if (*operator == BinaryOperator::Or) == truth {
                        left
                    } else {
                        right
                    });
                }
                if matches!(operator, BinaryOperator::And | BinaryOperator::Or)
                    && matches!(left, ValueShape::Array(_) | ValueShape::Record(_))
                {
                    return Ok(if *operator == BinaryOperator::Or {
                        left
                    } else {
                        right
                    });
                }
                Ok(match operator {
                    BinaryOperator::And | BinaryOperator::Or if left != right => {
                        ValueShape::Alternatives(vec![left, right])
                    }
                    BinaryOperator::And | BinaryOperator::Or => left,
                    _ => ValueShape::Scalar,
                })
            }
        }
    }

    fn root_shape(&self, root: &str, context: ExpressionContext) -> AccessResult {
        if context == ExpressionContext::DecisionInput
            && let Some((_, shape)) = self.aliases.iter().rev().find(|(name, _)| name == root)
        {
            return Ok(shape.clone());
        }
        let record = match root {
            "session" => Record::Session,
            "message" => Record::Message,
            "agent" => Record::Agent,
            "repo" => Record::Repo,
            "window" if context == ExpressionContext::TopLevel => {
                return Err(unsupported_reference(
                    "The `window` root is unavailable to top-level conditions.",
                ));
            }
            "window" if !self.has_window => {
                return Err(reference_error("The `window` root requires `on.window`."));
            }
            "window" => Record::Window,
            "decisions" if context != ExpressionContext::ActionCondition => {
                return Err(unsupported_reference(
                    "The `decisions` root is available only to actions.",
                ));
            }
            "decisions" if self.decisions.is_none() => {
                return Err(reference_error(
                    "Decision results require a compiled decision.",
                ));
            }
            "decisions" => Record::Decisions,
            "global" | "turn" => {
                return Err(unsupported_reference(&format!(
                    "The `{root}` root is not supported yet."
                )));
            }
            _ => return Err(reference_error(&format!("Unknown context root `{root}`."))),
        };
        Ok(ValueShape::Record(record))
    }

    fn access_shape(
        &self,
        mut shape: ValueShape,
        segments: &[AccessSegment],
        context: ExpressionContext,
    ) -> AccessResult {
        for (index, segment) in segments.iter().enumerate() {
            shape = match segment {
                AccessSegment::Property(key) => self.property_shape(shape, key)?,
                AccessSegment::Index(_) => self.index_shape(shape)?,
                AccessSegment::Dynamic(selector) => {
                    self.resolve_expression(selector, context)?;
                    match selector.as_ref() {
                        Expression::Literal(ExpressionLiteral::String(key)) => {
                            self.property_shape(shape, key)?
                        }
                        Expression::Literal(ExpressionLiteral::Number(_)) => {
                            self.index_shape(shape)?
                        }
                        _ => match shape {
                            ValueShape::Open => ValueShape::Open,
                            ValueShape::Array(item) => *item,
                            ValueShape::Record(Record::Probabilities) => ValueShape::Scalar,
                            _ => {
                                return Err(reference_error(
                                    "Dynamic keys cannot select unknown fields on closed normalized records.",
                                ));
                            }
                        },
                    }
                }
                AccessSegment::Projection => {
                    let item = self.projection_shape(shape)?;
                    let selected = self.access_shape(item, &segments[index + 1..], context)?;
                    return Ok(ValueShape::Array(Box::new(selected)));
                }
            };
        }
        Ok(shape)
    }

    fn property_shape(&self, shape: ValueShape, key: &str) -> AccessResult {
        match shape {
            ValueShape::Open => Ok(ValueShape::Open),
            ValueShape::Array(_) if key == "length" => Ok(ValueShape::Scalar),
            ValueShape::Alternatives(items) => {
                let shapes = items
                    .into_iter()
                    .map(|item| self.property_shape(item, key))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ValueShape::Alternatives(shapes))
            }
            ValueShape::Record(record) => self.record_field(record, key),
            _ => Err(reference_error(
                "The selected value has no such child field.",
            )),
        }
    }

    fn index_shape(&self, shape: ValueShape) -> AccessResult {
        match shape {
            ValueShape::Open => Ok(ValueShape::Open),
            ValueShape::Array(item) => Ok(*item),
            ValueShape::Alternatives(items) => Ok(ValueShape::Alternatives(
                items
                    .into_iter()
                    .map(|item| self.index_shape(item))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            _ => Err(reference_error(
                "A normalized object or scalar cannot use an array index.",
            )),
        }
    }

    fn projection_shape(&self, shape: ValueShape) -> AccessResult {
        match shape {
            ValueShape::Open => Ok(ValueShape::Open),
            ValueShape::Array(item) => Ok(*item),
            ValueShape::Record(record) => {
                let fields = self.record_fields(record);
                let shapes = fields
                    .iter()
                    .map(|key| self.record_field(record, key))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ValueShape::Alternatives(shapes))
            }
            ValueShape::Alternatives(items) => Ok(ValueShape::Alternatives(
                items
                    .into_iter()
                    .map(|item| self.projection_shape(item))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            ValueShape::Scalar => Err(reference_error("Projection requires an array or object.")),
        }
    }

    fn record_field(&self, record: Record, key: &str) -> AccessResult {
        use Record::*;
        use ValueShape::{Array, Open, Record as Object, Scalar};
        let result = match (record, key) {
            (Session, "id" | "requests_complete" | "source_coverage") => Scalar,
            (Session, "requests") => Array(Box::new(Object(Request))),
            (Window, "messages") => Array(Box::new(Object(Message))),
            (Request, "id" | "source" | "kind" | "text" | "in_reply_to") => Scalar,
            (
                Message,
                "id" | "source" | "occurred_at" | "kind" | "role" | "model" | "fast_mode"
                | "content",
            ) => Scalar,
            (Message, "tool") => Object(Tool),
            (Message, "quality") => Object(Quality),
            (
                Quality,
                "source_complete"
                | "tool_name_available"
                | "category_available"
                | "arguments_available"
                | "result_content_available"
                | "request_history_complete"
                | "main_session",
            ) => Scalar,
            (Tool, "name" | "category" | "call_id" | "provider_executed") => Scalar,
            (Tool, "paths") => Array(Box::new(Scalar)),
            (Tool, "arguments") => Open,
            (Tool, "result") => Object(Result),
            (Result, "status" | "output_bytes") => Scalar,
            (Result, "content") => Open,
            (Result, "outcome") => Object(Outcome),
            (
                Outcome,
                "kind" | "state" | "exit" | "signal" | "timed_out" | "shell_id" | "error"
                | "session_id",
            ) => Scalar,
            (Agent, "name") | (Repo, "root") => Scalar,
            (Decisions, name)
                if self
                    .decisions
                    .as_ref()
                    .is_some_and(|symbols| symbols.name == name) =>
            {
                Object(Decision)
            }
            (Decision, "outputs") => Object(Outputs),
            (Decision, "details") => Object(Details),
            (Outputs, name)
                if self
                    .decisions
                    .as_ref()
                    .is_some_and(|symbols| symbols.question == name) =>
            {
                Scalar
            }
            (Details, name)
                if self
                    .decisions
                    .as_ref()
                    .is_some_and(|symbols| symbols.question == name) =>
            {
                Object(ChoiceDetails)
            }
            (ChoiceDetails, "probabilities") => Object(Probabilities),
            (Probabilities, label)
                if self
                    .decisions
                    .as_ref()
                    .is_some_and(|symbols| symbols.labels.contains(label)) =>
            {
                Scalar
            }
            (
                Session,
                "model"
                | "fast_mode"
                | "start_reason"
                | "idle_seconds"
                | "last_activity_at"
                | "last_activity_period",
            )
            | (Agent, "skills")
            | (Repo, "branch" | "instructions" | "skills")
            | (ChoiceDetails, "confidence") => {
                return Err(unsupported_reference(
                    "The selected context field or provider detail is not supported yet.",
                ));
            }
            _ => {
                return Err(reference_error(&format!(
                    "Unknown normalized field `{key}`."
                )));
            }
        };
        Ok(result)
    }

    fn record_fields(&self, record: Record) -> Vec<&str> {
        use Record::*;
        match record {
            Session => vec!["id", "requests_complete", "source_coverage", "requests"],
            Request => vec!["id", "source", "kind", "text", "in_reply_to"],
            Window => vec!["messages"],
            Message => vec![
                "id",
                "source",
                "occurred_at",
                "kind",
                "role",
                "model",
                "fast_mode",
                "content",
                "tool",
                "quality",
            ],
            Quality => vec![
                "source_complete",
                "tool_name_available",
                "category_available",
                "arguments_available",
                "result_content_available",
                "request_history_complete",
                "main_session",
            ],
            Tool => vec![
                "name",
                "category",
                "call_id",
                "provider_executed",
                "paths",
                "arguments",
                "result",
            ],
            Result => vec!["status", "output_bytes", "content", "outcome"],
            Outcome => vec![
                "kind",
                "state",
                "exit",
                "signal",
                "timed_out",
                "shell_id",
                "error",
                "session_id",
            ],
            Agent => vec!["name"],
            Repo => vec!["root"],
            Decisions => self
                .decisions
                .as_ref()
                .map_or_else(Vec::new, |symbols| vec![symbols.name.as_str()]),
            Decision => vec!["outputs", "details"],
            Outputs | Details => self
                .decisions
                .as_ref()
                .map_or_else(Vec::new, |symbols| vec![symbols.question.as_str()]),
            ChoiceDetails => vec!["probabilities"],
            Probabilities => self.decisions.as_ref().map_or_else(Vec::new, |symbols| {
                symbols.labels.iter().map(String::as_str).collect()
            }),
        }
    }
}
