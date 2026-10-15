use std::collections::BTreeMap;
use std::path::PathBuf;

use antiburn_checks::diagnostics::{Diagnostic, DiagnosticCode};
use antiburn_checks::domain::ActivationId;

use super::MAX_DEFINITIONS;

const MAX_DIAGNOSTICS_PER_FILE: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SafeDiagnostic {
    code: &'static str,
    line: Option<usize>,
    column: Option<usize>,
    location: String,
    message: String,
}

impl SafeDiagnostic {
    fn new(diagnostic: &Diagnostic) -> Self {
        use DiagnosticCode::*;
        let message = match diagnostic.code {
            YamlSyntax => "Malformed YAML.",
            YamlDirective => "YAML directives are not allowed.",
            YamlDocumentCount => "Expected one YAML document.",
            YamlRootType => "The YAML root must be an object.",
            YamlNestingLimit => "YAML exceeds the nesting limit.",
            YamlAnchor | YamlAlias => "YAML anchors and aliases are not allowed.",
            YamlUnsupportedTag => "This YAML tag is not supported.",
            YamlNonStringKey => "Object keys must be strings.",
            YamlDuplicateKey => "Object keys must be unique.",
            YamlMergeKey => "YAML merge keys are not allowed.",
            YamlNonFiniteNumber => "Numbers must be finite JSON numbers.",
            YamlInvalidScalar => "The scalar does not match its YAML type.",
            ConfigMissingField => "A required field is missing.",
            ConfigUnknownField => "This field is not supported.",
            ConfigInvalidType => "The field has an incorrect type.",
            ConfigInvalidValue => "The field has an invalid value.",
            CompileUnsupportedFeature => "Burn Checks do not support this feature in pre-alpha.",
            CompileInvalidExpression => "The expression syntax is invalid.",
            CompileUnknownReference => "The expression references an unavailable field.",
            CompileInvalidWindow => "Window size must be 1 through 40, with 1 <= min <= size.",
            CompileInvalidCooldown => "Cooldown must be an integer duration ending in s, m, or h.",
            CompileInvalidDecision => "The decision does not match the Choice contract.",
            CompileInvalidAction => "The action does not match the status-action contract.",
            DiscoveryInvalidId => "The filename must match [a-z][a-z0-9-]*.yml or .yaml.",
            DiscoveryDuplicateId => "Multiple files define the same check ID in this scope.",
            DiscoveryReadFailed => &diagnostic.message,
            _ => "The check file is invalid.",
        };
        Self {
            code: diagnostic.code.as_str(),
            line: diagnostic.span.map(|span| span.line),
            column: diagnostic.span.map(|span| span.column),
            location: safe_location(diagnostic.config_path.as_deref().unwrap_or("")),
            message: message.to_owned(),
        }
    }

    pub(super) fn display(diagnostic: &Diagnostic) -> String {
        let safe = Self::new(diagnostic);
        let position = match (safe.line, safe.column) {
            (Some(line), Some(column)) => format!(" at {line}:{column}"),
            _ => String::new(),
        };
        format!(
            "{}{} [{}]: {}",
            safe.code, position, safe.location, safe.message
        )
    }
}

