//! Pure per-branch tool-result filtering and rolling candidate selection.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::compiler::{CompiledCheck, RollingWindow, ToolCategory, ToolResultStatus};
use crate::definition_lifecycle::EffectiveActivation;
use crate::domain::{
    BranchId, EvidenceDependency, EvidenceVersion, FrozenCandidate, FrozenContext, Message,
    NativeOrderKey, Observation, ObservationKind, ObservationPosition,
};

/// Maximum number of finalized tool results retained for one branch.
pub const MAX_WINDOW_MESSAGES: usize = 40;

/// The selector state for one activation, separated by branch.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SelectionState {
    /// Branch-local rolling windows required to resume selection after restart.
    pub branches: BTreeMap<BranchId, BranchSelection>,
    /// Latest revision and native order for each source identity seen by this activation.
    pub seen_results: BTreeMap<(BranchId, String), SeenResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BranchSelection {
    /// Selected finalized results in the order accepted by the source adapter.
    pub results: VecDeque<SelectedMessage>,
    /// Whether the latest source snapshot proved complete membership for this branch.
    pub coverage_complete: bool,
}

impl Default for BranchSelection {
    fn default() -> Self {
        Self {
            results: VecDeque::new(),
            coverage_complete: true,
        }
    }
}

/// Latest committed source version for one logical result identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeenResult {
    /// Whether this identity has reached its first finalized-result admission.
    pub terminal_seen: bool,
    /// Highest producer revision observed for this identity.
    pub revision: u64,
    /// Latest exact evidence version, including same-revision payload changes.
    pub version: EvidenceVersion,
    /// Native order is stable across corrections.
    pub native_order: NativeOrderKey,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectedMessage {
    /// Source version used to detect corrections.
    pub version: EvidenceVersion,
    /// Source-native order retained across corrections.
    pub native_order: NativeOrderKey,
    /// Immutable normalized result value.
    pub message: Message,
    /// Whether the source proved the selected result complete.
    pub complete: bool,
}

/// Result of applying one normalized observation to a selector.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectionOutcome {
    /// The observation updated context or ordering but did not create a candidate.
    Updated,
    /// The result did not match the effective tool/status filters.
    Filtered,
    /// Matching evidence is incomplete and cannot be assessed.
    Incomplete,
    /// A complete candidate is frozen at this trigger position.
    Candidate(Box<FrozenCandidate>),
}

