use crate::expression::{AccessSegment, BinaryOperator, Expression, PathSegment};

use super::{ChoiceDecision, ChoiceRequirements};

#[derive(Clone, Copy, Default)]
struct DetailsRead {
    probabilities: bool,
    confidence: bool,
}

impl DetailsRead {
    fn merge(&mut self, other: Self) {
        self.probabilities |= other.probabilities;
        self.confidence |= other.confidence;
    }
}

enum SelectedValue {
    Other,
    Scalar(DetailsRead),
    Record(DecisionRecord),
    Array(Box<Self>),
    Alternatives(Vec<Self>),
}

#[derive(Clone, Copy)]
enum DecisionRecord {
    Decisions,
    Decision,
    Outputs,
    Details,
    ChoiceDetails,
    Probabilities,
}

struct Analysis {
    value: SelectedValue,
    reads: DetailsRead,
}

pub(super) fn inspect(
    expression: &Expression,
    decision: &ChoiceDecision,
    requirements: &mut ChoiceRequirements,
) {
    let analysis = analyze(expression, decision);
    let mut reads = analysis.reads;
    reads.merge(analysis.value.details_read());
    requirements.probabilities |= reads.probabilities;
    requirements.confidence |= reads.confidence;
}

fn analyze(expression: &Expression, decision: &ChoiceDecision) -> Analysis {
    let mut reads = DetailsRead::default();
    let value = match expression {
        Expression::Literal(_) => SelectedValue::Other,
        Expression::Path(path) => {
            let mut value = if matches!(path.first(), Some(PathSegment::Property(root)) if root == "decisions")
            {
                SelectedValue::Record(DecisionRecord::Decisions)
            } else {
                SelectedValue::Other
            };
            for segment in path.iter().skip(1) {
                value = match segment {
                    PathSegment::Property(key) => value.property(key, decision),
                    PathSegment::Index(_) => value.index(),
                };
            }
            value
        }
        Expression::Access { base, segments } => {
            let base = analyze(base, decision);
            reads.merge(base.reads);
            select(base.value, segments, decision, &mut reads)
        }
        Expression::Call { arguments, .. } => {
            for argument in arguments {
                let argument = analyze(argument, decision);
                reads.merge(argument.reads);
                reads.merge(argument.value.details_read());
            }
            SelectedValue::Other
        }
        Expression::Not(value) => {
            let value = analyze(value, decision);
            reads.merge(value.reads);
            reads.merge(value.value.details_read());
            SelectedValue::Other
        }
        Expression::Binary {
            operator,
            left,
            right,
        } => {
            let left = analyze(left, decision);
            let right = analyze(right, decision);
            reads.merge(left.reads);
            reads.merge(right.reads);
            reads.merge(left.value.details_read());
            match operator {
                BinaryOperator::And | BinaryOperator::Or => {
                    SelectedValue::Alternatives(vec![left.value, right.value])
                }
                _ => {
                    reads.merge(right.value.details_read());
                    SelectedValue::Other
                }
            }
        }
    };
    Analysis { value, reads }
}

fn select(
    mut value: SelectedValue,
    segments: &[AccessSegment],
    decision: &ChoiceDecision,
    reads: &mut DetailsRead,
) -> SelectedValue {
    for (position, segment) in segments.iter().enumerate() {
        value = match segment {
            AccessSegment::Property(key) => value.property(key, decision),
            AccessSegment::Index(_) => value.index(),
            AccessSegment::Dynamic(selector) => {
                let selector = analyze(selector, decision);
                reads.merge(selector.reads);
                reads.merge(selector.value.details_read());
                value.dynamic()
            }
            AccessSegment::Projection => {
                let item = value.projection();
                let selected = select(item, &segments[position + 1..], decision, reads);
                return SelectedValue::Array(Box::new(selected));
            }
        };
    }
    value
}

impl SelectedValue {
    fn details_read(&self) -> DetailsRead {
        match self {
            Self::Other | Self::Record(DecisionRecord::Outputs) => DetailsRead::default(),
            Self::Scalar(reads) => *reads,
            Self::Record(_) => DetailsRead {
                probabilities: true,
                confidence: false,
            },
            Self::Array(item) => item.details_read(),
            Self::Alternatives(items) => {
                let mut reads = DetailsRead::default();
                for item in items {
                    reads.merge(item.details_read());
                }
                reads
            }
        }
    }

    fn property(self, key: &str, decision: &ChoiceDecision) -> Self {
        use DecisionRecord::*;
        match self {
            Self::Record(record) => match (record, key) {
                (Decisions, name) if name == decision.name => Self::Record(Decision),
                (Decision, "outputs") => Self::Record(Outputs),
                (Decision, "details") => Self::Record(Details),
                (Outputs, name) if name == decision.question_id => Self::Other,
                (Details, name) if name == decision.question_id => Self::Record(ChoiceDetails),
                (ChoiceDetails, "probabilities") => Self::Record(Probabilities),
                (ChoiceDetails, "confidence") => Self::Scalar(DetailsRead {
                    probabilities: false,
                    confidence: true,
                }),
                (Probabilities, label)
                    if decision.criteria.iter().any(|(name, _)| name == label) =>
                {
                    Self::Scalar(DetailsRead {
                        probabilities: true,
                        confidence: false,
                    })
                }
                _ => Self::Other,
            },
            Self::Alternatives(items) => Self::Alternatives(
                items
                    .into_iter()
                    .map(|item| item.property(key, decision))
                    .collect(),
            ),
            Self::Array(item) if key == "length" => Self::Scalar(item.details_read()),
            _ => Self::Other,
        }
    }

    fn index(self) -> Self {
        match self {
            Self::Array(item) => *item,
            Self::Alternatives(items) => {
                Self::Alternatives(items.into_iter().map(Self::index).collect())
            }
            _ => Self::Other,
        }
    }

    fn dynamic(self) -> Self {
        match self {
            Self::Record(DecisionRecord::Probabilities) => Self::Scalar(self.details_read()),
            Self::Alternatives(items) => {
                Self::Alternatives(items.into_iter().map(Self::dynamic).collect())
            }
            _ => self.index(),
        }
    }

    fn projection(self) -> Self {
        use DecisionRecord::*;
        match self {
            Self::Array(item) => *item,
            Self::Record(record) => match record {
                Decisions => Self::Record(Decision),
                Decision => Self::Alternatives(vec![Self::Record(Outputs), Self::Record(Details)]),
                Outputs => Self::Other,
                Details => Self::Record(ChoiceDetails),
                ChoiceDetails => Self::Record(Probabilities),
                Probabilities => Self::Scalar(self.details_read()),
            },
            Self::Alternatives(items) => {
                Self::Alternatives(items.into_iter().map(Self::projection).collect())
            }
            _ => Self::Other,
        }
    }
}
