//! Project winning installed definitions into the existing Checks report.

use std::collections::{BTreeMap, BTreeSet};

use antiburn_checks::domain::{ActivationScope, FrozenCandidate, RunScope};
use antiburn_local::analysis::{ANALYZER_REVISION, EVIDENCE_SCHEMA_REVISION, PARSER_REVISION};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::dto::BurnCheckSamplePayload;
use crate::insights_report::ReportRequest;
use crate::remediation::{
    BurnCheckEvidenceItem, BurnCheckEvidenceLabel, BurnCheckEvidenceStatus, BurnCheckSampleSession,
    BurnCheckTargetEvidence,
};
use crate::store::Store;

const MAX_DEFINITIONS: usize = 128;
const MAX_TARGETS: usize = 128;

/// Runtime metadata for one winning definition in one live activation scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledCheckDefinition {
    pub effective_id: String,
    pub check_id: String,
    pub activation_scope_key: String,
    pub generation: Option<i64>,
    pub definition_revision: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub description: Option<String>,
    pub source: InstalledCheckSource,
    pub availability: InstalledCheckAvailability,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledCheckSource {
    pub scope: InstalledCheckScope,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum InstalledCheckScope {
    Global,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub(crate) enum InstalledCheckAvailability {
    Available,
    Paused { reason: Option<String> },
    Invalid { reason: Option<String> },
    Unsupported { reason: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledCheckAvailabilityPayload {
    pub effective_id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub availability: InstalledCheckAvailability,
}

/// Counts use distinct sessions. Current status and latest assessment counts are independent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledCheckReportPayload {
    pub effective_id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub description: Option<String>,
    pub availability: InstalledCheckAvailability,
    pub diagnostic: Option<String>,
    pub explanation: Option<String>,
    pub finding: u64,
    pub clean: u64,
    pub unavailable: u64,
    pub review_failed: u64,
    pub reviewed_without_status: u64,
    pub checking_count: u64,
    pub waiting_for_data_count: u64,
    pub waiting_for_results_count: u64,
    pub agents: Vec<String>,
    pub targets: Vec<InstalledCheckTargetPayload>,
    pub targets_truncated: bool,
}

/// A read-only finding target. It cannot authorize a native remediation action.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledCheckTargetPayload {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub project_name: Option<String>,
    pub source_path: Option<String>,
    pub observation: Option<String>,
    pub evidence_available: bool,
    pub samples: Vec<BurnCheckSamplePayload>,
    #[serde(skip)]
    pub session: BurnCheckSampleSession,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct EvidenceTarget {
    check: String,
    activation_scope_key: String,
    branch: String,
    generation: i64,
    assessment: String,
}

fn grouped_definitions(
    definitions: &[InstalledCheckDefinition],
) -> Result<BTreeMap<&str, Vec<&InstalledCheckDefinition>>> {
    ensure!(
        definitions.len() <= MAX_DEFINITIONS,
        "installed check definition limit exceeded"
    );
    let mut groups = BTreeMap::<&str, Vec<&InstalledCheckDefinition>>::new();
    for definition in definitions {
        let scope: ActivationScope = serde_json::from_str(&definition.activation_scope_key)
            .context("decode installed check scope")?;
        if scope.environment == "native" && scope.run == RunScope::Live {
            groups
                .entry(&definition.check_id)
                .or_default()
                .push(definition);
        }
    }
    Ok(groups)
}

fn metadata(definitions: &[&InstalledCheckDefinition]) -> InstalledCheckAvailabilityPayload {
    let first = definitions
        .iter()
        .copied()
        .min_by_key(|definition| &definition.effective_id)
        .expect("a definition group is nonempty");
    let name = if definitions
        .iter()
        .all(|definition| definition.name == first.name)
    {
        first.name.clone()
    } else {
        first.check_id.clone()
    };
    let aliases = definitions
        .iter()
        .flat_map(|definition| {
            definition
                .aliases
                .iter()
                .chain(std::iter::once(&definition.name))
        })
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let availability = definitions
        .iter()
        .find(|definition| definition.availability == InstalledCheckAvailability::Available)
        .unwrap_or(&first)
        .availability
        .clone();
    InstalledCheckAvailabilityPayload {
        effective_id: format!("installed:{}", first.check_id),
        name,
        aliases,
        availability,
    }
}

/// Search uses the same logical identity and native/live inventory as Checks.
pub(crate) fn availability_rows(
    definitions: &[InstalledCheckDefinition],
) -> Vec<InstalledCheckAvailabilityPayload> {
    // Runtime metadata already passes discovery validation. Exclude malformed scopes.
    let mut groups = BTreeMap::<&str, Vec<&InstalledCheckDefinition>>::new();
    for definition in definitions {
        if let Ok(scope) = serde_json::from_str::<ActivationScope>(&definition.activation_scope_key)
            && scope.environment == "native"
            && scope.run == RunScope::Live
        {
            groups
                .entry(&definition.check_id)
                .or_default()
                .push(definition);
        }
    }
    groups
        .values()
        .map(|definitions| metadata(definitions))
        .collect()
}

/// Read the native thirty-day cohort when the caller has no report request.
#[cfg(test)]
pub(crate) fn read_installed_checks(
    store: &Store,
    definitions: &[InstalledCheckDefinition],
) -> Result<Vec<InstalledCheckReportPayload>> {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    read_installed_checks_in_window(
        store,
        definitions,
        &ReportRequest {
            environment_key: "native".into(),
            window: antiburn_local::insights::ReportWindow {
                start_epoch: now - 30 * 86_400,
                end_epoch: now + 1,
            },
            computed_at_epoch: now,
        },
    )
}

/// Read exact session counts before capping display targets. This performs no writes.
pub(crate) fn read_installed_checks_in_window(
    store: &Store,
    definitions: &[InstalledCheckDefinition],
    request: &ReportRequest,
) -> Result<Vec<InstalledCheckReportPayload>> {
    ensure!(
        request.environment_key == "native",
        "Checks requires the native environment"
    );
    let groups = grouped_definitions(definitions)?;
    let connection = store.lock();
    let transaction = connection.unchecked_transaction()?;
    groups
        .values()
        .map(|definitions| read_row(&transaction, definitions, request))
        .collect()
}

struct BranchResult {
    branch: String,
    checking: bool,
    latest_outcome: Option<String>,
    latest_actions: Option<String>,
    agent: String,
    session_id: String,
    incarnation: u64,
    observed_epoch: i64,
    status: Option<String>,
    assessment: Option<String>,
    actions: Option<String>,
    frozen: Option<String>,
}

// Journal receipts stay current when discovery observes later appends.
fn source_ready_sql(parser: usize, analyzer: usize, evidence: usize) -> String {
    format!("CASE WHEN s.environment_key = 'native' AND s.agent = 'opencode'
          AND s.source_kind = 'providerDb' THEN EXISTS (
            SELECT 1 FROM opencode_journal_delivery d
             WHERE d.environment_key = s.environment_key AND d.agent = s.agent AND d.session_id = s.session_id
               AND d.incarnation = s.incarnation AND d.source_label = s.source_label AND d.cwd IS s.cwd
               AND d.parser_revision = ?{parser} AND d.cursor_json IS NOT NULL
               AND d.blocked = 0 AND d.main_session = 1
               AND (d.last_error IS NULL OR d.last_error IN ('journal-history-incomplete', 'journal-cancelled'))
               AND NOT EXISTS(SELECT 1 FROM session_relation r WHERE r.environment_key = s.environment_key
                   AND r.agent = s.agent AND r.session_id = s.session_id AND r.kind = 'forkParent'))
          ELSE COALESCE(e.status = 'ready' AND e.analyzed_generation = s.source_generation
              AND e.parser_revision = ?{parser} AND e.analyzer_revision = ?{analyzer}
              AND e.evidence_schema_revision = ?{evidence}, 0) END")
}

fn current_branches(
    connection: &Connection,
    definition: &InstalledCheckDefinition,
    request: &ReportRequest,
) -> Result<Vec<BranchResult>> {
    if !current_activation(connection, definition)? {
        return Ok(Vec::new());
    }
    let ready = source_ready_sql(6, 7, 8);
    let mut statement = connection.prepare(&format!(
        "WITH ranked AS (
          SELECT c.*, ROW_NUMBER() OVER (
            PARTITION BY branch_key ORDER BY trigger_position DESC, candidate_id DESC) AS rank
          FROM burn_check_candidate c
          WHERE check_id = ?1 AND activation_scope_key = ?2 AND generation = ?3 AND state <> 'coalesced')
         SELECT c.branch_key, EXISTS(SELECT 1 FROM burn_check_candidate queued
                  WHERE queued.check_id = c.check_id AND queued.activation_scope_key = c.activation_scope_key
                    AND queued.generation = c.generation AND queued.branch_key = c.branch_key
                    AND queued.state IN ('pending', 'claimed', 'dispatching', 'answered')),
                 s.agent, s.session_id, s.incarnation,
                COALESCE(s.updated_at_epoch, s.started_at_epoch),
                CASE WHEN st.evidence_valid = 1 AND h.evidence_valid = 1 AND h.outcome = 'assessed'
                     THEN st.status END,
                 st.assessment_id, h.actions_json, original.frozen_input_json,
                 CASE WHEN latest.outcome <> 'assessed' OR latest.evidence_valid = 1
                      THEN latest.outcome END, latest.actions_json
         FROM ranked c
         JOIN session s ON s.environment_key = json_extract(c.session_key_json, '$[0]')
           AND s.agent = json_extract(c.session_key_json, '$[1]')
           AND s.session_id = json_extract(c.session_key_json, '$[2]')
          LEFT JOIN session_evidence e USING(environment_key, agent, session_id)
         LEFT JOIN burn_check_current_status st ON st.check_id = c.check_id
           AND st.activation_scope_key = c.activation_scope_key AND st.branch_key = c.branch_key AND st.generation = c.generation
         LEFT JOIN burn_check_assessment_history h ON h.assessment_id = st.assessment_id
           AND h.generation = c.generation
          LEFT JOIN burn_check_candidate original ON original.candidate_id = h.candidate_id
          LEFT JOIN burn_check_assessment_history latest ON latest.assessment_id = (
            SELECT assessment_id FROM burn_check_assessment_history
             WHERE check_id = c.check_id AND activation_scope_key = c.activation_scope_key
               AND generation = c.generation AND branch_key = c.branch_key
             ORDER BY trigger_position DESC, assessment_id DESC LIMIT 1)
         WHERE c.rank = 1 AND s.environment_key = 'native'
           AND COALESCE(s.updated_at_epoch, s.started_at_epoch) >= ?4
           AND COALESCE(s.updated_at_epoch, s.started_at_epoch) < ?5
            AND ({ready})
          ORDER BY c.branch_key",
    ))?;
    statement
        .query_map(
            params![
                definition.check_id,
                definition.activation_scope_key,
                definition.generation,
                request.window.start_epoch,
                request.window.end_epoch,
                PARSER_REVISION,
                ANALYZER_REVISION,
                EVIDENCE_SCHEMA_REVISION,
            ],
            |row| {
                Ok(BranchResult {
                    branch: row.get(0)?,
                    checking: row.get(1)?,
                    agent: row.get(2)?,
                    session_id: row.get(3)?,
                    incarnation: row.get(4)?,
                    observed_epoch: row.get(5)?,
                    status: row.get(6)?,
                    assessment: row.get(7)?,
                    actions: row.get(8)?,
                    frozen: row.get(9)?,
                    latest_outcome: row.get(10)?,
                    latest_actions: row.get(11)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

#[derive(Deserialize)]
struct StoredPlan {
    outcomes: Vec<StoredAction>,
}
#[derive(Deserialize)]
struct StoredAction {
    outcome: StoredActionOutcome,
}
#[derive(Deserialize)]
enum StoredActionOutcome {
    Applied {
        result: InstalledCheckStatus,
        message: Option<String>,
    },
    SkippedConditionFalse,
    SkippedConditionUnavailable,
    Failed {
        message: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) enum InstalledCheckStatus {
    Failure,
    Success,
}

fn status_message(actions: Option<&str>, status: &str) -> Result<Option<String>> {
    let Some(actions) = actions else {
        return Ok(None);
    };
    let plan: StoredPlan = serde_json::from_str(actions).context("decode stored status actions")?;
    for action in plan.outcomes.into_iter().rev() {
        match action.outcome {
            StoredActionOutcome::Applied { result, message } => {
                ensure!(
                    matches!(
                        (result, status),
                        (InstalledCheckStatus::Failure, "failure")
                            | (InstalledCheckStatus::Success, "success")
                    ),
                    "status write does not match its action"
                );
                return Ok(message);
            }
            StoredActionOutcome::Failed { message } => {
                drop(message);
            }
            StoredActionOutcome::SkippedConditionFalse
            | StoredActionOutcome::SkippedConditionUnavailable => {}
        }
    }
    anyhow::bail!("current status has no applied action")
}

#[derive(Default)]
struct SessionResult {
    finding: bool,
    clean: bool,
    checking: bool,
    review_failed: bool,
    reviewed_without_status: bool,
    waiting_for_data: bool,
    ready: bool,
}

fn current_activation(
    connection: &Connection,
    definition: &InstalledCheckDefinition,
) -> Result<bool> {
    Ok(
        definition.availability == InstalledCheckAvailability::Available
            && connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM burn_check_activation
              WHERE check_id = ?1 AND activation_scope_key = ?2 AND generation = ?3
                AND definition_revision = ?4 AND enabled = 1)",
                params![
                    definition.check_id,
                    definition.activation_scope_key,
                    definition.generation,
                    definition.definition_revision
                ],
                |row| row.get::<_, bool>(0),
            )?,
    )
}

fn read_row(
    connection: &Connection,
    definitions: &[&InstalledCheckDefinition],
    request: &ReportRequest,
) -> Result<InstalledCheckReportPayload> {
    let meta = metadata(definitions);
    let mut sessions = BTreeMap::<(String, String), SessionResult>::new();
    let mut targets = Vec::new();
    let mut messages = BTreeSet::new();
    let mut diagnostics = BTreeSet::new();
    for definition in definitions {
        if let Some(diagnostic) = &definition.diagnostic {
            diagnostics.insert(diagnostic.clone());
        }
        match &definition.availability {
            InstalledCheckAvailability::Paused { reason }
            | InstalledCheckAvailability::Invalid { reason }
            | InstalledCheckAvailability::Unsupported { reason } => {
                if let Some(reason) = reason {
                    diagnostics.insert(reason.clone());
                }
            }
            InstalledCheckAvailability::Available => {}
        }
        if current_activation(connection, definition)? {
            let scope: ActivationScope = serde_json::from_str(&definition.activation_scope_key)?;
            let ready = source_ready_sql(5, 6, 7);
            let mut statement = connection.prepare(&format!(
                "SELECT s.agent, s.session_id,
                        ({ready}),
                        EXISTS(SELECT 1 FROM burn_check_candidate c
                          WHERE c.check_id = ?8 AND c.activation_scope_key = ?9 AND c.generation = ?10
                            AND json_extract(c.session_key_json, '$[0]') = s.environment_key
                            AND json_extract(c.session_key_json, '$[1]') = s.agent
                            AND json_extract(c.session_key_json, '$[2]') = s.session_id
                            AND c.state IN ('pending', 'claimed', 'dispatching', 'answered')),
                        e.status IS NULL OR e.status NOT IN ('failed', 'unsupported')
                   FROM session s LEFT JOIN session_evidence e USING(environment_key, agent, session_id)
                  WHERE s.environment_key = ?1 AND s.cwd = ?2
                    AND COALESCE(s.updated_at_epoch, s.started_at_epoch) >= ?3
                    AND COALESCE(s.updated_at_epoch, s.started_at_epoch) < ?4
                   ORDER BY s.agent, s.session_id",
            ))?;
            let active = statement
                .query_map(
                    params![
                        scope.environment,
                        scope.session_cwd,
                        request.window.start_epoch,
                        request.window.end_epoch,
                        PARSER_REVISION,
                        ANALYZER_REVISION,
                        EVIDENCE_SCHEMA_REVISION,
                        definition.check_id,
                        definition.activation_scope_key,
                        definition.generation,
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, bool>(2)?,
                            row.get::<_, bool>(3)?,
                            row.get::<_, bool>(4)?,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (agent, session_id, ready, checking, data_can_arrive) in active {
                let result = sessions.entry((agent, session_id)).or_default();
                result.ready |= ready;
                result.waiting_for_data |= !ready && data_can_arrive;
                result.checking |= checking;
            }
        }
        for branch in current_branches(connection, definition, request)? {
            let counts = sessions
                .entry((branch.agent.clone(), branch.session_id.clone()))
                .or_default();
            counts.finding |= branch.status.as_deref() == Some("failure");
            counts.clean |= branch.status.as_deref() == Some("success");
            counts.checking |= branch.checking;
            counts.ready = true;
            counts.review_failed |= matches!(
                branch.latest_outcome.as_deref(),
                Some("execution_failed" | "not_assessed")
            );
            if branch.latest_outcome.as_deref() == Some("assessed") {
                let plan: StoredPlan = serde_json::from_str(
                    branch
                        .latest_actions
                        .as_deref()
                        .context("latest assessment actions missing")?,
                )
                .context("decode latest assessment actions")?;
                counts.reviewed_without_status |= !plan
                    .outcomes
                    .iter()
                    .any(|action| matches!(action.outcome, StoredActionOutcome::Applied { .. }));
            }
            if let Some(status) = branch.status.as_deref() {
                let message = status_message(branch.actions.as_deref(), status)?;
                messages.insert((status == "failure", message.clone()));
                if status == "failure" {
                    let locator = EvidenceTarget {
                        check: definition.check_id.clone(),
                        activation_scope_key: definition.activation_scope_key.clone(),
                        branch: branch.branch,
                        generation: definition
                            .generation
                            .context("current generation missing")?,
                        assessment: branch.assessment.context("status assessment missing")?,
                    };
                    let scope: ActivationScope =
                        serde_json::from_str(&definition.activation_scope_key)?;
                    targets.push(InstalledCheckTargetPayload {
                        id: serde_json::to_string(&locator)?,
                        title: definition.name.clone(),
                        agent: branch.agent.clone(),
                        project_name: std::path::Path::new(&scope.session_cwd)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned()),
                        source_path: definition.source.path.clone(),
                        observation: message,
                        evidence_available: branch
                            .frozen
                            .as_deref()
                            .is_some_and(|value| value != "{}"),
                        samples: Vec::new(),
                        session: BurnCheckSampleSession {
                            environment_key: "native".into(),
                            agent: branch.agent,
                            session_id: branch.session_id,
                            observed_at_ms: branch
                                .observed_epoch
                                .checked_mul(1000)
                                .context("session timestamp overflow")?,
                            incarnation: Some(branch.incarnation),
                        },
                    });
                }
            }
        }
    }
    let finding = sessions.values().filter(|counts| counts.finding).count() as u64;
    let clean = sessions
        .values()
        .filter(|counts| !counts.finding && counts.clean)
        .count() as u64;
    let unavailable = sessions
        .values()
        .filter(|counts| !counts.finding && !counts.clean)
        .count() as u64;
    let review_failed = sessions
        .values()
        .filter(|counts| counts.review_failed)
        .count() as u64;
    let reviewed_without_status = sessions
        .values()
        .filter(|counts| counts.reviewed_without_status)
        .count() as u64;
    let checking_count = sessions.values().filter(|counts| counts.checking).count() as u64;
    let waiting_for_data_count = sessions
        .values()
        .filter(|counts| counts.waiting_for_data && !counts.checking)
        .count() as u64;
    let waiting_for_results_count = sessions
        .values()
        .filter(|counts| counts.ready && !counts.waiting_for_data && !counts.checking)
        .count() as u64;
    let agents = sessions
        .iter()
        .filter(|(_, counts)| {
            if finding > 0 {
                counts.finding
            } else {
                counts.clean
            }
        })
        .map(|((agent, _), _)| agent.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let explanation = messages
        .into_iter()
        .filter(|(failure, _)| *failure == (finding > 0))
        .filter_map(|(_, message)| message)
        .collect::<Vec<_>>()
        .join("\n");
    targets.sort_by(|left, right| {
        right
            .session
            .observed_at_ms
            .cmp(&left.session.observed_at_ms)
            .then(left.id.cmp(&right.id))
    });
    let targets_truncated = targets.len() > MAX_TARGETS;
    targets.truncate(MAX_TARGETS);
    Ok(InstalledCheckReportPayload {
        effective_id: meta.effective_id,
        name: meta.name,
        aliases: meta.aliases,
        description: if definitions
            .iter()
            .all(|definition| definition.description == definitions[0].description)
        {
            definitions[0].description.clone()
        } else {
            None
        },
        availability: meta.availability,
        diagnostic: (!diagnostics.is_empty())
            .then(|| diagnostics.into_iter().collect::<Vec<_>>().join("\n")),
        explanation: (!explanation.is_empty()).then_some(explanation),
        finding,
        clean,
        unavailable,
        review_failed,
        reviewed_without_status,
        checking_count,
        waiting_for_data_count,
        waiting_for_results_count,
        agents,
        targets,
        targets_truncated,
    })
}

/// Mint existing session navigation cards after the report read releases its lock.
pub(crate) fn enrich_samples(
    state: &crate::main_window::MainWindowState,
    store: &Store,
    rows: &mut [InstalledCheckReportPayload],
    now_epoch: i64,
    smart_checks_available: bool,
) -> Result<()> {
    let repositories = store.repositories()?;
    for row in rows {
        for target in &mut row.targets {
            target.samples = crate::main_window::sample_payloads_from_store(
                state,
                store,
                &repositories,
                std::slice::from_ref(&target.session),
                now_epoch,
                smart_checks_available,
            )
            .map_err(anyhow::Error::msg)?;
        }
    }
    Ok(())
}

/// Return frozen source details only while the exact status remains eligible.
pub(crate) fn read_evidence(
    store: &Store,
    definitions: &[InstalledCheckDefinition],
    request: &ReportRequest,
    target_id: &str,
) -> Result<BurnCheckTargetEvidence> {
    ensure!(
        target_id.len() <= 16_384,
        "invalid installed evidence target"
    );
    let locator: EvidenceTarget =
        serde_json::from_str(target_id).context("decode installed evidence target")?;
    let unavailable = || BurnCheckTargetEvidence {
        status: BurnCheckEvidenceStatus::Unavailable,
        items: Vec::new(),
        comparison: None,
        decision_proof: None,
        occurrences: Vec::new(),
    };
    if request.environment_key != "native" {
        return Ok(unavailable());
    }
    let Some(definition) = definitions.iter().find(|definition| {
        definition.check_id == locator.check
            && definition.activation_scope_key == locator.activation_scope_key
            && definition.generation == Some(locator.generation)
    }) else {
        return Ok(unavailable());
    };
    let scope: ActivationScope = serde_json::from_str(&definition.activation_scope_key)?;
    if scope.environment != "native" || scope.run != RunScope::Live {
        return Ok(unavailable());
    }
    let connection = store.lock();
    let transaction = connection.unchecked_transaction()?;
    let Some(branch) = current_branches(&transaction, definition, request)?
        .into_iter()
        .find(|branch| {
            branch.branch == locator.branch
                && branch.status.as_deref() == Some("failure")
                && branch.assessment.as_deref() == Some(&locator.assessment)
        })
    else {
        return Ok(unavailable());
    };
    let Some(frozen) = branch.frozen.filter(|value| value != "{}") else {
        return Ok(unavailable());
    };
    let candidate: FrozenCandidate =
        serde_json::from_str(&frozen).context("decode frozen installed evidence")?;
    let mut items = Vec::new();
    if let Some(requests) = candidate
        .context
        .value
        .pointer("/session/requests")
        .and_then(serde_json::Value::as_array)
    {
        for (index, request) in requests.iter().enumerate() {
            items.push(evidence_item(
                BurnCheckEvidenceLabel::Context,
                "Requested task",
                format!("request:{index}"),
                request,
            ));
        }
    }
    for message in &candidate.messages {
        items.push(evidence_item(
            BurnCheckEvidenceLabel::ObservedAction,
            "Tool result",
            message.id.clone(),
            &message.value,
        ));
    }
    Ok(BurnCheckTargetEvidence {
        status: BurnCheckEvidenceStatus::Available,
        items,
        comparison: None,
        decision_proof: None,
        occurrences: Vec::new(),
    })
}

fn evidence_item(
    label: BurnCheckEvidenceLabel,
    source: &str,
    reference: String,
    value: &serde_json::Value,
) -> BurnCheckEvidenceItem {
    BurnCheckEvidenceItem {
        label,
        source_label: source.into(),
        reference,
        observed_at_ms: None,
        start_line: None,
        end_line: None,
        excerpt: value.to_string(),
        explanation: String::new(),
        limitation: None,
    }
}

#[cfg(test)]
#[path = "insights_report/installed_checks_tests.rs"]
mod tests;