impl SelectionState {
    /// Applies one observation and freezes a candidate once the minimum is met.
    ///
    /// Historical tool results at or before activation bootstrap never enter a
    /// fresh rolling window. User messages may update frozen context on either
    /// side of that boundary. Corrections are expected to be rejected or
    /// invalidated by the caller before this append-only transition.
    pub fn apply(
        &mut self,
        activation: &EffectiveActivation,
        observation: &Observation,
    ) -> SelectionOutcome {
        let branch = self.branches.entry(observation.branch.clone()).or_default();
        if let ObservationKind::UserMessage(_) = observation.kind {
            return SelectionOutcome::Updated;
        }
        let ObservationKind::ToolResult {
            message,
            category,
            status,
        } = &observation.kind
        else {
            return SelectionOutcome::Updated;
        };
        if observation.position <= activation.bootstrap {
            return SelectionOutcome::Updated;
        }
        let Some(category) = parse_category(category) else {
            return SelectionOutcome::Filtered;
        };
        let Some(status) = parse_status(status) else {
            return SelectionOutcome::Filtered;
        };
        if !matches_filters(&activation.compiled, category, status) {
            return SelectionOutcome::Filtered;
        }

        let config = activation.compiled.trigger.window.unwrap_or(RollingWindow {
            size: 1,
            minimum: 1,
        });
        let size = usize::from(config.size).min(MAX_WINDOW_MESSAGES);
        branch.results.push_back(SelectedMessage {
            version: observation.version.clone(),
            native_order: observation.native_order.clone(),
            message: message.clone(),
            complete: observation.complete,
        });
        while branch.results.len() > size {
            branch.results.pop_front();
        }
        if branch.results.len() < usize::from(config.minimum) {
            return SelectionOutcome::Updated;
        }
        let mut dependencies = branch
            .results
            .iter()
            .map(|item| EvidenceDependency::Message {
                id: item.message.id.clone(),
                version: item.version.clone(),
            })
            .collect::<Vec<_>>();
        dependencies.extend(observation.context.dependencies.clone());
        dependencies.push(EvidenceDependency::MembershipThrough(observation.position));
        dependencies.sort();
        dependencies.dedup();
        let messages = branch
            .results
            .iter()
            .map(|item| item.message.clone())
            .collect::<Vec<_>>();
        let mut context = observation.context.clone();
        let Some(roots) = context.value.as_object_mut() else {
            return SelectionOutcome::Incomplete;
        };
        let ObservationKind::ToolResult { message, .. } = &observation.kind else {
            return SelectionOutcome::Incomplete;
        };
        roots.insert("message".to_owned(), message.value.clone());
        if let Some(session) = roots
            .get_mut("session")
            .and_then(serde_json::Value::as_object_mut)
        {
            session.insert(
                "source_coverage".to_owned(),
                serde_json::Value::String(
                    if branch.coverage_complete {
                        "complete"
                    } else {
                        "partial"
                    }
                    .to_owned(),
                ),
            );
        }
        roots.insert(
            "window".to_owned(),
            serde_json::json!({"messages": messages.iter().map(|message| &message.value).collect::<Vec<_>>()}),
        );
        let fingerprint = candidate_fingerprint(
            &activation.id,
            &observation.branch,
            observation.position,
            &messages,
            &context,
            &dependencies,
        );
        SelectionOutcome::Candidate(Box::new(FrozenCandidate {
            activation: activation.id.clone(),
            branch: observation.branch.clone(),
            trigger_position: observation.position,
            messages,
            context,
            dependencies,
            fingerprint,
        }))
    }

    /// Applies one published result without treating a correction as a new trigger.
    pub fn apply_published(
        &mut self,
        activation: &EffectiveActivation,
        identity: &str,
        revision: u64,
        observation: &Observation,
    ) -> SelectionOutcome {
        let key = (observation.branch.clone(), identity.to_owned());
        if let Some(previous) = self.seen_results.get(&key) {
            if revision < previous.revision
                || (revision == previous.revision && observation.version == previous.version)
            {
                return SelectionOutcome::Updated;
            }
            let order_changed = previous.native_order != observation.native_order;
            let first_terminal = !previous.terminal_seen
                && matches!(observation.kind, ObservationKind::ToolResult { .. });
            let terminal_seen = previous.terminal_seen || first_terminal;
            self.seen_results.insert(
                key,
                SeenResult {
                    terminal_seen,
                    revision,
                    version: observation.version.clone(),
                    native_order: observation.native_order.clone(),
                },
            );
            if first_terminal {
                return self.apply(activation, observation);
            }
            let branch = self.branches.entry(observation.branch.clone()).or_default();
            if order_changed {
                branch.results.clear();
                branch.coverage_complete = false;
                return SelectionOutcome::Incomplete;
            }
            if let Some(selected) = branch
                .results
                .iter_mut()
                .find(|selected| selected.message.id == identity)
            {
                let ObservationKind::ToolResult {
                    message,
                    category,
                    status,
                } = &observation.kind
                else {
                    branch.results.clear();
                    branch.coverage_complete = false;
                    return SelectionOutcome::Incomplete;
                };
                let matches = parse_category(category)
                    .zip(parse_status(status))
                    .is_some_and(|(category, status)| {
                        matches_filters(&activation.compiled, category, status)
                    });
                if !matches {
                    branch
                        .results
                        .retain(|selected| selected.message.id != identity);
                    return SelectionOutcome::Updated;
                }
                selected.version = observation.version.clone();
                selected.message = message.clone();
                selected.complete = observation.complete;
            } else if matches!(&observation.kind, ObservationKind::ToolResult { category, status, .. }
                if parse_category(category).zip(parse_status(status)).is_some_and(|(category, status)| matches_filters(&activation.compiled, category, status)))
            {
                // The retained window cannot prove the old position of a newly matching result.
                branch.results.clear();
                branch.coverage_complete = false;
            }
            return SelectionOutcome::Updated;
        }

        self.seen_results.insert(
            key,
            SeenResult {
                terminal_seen: matches!(observation.kind, ObservationKind::ToolResult { .. }),
                revision,
                version: observation.version.clone(),
                native_order: observation.native_order.clone(),
            },
        );
        self.apply(activation, observation)
    }

