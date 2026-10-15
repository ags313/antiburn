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
const ACTIVE_SESSION_WINDOW_SECS: i64 = 180;

/// Runtime metadata for one winning definition in one live project.
#[derive(Debug, Clone)]
pub(crate) struct InstalledCheckDefinition {
    pub effective_id: String,
    pub check_id: String,
    pub project_key: String,
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

/// One logical check type. Counts describe distinct sessions with current status.
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
    project: String,
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
        let scope: ActivationScope = serde_json::from_str(&definition.project_key)
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
        if let Ok(scope) = serde_json::from_str::<ActivationScope>(&definition.project_key)
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
    state: String,
    agent: String,
    session_id: String,
    incarnation: u64,
    observed_epoch: i64,
    status: Option<String>,
    assessment: Option<String>,
    actions: Option<String>,
    frozen: Option<String>,
}

fn current_branches(
    connection: &Connection,
    definition: &InstalledCheckDefinition,
    request: &ReportRequest,
) -> Result<Vec<BranchResult>> {
    let current: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM burn_check_activation
          WHERE check_id = ?1 AND project_key = ?2 AND generation = ?3
            AND definition_revision = ?4 AND enabled = 1)",
        params![
            definition.check_id,
            definition.project_key,
            definition.generation,
            definition.definition_revision
        ],
        |row| row.get(0),
    )?;
    if !current || definition.availability != InstalledCheckAvailability::Available {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "WITH ranked AS (
          SELECT c.*, ROW_NUMBER() OVER (
            PARTITION BY branch_key ORDER BY trigger_position DESC, candidate_id DESC) AS rank
          FROM burn_check_candidate c
          WHERE check_id = ?1 AND project_key = ?2 AND generation = ?3 AND state <> 'coalesced')
         SELECT c.branch_key, c.state, s.agent, s.session_id, s.incarnation,
                COALESCE(s.updated_at_epoch, s.started_at_epoch),
                CASE WHEN st.evidence_valid = 1 AND h.evidence_valid = 1 AND h.outcome = 'assessed'
                     THEN st.status END,
                st.assessment_id, h.actions_json, original.frozen_input_json
         FROM ranked c
         JOIN session s ON s.environment_key = json_extract(c.session_key_json, '$[0]')
           AND s.agent = json_extract(c.session_key_json, '$[1]')
           AND s.session_id = json_extract(c.session_key_json, '$[2]')
         JOIN session_evidence e USING(environment_key, agent, session_id)
         LEFT JOIN burn_check_current_status st ON st.check_id = c.check_id
           AND st.project_key = c.project_key AND st.branch_key = c.branch_key AND st.generation = c.generation
         LEFT JOIN burn_check_assessment_history h ON h.assessment_id = st.assessment_id
           AND h.generation = c.generation
         LEFT JOIN burn_check_candidate original ON original.candidate_id = h.candidate_id
         WHERE c.rank = 1 AND s.environment_key = 'native'
           AND COALESCE(s.updated_at_epoch, s.started_at_epoch) >= ?4
           AND COALESCE(s.updated_at_epoch, s.started_at_epoch) < ?5
           AND e.status = 'ready' AND e.analyzed_generation = s.source_generation
           AND e.parser_revision = ?6 AND e.analyzer_revision = ?7 AND e.evidence_schema_revision = ?8
         ORDER BY c.branch_key",
    )?;
    statement
        .query_map(
            params![
                definition.check_id,
                definition.project_key,
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
                    state: row.get(1)?,
                    agent: row.get(2)?,
                    session_id: row.get(3)?,
                    incarnation: row.get(4)?,
                    observed_epoch: row.get(5)?,
                    status: row.get(6)?,
                    assessment: row.get(7)?,
                    actions: row.get(8)?,
                    frozen: row.get(9)?,
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

fn read_row(
    connection: &Connection,
    definitions: &[&InstalledCheckDefinition],
    request: &ReportRequest,
) -> Result<InstalledCheckReportPayload> {
    let meta = metadata(definitions);
    let mut sessions = BTreeMap::<(String, String), (bool, bool, bool, bool, bool, bool)>::new();
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
        if definition.availability == InstalledCheckAvailability::Available {
            let scope: ActivationScope = serde_json::from_str(&definition.project_key)?;
            let mut statement = connection.prepare(
                "SELECT agent, session_id FROM session
                  WHERE environment_key = ?1 AND cwd = ?2 AND updated_at_epoch >= ?3
                    AND NOT EXISTS (
                      SELECT 1 FROM burn_check_current_status st
                       WHERE st.check_id = ?4 AND st.project_key = ?5
                         AND st.generation = ?6 AND st.evidence_valid = 1
                         AND st.branch_key = session.environment_key || '/' || session.agent || '/' || session.session_id)
                  ORDER BY agent, session_id",
            )?;
            let active = statement
                .query_map(
                    params![
                        scope.environment,
                        scope.project,
                        request.computed_at_epoch - ACTIVE_SESSION_WINDOW_SECS,
                        definition.check_id,
                        definition.project_key,
                        definition.generation,
                    ],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (agent, session_id) in active {
                sessions.entry((agent, session_id)).or_default().4 = true;
            }
        }
        for branch in current_branches(connection, definition, request)? {
            let counts = sessions
                .entry((branch.agent.clone(), branch.session_id.clone()))
                .or_default();
            counts.0 |= branch.status.as_deref() == Some("failure");
            counts.1 |= branch.status.as_deref() == Some("success");
            counts.2 |= matches!(
                branch.state.as_str(),
                "pending" | "claimed" | "dispatching" | "answered"
            );
            counts.3 |= matches!(branch.state.as_str(), "execution_failed" | "not_assessed");
            counts.5 |= branch.state == "accepted" && branch.status.is_none();
            if let Some(status) = branch.status.as_deref() {
                let message = status_message(branch.actions.as_deref(), status)?;
                messages.insert((status == "failure", message.clone()));
                if status == "failure" {
                    let locator = EvidenceTarget {
                        check: definition.check_id.clone(),
                        project: definition.project_key.clone(),
                        branch: branch.branch,
                        generation: definition
                            .generation
                            .context("current generation missing")?,
                        assessment: branch.assessment.context("status assessment missing")?,
                    };
                    let scope: ActivationScope = serde_json::from_str(&definition.project_key)?;
                    targets.push(InstalledCheckTargetPayload {
                        id: serde_json::to_string(&locator)?,
                        title: definition.name.clone(),
                        agent: branch.agent.clone(),
                        project_name: std::path::Path::new(&scope.project)
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
    let finding = sessions.values().filter(|counts| counts.0).count() as u64;
    let clean = sessions
        .values()
        .filter(|counts| !counts.0 && counts.1)
        .count() as u64;
    let unavailable = sessions
        .values()
        .filter(|counts| !counts.0 && !counts.1)
        .count() as u64;
    let review_failed = sessions
        .values()
        .filter(|counts| !counts.0 && !counts.1 && counts.3)
        .count() as u64;
    let reviewed_without_status = sessions.values().filter(|counts| counts.5).count() as u64;
    let checking_count = sessions
        .values()
        .filter(|counts| counts.2 || counts.4)
        .count() as u64;
    let agents = sessions
        .iter()
        .filter(|(_, counts)| if finding > 0 { counts.0 } else { counts.1 })
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
            && definition.project_key == locator.project
            && definition.generation == Some(locator.generation)
    }) else {
        return Ok(unavailable());
    };
    let scope: ActivationScope = serde_json::from_str(&definition.project_key)?;
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