fn safe_location(path: &str) -> String {
    const FIELDS: &[&str] = &[
        "name",
        "description",
        "on",
        "if",
        "decisions",
        "actions",
        "tool_result",
        "tools",
        "status",
        "models",
        "fast_mode",
        "paths",
        "paths-ignore",
        "window",
        "size",
        "min",
        "cooldown",
        "model",
        "input",
        "output",
        "type",
        "question",
        "criteria",
        "result",
        "message",
    ];
    if path.is_empty() {
        return "root".into();
    }
    path.split('.')
        .take(16)
        .map(|segment| {
            let name = segment.split('[').next().unwrap_or(segment);
            if FIELDS.contains(&name)
                && segment.len() <= 32
                && segment[name.len()..]
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || ch == '[' || ch == ']')
            {
                segment
            } else {
                "<name>"
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

#[derive(Default)]
pub(super) struct DiagnosticLog {
    previous: BTreeMap<PathBuf, Vec<SafeDiagnostic>>,
    conditions: BTreeMap<ActivationId, ConditionState>,
}

#[derive(Default, PartialEq, Eq)]
struct ConditionState {
    condition_false: bool,
    condition_unavailable: bool,
    condition_failed: bool,
}

pub(super) type DiagnosticBatch = BTreeMap<PathBuf, Vec<SafeDiagnostic>>;

pub(super) fn collect(batch: &mut DiagnosticBatch, diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        if matches!(
            diagnostic.code,
            DiagnosticCode::DiscoveryPaused | DiagnosticCode::DiscoveryShadowed
        ) {
            continue;
        }
        let Some(path) = &diagnostic.source_path else {
            continue;
        };
        if batch.len() >= MAX_DEFINITIONS * 2 && !batch.contains_key(path) {
            continue;
        }
        let errors = batch.entry(path.clone()).or_default();
        let safe = SafeDiagnostic::new(diagnostic);
        if errors.len() < MAX_DIAGNOSTICS_PER_FILE && !errors.contains(&safe) {
            errors.push(safe);
        }
    }
}

impl DiagnosticLog {
    pub(super) fn publish_conditions(
        &mut self,
        activation: &antiburn_checks::definition_lifecycle::EffectiveActivation,
        outcome: &antiburn_checks::engine::BatchOutcome,
    ) {
        use antiburn_checks::preparation::ConditionOutcome;
        let mut current = ConditionState::default();
        for (_, condition) in &outcome.condition_outcomes {
            match condition {
                ConditionOutcome::Matched => {}
                ConditionOutcome::False => current.condition_false = true,
                ConditionOutcome::Unavailable => current.condition_unavailable = true,
                ConditionOutcome::Failed(_) => current.condition_failed = true,
            }
        }
        if current == ConditionState::default() {
            if outcome.candidates > 0 {
                self.conditions.remove(&activation.id);
            }
            return;
        }
        if self.conditions.get(&activation.id) == Some(&current)
            || (self.conditions.len() >= MAX_DEFINITIONS
                && !self.conditions.contains_key(&activation.id))
        {
            return;
        }
        ::tracing::debug!(
            event = "installed_check_condition_filtered",
            check_id = %activation.id.definition_id,
            generation = activation.id.generation.0,
            reason = if current.condition_failed { "condition_failed" }
                else if current.condition_unavailable { "condition_unavailable" }
                else { "condition_false" },
            condition_false = current.condition_false,
            condition_unavailable = current.condition_unavailable,
            condition_failed = current.condition_failed,
            provider_attempts = 0,
        );
        self.conditions.insert(activation.id.clone(), current);
    }

    pub(super) fn retain_conditions(&mut self, mut keep: impl FnMut(&ActivationId) -> bool) {
        self.conditions.retain(|id, _| keep(id));
    }

    pub(super) fn publish(&mut self, current: DiagnosticBatch) {
        for (path, diagnostics) in &current {
            if self.previous.get(path) == Some(diagnostics) {
                continue;
            }
            for diagnostic in diagnostics {
                ::tracing::error!(
                    event = "installed_check_invalid",
                    path = %path.display(),
                    code = diagnostic.code,
                    line = diagnostic.line,
                    column = diagnostic.column,
                    config_location = diagnostic.location,
                    message = diagnostic.message,
                );
            }
        }
        self.previous = current;
    }
}

pub(super) fn lifecycle(
    check_id: &str,
    transition: &antiburn_checks::definition_lifecycle::LifecycleTransition,
) {
    use antiburn_checks::definition_lifecycle::LifecycleTransition;
    match transition {
        LifecycleTransition::Activated(active) => ::tracing::debug!(
            event = "installed_check_activated",
            check_id,
            generation = active.id.generation.0,
            bootstrap_publication = active.bootstrap_publication.0,
            repository_root_available = active.project_root.is_some(),
        ),
        LifecycleTransition::Deactivated {
            next_generation,
            inactive_kind,
            ..
        } => ::tracing::debug!(
            event = "installed_check_deactivated",
            check_id,
            generation = next_generation.0,
            reason = match inactive_kind {
                antiburn_checks::definition_lifecycle::InactiveKind::Invalid => "invalid",
                antiburn_checks::definition_lifecycle::InactiveKind::Paused => "paused",
            },
        ),
        LifecycleTransition::Removed { next_generation } => ::tracing::debug!(
            event = "installed_check_removed",
            check_id,
            generation = next_generation.0,
        ),
        LifecycleTransition::Unchanged(_) | LifecycleTransition::UnchangedInactive => {}
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SourceState {
    waiting_for_data: u64,
    ready_sessions: u64,
    waiting_for_window: usize,
    partial_branches: usize,
    queued: u64,
    in_flight: u64,
    last_status_write: Option<(u64, String)>,
}

impl SourceState {
    pub(super) fn read(
        store: &crate::store::Store,
        activation: &antiburn_checks::definition_lifecycle::EffectiveActivation,
        progress: &antiburn_checks::check_store::SourceProgress,
    ) -> anyhow::Result<Self> {
        use antiburn_local::analysis::{
            ANALYZER_REVISION, EVIDENCE_SCHEMA_REVISION, PARSER_REVISION,
        };
        use rusqlite::{OptionalExtension, params};
        let connection = store.lock();
        let (waiting_for_data, ready_sessions): (u64, u64) = connection.query_row(
            "SELECT COALESCE(SUM(NOT ready AND data_can_arrive), 0), COALESCE(SUM(ready), 0)
              FROM (SELECT CASE WHEN s.environment_key = 'native' AND s.agent = 'opencode'
                AND s.source_kind = 'providerDb' THEN EXISTS (
                  SELECT 1 FROM opencode_journal_delivery d
                   WHERE d.environment_key = s.environment_key AND d.agent = s.agent AND d.session_id = s.session_id
                     AND d.incarnation = s.incarnation AND d.source_label = s.source_label AND d.cwd IS s.cwd
                     AND d.parser_revision = ?3 AND d.cursor_json IS NOT NULL
                     AND d.blocked = 0 AND d.main_session = 1
                     AND (d.last_error IS NULL OR d.last_error IN ('journal-history-incomplete', 'journal-cancelled'))
                     AND NOT EXISTS(SELECT 1 FROM session_relation r WHERE r.environment_key = s.environment_key
                         AND r.agent = s.agent AND r.session_id = s.session_id AND r.kind = 'forkParent'))
                ELSE COALESCE(e.status = 'ready' AND e.analyzed_generation = s.source_generation
                    AND e.parser_revision = ?3 AND e.analyzer_revision = ?4
                    AND e.evidence_schema_revision = ?5, 0) END AS ready,
                e.status IS NULL OR e.status NOT IN ('failed', 'unsupported') AS data_can_arrive
                FROM session s LEFT JOIN session_evidence e USING(environment_key, agent, session_id)
               WHERE s.environment_key = ?1 AND s.cwd = ?2)",
            params![
                activation.id.scope.environment,
                activation.id.scope.session_cwd,
                PARSER_REVISION,
                ANALYZER_REVISION,
                EVIDENCE_SCHEMA_REVISION
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let activation_scope_key =
            crate::check_store::StoreCheckStore::activation_scope_key(&activation.id.scope);
        let (queued, in_flight) = connection.query_row(
            "SELECT COALESCE(SUM(state = 'pending'), 0),
                    COALESCE(SUM(state IN ('claimed', 'dispatching', 'answered')), 0)
               FROM burn_check_candidate WHERE check_id = ?1 AND activation_scope_key = ?2 AND generation = ?3
                 AND state IN ('pending', 'claimed', 'dispatching', 'answered')",
            params![activation.id.definition_id, activation_scope_key, activation.id.generation.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let last_status_write = connection
            .query_row(
                "SELECT h.rowid, st.status FROM burn_check_current_status st
               JOIN burn_check_assessment_history h USING(assessment_id)
              WHERE st.check_id = ?1 AND st.activation_scope_key = ?2 AND st.generation = ?3
              ORDER BY h.rowid DESC LIMIT 1",
                params![
                    activation.id.definition_id,
                    activation_scope_key,
                    activation.id.generation.0
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let minimum = activation
            .compiled
            .trigger
            .window
            .map_or(1, |window| usize::from(window.minimum));
        Ok(Self {
            waiting_for_data,
            ready_sessions,
            waiting_for_window: progress
                .selection
                .branches
                .values()
                .filter(|branch| !branch.results.is_empty() && branch.results.len() < minimum)
                .count(),
            partial_branches: progress
                .selection
                .branches
                .values()
                .filter(|branch| !branch.coverage_complete)
                .count(),
            queued,
            in_flight,
            last_status_write,
        })
    }

    pub(super) fn publish(
        &self,
        activation: &antiburn_checks::definition_lifecycle::EffectiveActivation,
    ) {
        ::tracing::debug!(
            event = "installed_check_source_state",
            check_id = %activation.id.definition_id,
            generation = activation.id.generation.0,
            state = if self.queued > 0 || self.in_flight > 0 { "checking" }
                else if self.waiting_for_data > 0 { "waiting_for_data" }
                else if self.ready_sessions == 0 { "source_unavailable" }
                else if self.waiting_for_window > 0 { "waiting_for_window" }
                else { "waiting_for_results" },
            waiting_for_data = self.waiting_for_data,
            ready_sessions = self.ready_sessions,
            waiting_for_window = self.waiting_for_window,
            partial_branches = self.partial_branches,
            queued = self.queued,
            in_flight = self.in_flight,
            status_write_revision = self.last_status_write.as_ref().map(|(revision, _)| *revision),
            last_written_status = self.last_status_write.as_ref().map(|(_, status)| status.as_str()),
        );
    }
}