    /// Removes a result from the rolling window without creating a trigger.
    pub fn remove_result(&mut self, branch: &BranchId, identity: &str) {
        if let Some(selection) = self.branches.get_mut(branch) {
            selection
                .results
                .retain(|selected| selected.message.id != identity);
        }
    }

    /// Records incomplete membership without discarding usable published results.
    pub fn mark_coverage_gap(&mut self, branch: &BranchId) {
        let selection = self.branches.entry(branch.clone()).or_default();
        selection.coverage_complete = false;
    }

    /// Starts a fresh rolling window after the source proves coverage is complete.
    pub fn restore_coverage(&mut self, branch: &BranchId) {
        let selection = self.branches.entry(branch.clone()).or_default();
        selection.results.clear();
        selection.coverage_complete = true;
    }
}

fn matches_filters(
    check: &CompiledCheck,
    category: ToolCategory,
    status: ToolResultStatus,
) -> bool {
    check
        .trigger
        .tools
        .as_ref()
        .is_none_or(|tools| tools.contains(&category))
        && check
            .trigger
            .statuses
            .as_ref()
            .is_none_or(|statuses| statuses.contains(&status))
}

fn parse_category(value: &str) -> Option<ToolCategory> {
    Some(match value {
        "shell" => ToolCategory::Shell,
        "read" => ToolCategory::Read,
        "search" => ToolCategory::Search,
        "edit" => ToolCategory::Edit,
        "write" => ToolCategory::Write,
        "delegate" => ToolCategory::Delegate,
        "skill" => ToolCategory::Skill,
        "mcp" => ToolCategory::Mcp,
        "other" => ToolCategory::Other,
        _ => return None,
    })
}

fn parse_status(value: &str) -> Option<ToolResultStatus> {
    Some(match value {
        "completed" => ToolResultStatus::Completed,
        "error" => ToolResultStatus::Error,
        "cancelled" => ToolResultStatus::Cancelled,
        "unknown" => ToolResultStatus::Unknown,
        _ => return None,
    })
}

fn candidate_fingerprint(
    activation: &crate::domain::ActivationId,
    branch: &BranchId,
    trigger: ObservationPosition,
    messages: &[Message],
    context: &FrozenContext,
    dependencies: &[EvidenceDependency],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(activation).expect("activation identity serializes"));
    update_string(&mut hasher, &branch.0);
    hasher.update(trigger.0.to_be_bytes());
    hasher.update(
        serde_json::to_vec(&messages.iter().map(|item| &item.value).collect::<Vec<_>>())
            .expect("JSON message values serialize"),
    );
    hasher.update(serde_json::to_vec(&context.value).expect("JSON context serializes"));
    for dependency in dependencies {
        match dependency {
            EvidenceDependency::Message { id, version } => {
                hasher.update([0]);
                update_string(&mut hasher, id);
                update_string(&mut hasher, &version.0);
            }
            EvidenceDependency::MembershipThrough(position) => {
                hasher.update([1]);
                hasher.update(position.0.to_be_bytes());
            }
            EvidenceDependency::Context { id, version } => {
                hasher.update([2]);
                update_string(&mut hasher, id);
                update_string(&mut hasher, &version.0);
            }
        }
    }
    hasher.finalize().into()
}

fn update_string(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::compiler::compile;
    use crate::definition_lifecycle::EffectiveActivation;
    use crate::domain::{ActivationId, EvidenceVersion, Generation, NativeOrderKey};

    fn activation() -> EffectiveActivation {
        let source = "name: Retry\non:\n  tool_result:\n  window: {size: 3, min: 3}\ndecisions:\n  review:\n    model: typesafe/jev-1.13.0\n    input: {items: \"${{ window.messages }}\"}\n    output:\n      verdict:\n        type: choice\n        question: Is this useful?\n        criteria: {yes: Yes., no: No.}\nactions:\n  - type: status\n    input: {result: failure}\n";
        EffectiveActivation {
            id: ActivationId {
                scope: Default::default(),
                definition_id: "retry".to_owned(),
                generation: Generation(1),
            },
            revision: [1; 32],
            scope: crate::discovery::DefinitionScope::Global,
            source_paths: vec![],
            project_root: None,
            compiled: std::sync::Arc::new(compile(source).unwrap()),
            bootstrap: ObservationPosition(5),
            bootstrap_publication: Default::default(),
        }
    }

    fn result(position: u64) -> Observation {
        Observation {
            position: ObservationPosition(position),
            branch: BranchId("main-session".to_owned()),
            native_order: NativeOrderKey(format!("native-{position}")),
            version: EvidenceVersion(format!("v{position}")),
            kind: ObservationKind::ToolResult {
                message: Message {
                    id: format!("message-{position}"),
                    value: json!({"id": format!("message-{position}"), "tool": {"result": {"status": "error"}}}),
                },
                category: "shell".to_owned(),
                status: "error".to_owned(),
            },
            context: FrozenContext {
                value: json!({"session": {"requests": []}}),
                dependencies: vec![],
            },
            complete: true,
        }
    }

    #[test]
    fn bootstrap_excludes_old_results_and_window_rolls_at_its_limit() {
        let activation = activation();
        let mut state = SelectionState::default();
        assert_eq!(
            state.apply(&activation, &result(5)),
            SelectionOutcome::Updated
        );
        assert_eq!(
            state.apply(&activation, &result(6)),
            SelectionOutcome::Updated
        );
        assert_eq!(
            state.apply(&activation, &result(7)),
            SelectionOutcome::Updated
        );
        let SelectionOutcome::Candidate(first) = state.apply(&activation, &result(8)) else {
            panic!("minimum-sized window must create a candidate");
        };
        assert_eq!(
            first
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["message-6", "message-7", "message-8"]
        );
        let SelectionOutcome::Candidate(rolled) = state.apply(&activation, &result(9)) else {
            panic!("full rolling window must continue creating candidates");
        };
        assert_eq!(
            rolled
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["message-7", "message-8", "message-9"]
        );
        assert_ne!(first.fingerprint, rolled.fingerprint);
    }

    #[test]
    fn result_with_missing_source_fields_still_creates_a_partial_candidate() {
        let activation = activation();
        let mut state = SelectionState::default();
        for position in 6..=8 {
            let mut observation = result(position);
            if position == 8 {
                observation.complete = false;
                let ObservationKind::ToolResult { message, .. } = &mut observation.kind else {
                    unreachable!();
                };
                message.value["quality"] = json!({"arguments_available": false});
            }
            let outcome =
                state.apply_published(&activation, &format!("message-{position}"), 1, &observation);
            if position == 8 {
                let SelectionOutcome::Candidate(candidate) = outcome else {
                    panic!("missing optional source fields must not block review");
                };
                assert_eq!(
                    candidate.messages[2].value["quality"]["arguments_available"],
                    false
                );
            } else {
                assert_eq!(outcome, SelectionOutcome::Updated);
            }
        }
    }

    #[test]
    fn correction_updates_the_selected_identity_without_creating_a_trigger() {
        let activation = activation();
        let mut state = SelectionState::default();
        for position in 6..=8 {
            let observation = result(position);
            let outcome =
                state.apply_published(&activation, &format!("message-{position}"), 1, &observation);
            assert_eq!(
                matches!(outcome, SelectionOutcome::Candidate(_)),
                position == 8
            );
        }

        let mut correction = result(9);
        correction.native_order = NativeOrderKey("native-8".to_owned());
        correction.version = EvidenceVersion("v8-corrected".to_owned());
        let ObservationKind::ToolResult { message, .. } = &mut correction.kind else {
            unreachable!();
        };
        message.id = "message-8".to_owned();
        message.value = json!({"id": "message-8", "revision": 2});
        assert_eq!(
            state.apply_published(&activation, "message-8", 2, &correction),
            SelectionOutcome::Updated
        );

        let next = result(10);
        let SelectionOutcome::Candidate(candidate) =
            state.apply_published(&activation, "message-10", 1, &next)
        else {
            panic!("a later new result must trigger from the corrected window");
        };
        assert_eq!(candidate.messages[1].value["revision"], 2);
        assert!(
            candidate
                .dependencies
                .contains(&EvidenceDependency::Message {
                    id: "message-8".to_owned(),
                    version: EvidenceVersion("v8-corrected".to_owned()),
                })
        );
    }

    #[test]
    fn first_terminal_after_a_non_final_update_triggers_once() {
        let activation = activation();
        let mut state = SelectionState::default();
        for position in 6..=7 {
            state.apply_published(
                &activation,
                &format!("message-{position}"),
                1,
                &result(position),
            );
        }
        let mut pending = result(8);
        pending.kind = ObservationKind::Other;
        assert_eq!(
            state.apply_published(&activation, "message-8", 1, &pending),
            SelectionOutcome::Updated
        );
        let mut finalized = result(9);
        let ObservationKind::ToolResult { message, .. } = &mut finalized.kind else {
            unreachable!()
        };
        message.id = "message-8".into();
        assert!(matches!(
            state.apply_published(&activation, "message-8", 2, &finalized),
            SelectionOutcome::Candidate(_)
        ));
        finalized.position = ObservationPosition(10);
        finalized.version = EvidenceVersion("corrected".into());
        assert_eq!(
            state.apply_published(&activation, "message-8", 3, &finalized),
            SelectionOutcome::Updated
        );
        assert_eq!(state.branches[&finalized.branch].results.len(), 3);
    }

    #[test]
    fn same_revision_payload_change_updates_selected_evidence() {
        let activation = activation();
        let mut state = SelectionState::default();
        for position in 6..=8 {
            let observation = result(position);
            state.apply_published(&activation, &format!("message-{position}"), 1, &observation);
        }
        let mut changed = result(9);
        changed.native_order = NativeOrderKey("native-8".to_owned());
        changed.version = EvidenceVersion("same-revision-payload-v2".to_owned());
        let ObservationKind::ToolResult { message, .. } = &mut changed.kind else {
            unreachable!();
        };
        message.id = "message-8".to_owned();
        message.value = json!({"id": "message-8", "changed": true});
        assert_eq!(
            state.apply_published(&activation, "message-8", 1, &changed),
            SelectionOutcome::Updated
        );
        let next = result(10);
        let SelectionOutcome::Candidate(candidate) =
            state.apply_published(&activation, "message-10", 1, &next)
        else {
            panic!("new result must select the changed same-revision evidence");
        };
        assert_eq!(candidate.messages[1].value["changed"], true);
        assert!(
            candidate
                .dependencies
                .contains(&EvidenceDependency::Message {
                    id: "message-8".to_owned(),
                    version: EvidenceVersion("same-revision-payload-v2".to_owned()),
                })
        );
    }

    #[test]
    fn coverage_gap_keeps_usable_results_and_marks_candidates_partial() {
        let activation = activation();
        let mut state = SelectionState::default();
        let first = result(6);
        assert_eq!(
            state.apply_published(&activation, "message-6", 1, &first),
            SelectionOutcome::Updated
        );
        let branch = BranchId("main-session".to_owned());
        state.mark_coverage_gap(&branch);
        let during_gap = result(7);
        assert_eq!(
            state.apply_published(&activation, "message-7", 1, &during_gap),
            SelectionOutcome::Updated
        );
        let next = result(8);
        let SelectionOutcome::Candidate(candidate) =
            state.apply_published(&activation, "message-8", 1, &next)
        else {
            panic!("partial evidence must remain assessable");
        };
        assert_eq!(
            candidate.context.value["session"]["source_coverage"],
            "partial"
        );

        state.restore_coverage(&branch);
        for position in 9..=10 {
            let observation = result(position);
            assert_eq!(
                state
                    .apply_published(&activation, &format!("message-{position}"), 1, &observation,),
                SelectionOutcome::Updated
            );
        }
        let third = result(11);
        assert!(matches!(
            state.apply_published(&activation, "message-11", 1, &third),
            SelectionOutcome::Candidate(_)
        ));
    }
}
