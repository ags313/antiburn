//! Project winning installed definitions into the existing Checks report.

use std::collections::{BTreeMap, BTreeSet};

use antiburn_checks::domain::{ActivationScope, FrozenCandidate, RunScope};
use antiburn_checks::fit::PreparationRecord;
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
use crate::store::native_tool_results::opencode_v2_status::{
    OpenCodeV2SourceStatus, SOURCE_SQL as OPENCODE_V2_SOURCE_SQL, live_capture_sql,
    source_status_sql,
};
use crate::store::{SessionKey, Store};

#[path = "installed_checks_report/preparation.rs"]
mod preparation;
use preparation::ReportPreparation;

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
    pub decision_name: Option<String>,
    pub trigger: Option<InstalledCheckTrigger>,
    pub source: InstalledCheckSource,
    pub availability: InstalledCheckAvailability,
    pub diagnostic: Option<String>,
    pub diagnostics: Vec<InstalledCheckDiagnostic>,
    pub definition_sources: Vec<InstalledCheckDiagnosticSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledCheckDiagnostic {
    pub code: String,
    pub severity: InstalledCheckDiagnosticSeverity,
    pub title: String,
    pub message: String,
    pub source: Option<InstalledCheckDiagnosticSource>,
    pub details: Vec<InstalledCheckDiagnosticDetail>,
    pub affected_session_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum InstalledCheckDiagnosticSeverity {
    Error,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledCheckDiagnosticSource {
    pub reference: String,
    pub path: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InstalledCheckDiagnosticDetail {
    pub label: String,
    pub value: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum InstalledCheckEvent {
    ToolResult,
    UserMessage,
    AssistantMessage,
    ToolCall,
    Message,
    TurnEnd,
    SessionStart,
    SessionIdle,
    SessionEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstalledCheckTrigger {
    pub event: InstalledCheckEvent,
    pub minimum: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledCheckWaitingFor {
    pub event: InstalledCheckEvent,
    pub count: u8,
    pub catching_up: bool,
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
    pub diagnostics: Vec<InstalledCheckDiagnostic>,
    pub explanation: Option<String>,
    pub finding: u64,
    pub clean: u64,
    pub unavailable: u64,
    pub review_failed: u64,
    pub reviewed_without_status: u64,
    pub has_started: bool,
    pub checking_count: u64,
    pub waiting_for_data_count: u64,
    pub waiting_for_results_count: u64,
    pub waiting_for_events: Option<InstalledCheckWaitingFor>,
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
    let transaction = match connection.unchecked_transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            ::tracing::error!(event = "installed_check_report_transaction_failed", error = %error);
            return Ok(groups
                .values()
                .map(|definitions| failed_report(definitions))
                .collect());
        }
    };
    groups
        .values()
        .map(
            |definitions| match read_row(&transaction, definitions, request) {
                Ok(row) => {
                    trace_report(&row);
                    Ok(row)
                }
                Err(error) => {
                    ::tracing::error!(event = "installed_check_report_failed", error = %error);
                    Ok(failed_report(definitions))
                }
            },
        )
        .collect()
}

fn trace_report(row: &InstalledCheckReportPayload) {
    use std::sync::{Mutex, OnceLock};
    type State = (bool, u64, u64, u64, u64, u64, u64, Option<(u8, bool)>);
    static PREVIOUS: OnceLock<Mutex<BTreeMap<String, State>>> = OnceLock::new();
    let state = (
        row.availability == InstalledCheckAvailability::Available,
        row.finding,
        row.clean,
        row.unavailable,
        row.checking_count,
        row.waiting_for_data_count,
        row.waiting_for_results_count,
        row.waiting_for_events
            .map(|waiting| (waiting.count, waiting.catching_up)),
    );
    let mut previous = PREVIOUS
        .get_or_init(Default::default)
        .lock()
        .expect("report trace lock");
    if previous.get(&row.effective_id) == Some(&state) {
        return;
    }
    if previous.len() >= MAX_DEFINITIONS {
        previous.clear();
    }
    previous.insert(row.effective_id.clone(), state);
    ::tracing::debug!(event = "installed_check_report_state", check_id = %row.effective_id,
        reason = if row.availability != InstalledCheckAvailability::Available { "not_available" }
            else if row.checking_count > 0 { "checking" }
            else if row.waiting_for_data_count > 0 { "source_or_attachment_catchup" }
            else if row.waiting_for_results_count > 0 { "waiting_for_matching_results" }
            else if row.finding + row.clean > 0 { "saved_status" }
            else if row.unavailable > 0 { "no_usable_status" }
            else { "no_current_eligible_sessions" },
        finding = row.finding, clean = row.clean, unavailable = row.unavailable,
        checking = row.checking_count, waiting_for_data = row.waiting_for_data_count,
        waiting_for_results = row.waiting_for_results_count,
        remaining_events = row.waiting_for_events.map(|waiting| waiting.count),
        catching_up = row.waiting_for_events.is_some_and(|waiting| waiting.catching_up));
}

fn storage_diagnostic() -> InstalledCheckDiagnostic {
    crate::installed_checks::diagnostics::payload(
        "storage-failed",
        InstalledCheckDiagnosticSeverity::Error,
        "Check data could not be read",
        "Some saved check data is unavailable. Check disk space and local data access, then refresh Checks.",
        None,
        Vec::new(),
    )
}

fn failed_report(definitions: &[&InstalledCheckDefinition]) -> InstalledCheckReportPayload {
    let meta = metadata(definitions);
    let mut diagnostics = definitions
        .iter()
        .flat_map(|definition| definition.diagnostics.clone())
        .collect::<Vec<_>>();
    diagnostics.push(storage_diagnostic());
    InstalledCheckReportPayload {
        effective_id: meta.effective_id,
        name: meta.name,
        aliases: meta.aliases,
        description: None,
        availability: meta.availability,
        diagnostic: None,
        diagnostics,
        explanation: None,
        finding: 0,
        clean: 0,
        unavailable: 0,
        review_failed: 0,
        reviewed_without_status: 0,
        has_started: false,
        checking_count: 0,
        waiting_for_data_count: 0,
        waiting_for_results_count: 0,
        waiting_for_events: None,
        agents: Vec::new(),
        targets: Vec::new(),
        targets_truncated: false,
    }
}

struct BranchResult {
    branch: String,
    checking: bool,
    has_started: bool,
    latest_outcome: Option<String>,
    latest_actions: Option<String>,
    latest_preparation: Option<String>,
    latest_diagnostic: Option<String>,
    latest_decision: Option<String>,
    agent: String,
    session_id: String,
    incarnation: u64,
    observed_epoch: i64,
    status: Option<String>,
    assessment: Option<String>,
    actions: Option<String>,
    has_frozen: bool,
    source_ready: bool,
}

#[derive(Default)]
struct DiagnosticGroups(BTreeMap<String, (InstalledCheckDiagnostic, BTreeSet<(String, String)>)>);

impl DiagnosticGroups {
    fn add(
        &mut self,
        diagnostic: InstalledCheckDiagnostic,
        session: Option<(String, String)>,
    ) -> Result<()> {
        let key = serde_json::to_string(&diagnostic)?;
        let (_, sessions) = self
            .0
            .entry(key)
            .or_insert_with(|| (diagnostic, BTreeSet::new()));
        if let Some(session) = session {
            sessions.insert(session);
        }
        Ok(())
    }

    fn finish(self) -> Vec<InstalledCheckDiagnostic> {
        let mut diagnostics = self
            .0
            .into_values()
            .map(|(mut diagnostic, sessions)| {
                diagnostic.affected_session_count = sessions.len() as u64;
                diagnostic
            })
            .collect::<Vec<_>>();
        diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.severity == InstalledCheckDiagnosticSeverity::Info,
                diagnostic.code.clone(),
                diagnostic.source.as_ref().map(|source| source.path.clone()),
            )
        });
        diagnostics
    }
}

fn runtime_diagnostics(
    definition: &InstalledCheckDefinition,
    branch: &BranchResult,
) -> Result<Vec<InstalledCheckDiagnostic>> {
    use crate::installed_checks::diagnostics::{detail, payload};
    use antiburn_checks::engine::{EvaluationDiagnostic, EvaluationFailureCode};
    use antiburn_checks::fit::{FitStrategy, PreparationFailureCode};
    let mut diagnostics = Vec::new();
    let mut provenance = None;
    let mut condition_false = false;
    let mut diagnostic_decision = branch
        .latest_decision
        .clone()
        .or_else(|| definition.decision_name.clone());
    let local = definition.decision_name.is_none();
    let source = definition.definition_sources.first().cloned();
    let failed = matches!(
        branch.latest_outcome.as_deref(),
        Some("execution_failed" | "not_assessed")
    );
    if failed && let Some(json) = &branch.latest_diagnostic {
        let diagnostic: EvaluationDiagnostic =
            serde_json::from_str(json).context("decode check evaluation diagnostic")?;
        use EvaluationFailureCode::*;
        let (code, title, message) = match diagnostic.code {
            AuthenticationRejected => (
                "provider-authentication",
                "Reconnect the assessment provider",
                "The provider rejected the saved credential. Update the selected connection in Settings → Checks.",
            ),
            RateLimited => (
                "provider-rate-limited",
                "Provider rate limit reached",
                "The assessment provider reached its rate limit. Check your provider limits before assessing new activity.",
            ),
            ProviderOverloaded => (
                "provider-overloaded",
                "The assessment provider is busy",
                "The provider could not complete this assessment. Check its availability before assessing new activity.",
            ),
            ProviderUnavailable => (
                "provider-unavailable",
                "The assessment provider is unavailable",
                "Check the selected connection in Settings → Checks and your network connection.",
            ),
            DeliveryUnknown => (
                "provider-delivery-unknown",
                "The assessment did not finish",
                "The connection ended before the provider confirmed a response. This can include a timeout. The request may have reached the provider, so antiburn will not resend it automatically.",
            ),
            InvalidResponse | InvalidDecision => (
                "provider-invalid-response",
                "The provider returned an invalid response",
                "The response does not match the answers defined by this check. Review the decision in the YAML file and test the selected connection in Settings → Checks.",
            ),
            ContextRejected => (
                "provider-context-rejected",
                "The provider rejected the input size",
                "The provider rejected this check's input as too large. Shorten the decision input or select a decision model with a larger context in Settings → Checks. If the check can omit some information, add fitting rules in its YAML file.",
            ),
            InvalidRequest => (
                "provider-invalid-request",
                "Correct the decision request",
                "The provider could not accept this decision request. Check the model in Settings → Checks and the decision in the YAML file.",
            ),
            Cancelled => (
                "assessment-cancelled",
                "The assessment was stopped",
                "The check or provider connection changed before this assessment finished. Check the definition and the selected connection in Settings → Checks.",
            ),
            StorageFailure => (
                "storage-failed",
                "The assessment could not be saved",
                "Check available disk space and local data access, then refresh Checks.",
            ),
            ActionFailed => (
                "action-failed",
                "The status action could not run",
                "A status action was invalid or reached an evaluation limit. Check the action expression and result in the YAML file.",
            ),
        };
        diagnostics.push(payload(
            code,
            InstalledCheckDiagnosticSeverity::Error,
            title,
            message,
            source.clone(),
            Vec::new(),
        ));
    }
    if let Some(json) = &branch.latest_preparation {
        let preparation = match preparation::decode(
            json.as_bytes(),
            branch.latest_outcome.as_deref() == Some("assessed"),
        ) {
            Ok(preparation) => preparation,
            Err(_) if local => {
                diagnostics.push(local_state_diagnostic());
                return Ok(diagnostics);
            }
            Err(error) => return Err(error).context("decode check preparation"),
        };
        let (reports, rejected) = match preparation {
            ReportPreparation::Local(prepared) => {
                if !local || prepared.validate_for_resume().is_err() {
                    diagnostics.push(local_state_diagnostic());
                }
                (Vec::new(), false)
            }
            ReportPreparation::Model(_) if local => {
                diagnostics.push(local_state_diagnostic());
                (Vec::new(), false)
            }
            ReportPreparation::Model(prepared) => {
                if branch.latest_decision.is_none() && prepared.blocks.len() > 1 {
                    diagnostic_decision = None;
                }
                provenance =
                    crate::jev::decision_evaluator::prepared_profile_provenance(&prepared.profile);
                (
                    prepared
                        .blocks
                        .into_iter()
                        .map(|block| (block.decision, block.report))
                        .collect(),
                    false,
                )
            }
            ReportPreparation::Rejected(failure)
                if local
                    && !matches!(
                        failure.code,
                        PreparationFailureCode::ConditionFalse
                            | PreparationFailureCode::InputUnavailable
                            | PreparationFailureCode::InvalidInput
                            | PreparationFailureCode::IncompatibleProfile
                    ) =>
            {
                diagnostics.push(local_state_diagnostic());
                (Vec::new(), false)
            }
            ReportPreparation::Rejected(failure) => {
                condition_false = failure.code == PreparationFailureCode::ConditionFalse;
                if failed {
                    use PreparationFailureCode::*;
                    let text = match failure.code {
                        ConditionFalse => None,
                        NoFit => Some(no_fit_text(&failure.context)),
                        PreparationResourceLimit => Some((
                            "preparation-resource-limit",
                            "Input preparation reached a local limit",
                            "A local preparation limit stopped this assessment. Select less input or fewer fitting targets. Nothing was sent to the provider.",
                        )),
                        InvalidFit => Some((
                            "fit-config-invalid",
                            "Correct the input fitting rules",
                            "A fitting target has the wrong type or overlaps another target. Use recent for arrays and truncate for text. Nothing was sent to the provider.",
                        )),
                        InputUnavailable if local => Some((
                            "input-unavailable",
                            "The check condition needs unavailable evidence",
                            "The check's top-level condition depends on unavailable session evidence. New activity can run this check again.",
                        )),
                        InputUnavailable => Some((
                            "input-unavailable",
                            "Required decision input is unavailable",
                            "This session does not provide a field required by the decision input. Check its input references and source coverage. Nothing was sent to the provider.",
                        )),
                        InvalidInput if local => Some((
                            "input-invalid",
                            "The check condition could not run",
                            "Check the top-level condition expression and available session evidence in the YAML file.",
                        )),
                        InvalidInput => Some((
                            "input-invalid",
                            "The input could not be prepared",
                            "Check the decision input expressions and selected session context. Nothing was sent to the provider.",
                        )),
                        InvalidRequest => Some((
                            "provider-invalid-request",
                            "Correct the decision request",
                            "The decision does not match the supported model request format. Check the field and local limit shown in Details. Nothing was sent to the provider.",
                        )),
                        ProviderUnavailable => Some((
                            "provider-unavailable",
                            "The decision model is unavailable",
                            "Check the selected model and connection in Settings → Checks. Nothing was sent to the provider.",
                        )),
                        IncompatibleProfile if local => Some((
                            "local-state-invalid",
                            "Local check data is unavailable",
                            "The saved local check state is incomplete or incompatible. Its context is unavailable. New activity can run this check again.",
                        )),
                        IncompatibleProfile => Some((
                            "provider-profile-changed",
                            "The decision configuration changed",
                            "The saved request no longer matches this decision model's configuration. New activity can create a new assessment; the saved request was not sent again.",
                        )),
                    };
                    if let Some((code, title, message)) = text {
                        diagnostics.push(payload(
                            code,
                            InstalledCheckDiagnosticSeverity::Error,
                            title,
                            message,
                            source.clone(),
                            if local {
                                Vec::new()
                            } else {
                                preparation_details(&failure.context)
                            },
                        ));
                    }
                }
                (
                    if local {
                        Vec::new()
                    } else {
                        failure
                            .report
                            .into_iter()
                            .map(|report| (failure.context.decision.clone(), report))
                            .collect()
                    },
                    true,
                )
            }
        };
        for (decision, report) in reports
            .into_iter()
            .filter(|(_, report)| !report.omissions.is_empty())
        {
            let mut details = report
                .omissions
                .iter()
                .map(|omission| {
                    let (strategy, unit) = match omission.strategy {
                        FitStrategy::Recent => ("recent", "records"),
                        FitStrategy::Truncate => ("truncate", "UTF-8 bytes"),
                    };
                    detail(
                        &omission.path,
                        format!(
                            "{strategy}: retained {} of {} {unit}",
                            omission.retained_count, omission.original_count
                        ),
                    )
                })
                .collect::<Vec<_>>();
            if !rejected && let Some(decision) = decision {
                details.push(detail("Decision", decision));
            }
            if rejected {
                if let Some(diagnostic) = diagnostics.last_mut() {
                    diagnostic.details.extend(details);
                }
            } else if branch.latest_outcome.as_deref() == Some("assessed") {
                diagnostics.push(payload("input-shortened", InstalledCheckDiagnosticSeverity::Info, "Input shortened", "Fitting rules shortened this assessment's input. Session evidence is unchanged.", source.clone(), details));
            }
        }
    }
    if local && branch.latest_outcome.is_some() && branch.latest_preparation.is_none() {
        diagnostics.push(local_state_diagnostic());
    }
    if failed && diagnostics.is_empty() && !condition_false {
        diagnostics.push(payload("assessment-failed", InstalledCheckDiagnosticSeverity::Error, "The assessment did not complete", "No new status was written. Check the definition, session source access, and selected connection before assessing new activity.", source, Vec::new()));
    }
    if let Some(decision) = &diagnostic_decision {
        for diagnostic in &mut diagnostics {
            if !diagnostic
                .details
                .iter()
                .any(|detail| detail.label == "Decision")
            {
                diagnostic.details.push(detail("Decision", decision));
            }
        }
    }
    if let Some(provenance) = provenance {
        for diagnostic in &mut diagnostics {
            diagnostic
                .details
                .push(detail("Provider", provider_name(provenance.provider)));
            diagnostic.details.push(detail("Model", &provenance.model));
        }
    }
    Ok(diagnostics)
}

fn local_state_diagnostic() -> InstalledCheckDiagnostic {
    crate::installed_checks::diagnostics::payload(
        "local-state-invalid",
        InstalledCheckDiagnosticSeverity::Error,
        "Local check data is unavailable",
        "The saved local check state is incomplete or incompatible. Its context is unavailable. New activity can run this check again.",
        None,
        Vec::new(),
    )
}

fn provider_name(provider: crate::jev::config::SystemOneProvider) -> &'static str {
    use crate::jev::config::SystemOneProvider;
    match provider {
        SystemOneProvider::Jev => "Jev",
        SystemOneProvider::Ollama => "Ollama",
        SystemOneProvider::Cloudflare => "Cloudflare",
        SystemOneProvider::Custom => "Custom",
    }
}

fn no_fit_text(
    context: &antiburn_checks::fit::PreparationFailureContext,
) -> (&'static str, &'static str, &'static str) {
    use antiburn_checks::fit::{NoFitReason, PreparationBudgetKind};
    match context.no_fit_reason {
        Some(NoFitReason::NoRules) => (
            "input-too-large",
            "The check input is too large",
            match context.budget.as_ref().map(|budget| budget.kind) {
                Some(PreparationBudgetKind::EstimatedTokens) => {
                    "This check sends more information than the selected decision model can accept. Shorten the decision input or select a larger decision model in Settings → Checks. If the check can omit some information, add fitting rules in its YAML file. Nothing was sent to the provider."
                }
                Some(PreparationBudgetKind::ResidentBytes) => {
                    "This check's input exceeds a local memory limit for model requests. Shorten the decision input. If the check can omit some information, add fitting rules in its YAML file. Nothing was sent to the provider."
                }
                _ => {
                    "This check's input exceeds a local request-size limit. Shorten the decision input. If the check can omit some information, add fitting rules in its YAML file. Nothing was sent to the provider."
                }
            },
        ),
        Some(NoFitReason::NoMatchingTargets) => (
            "input-too-large",
            "No fitting targets matched",
            "The complete request exceeds a local request limit, and no current input matched the fitting paths. Check the paths or select less input. Nothing was sent to the provider.",
        ),
        Some(NoFitReason::TargetsAtMinimum) => (
            "input-too-large",
            "Fitting targets cannot be reduced",
            "The complete request exceeds a local request limit. The selected targets cannot be reduced further. Select less input or permit fitting of other fields. Nothing was sent to the provider.",
        ),
        Some(NoFitReason::ReductionsExhausted) => (
            "input-too-large",
            "The input is still too large",
            "The complete request exceeds a local request limit after all permitted reductions. Select less input or permit fitting of other fields. Nothing was sent to the provider.",
        ),
        None => (
            "preparation-incomplete",
            "Input preparation did not complete",
            "The saved assessment stopped at a local preparation limit, but it does not identify which limit. Review the selected input and fitting rules. Nothing was sent to the provider.",
        ),
    }
}

fn preparation_details(
    context: &antiburn_checks::fit::PreparationFailureContext,
) -> Vec<InstalledCheckDiagnosticDetail> {
    use crate::installed_checks::diagnostics::detail;
    use antiburn_checks::fit::{NoFitReason, PreparationBudgetKind};
    let mut details = Vec::new();
    for (label, value) in [
        ("Decision", &context.decision),
        ("Field", &context.field),
        ("Input path", &context.input_path),
    ] {
        if let Some(value) = value {
            details.push(detail(label, value));
        }
    }
    if let Some(reason) = context.no_fit_reason {
        details.push(detail(
            "Fitting result",
            match reason {
                NoFitReason::NoRules => "No fitting rules are configured.",
                NoFitReason::NoMatchingTargets => "The fitting paths have no current targets.",
                NoFitReason::TargetsAtMinimum => {
                    "The targets cannot become smaller under the retention and omission rules."
                }
                NoFitReason::ReductionsExhausted => "All permitted reductions were exhausted.",
            },
        ));
    }
    if let Some(budget) = &context.budget {
        let (label, unit) = match budget.kind {
            PreparationBudgetKind::InputBytes => ("Original input size", "bytes"),
            PreparationBudgetKind::InputNodes => ("Input values", "nodes"),
            PreparationBudgetKind::InputDepth => ("Input nesting", "levels"),
            PreparationBudgetKind::FitRules => ("Fitting rules", "rules"),
            PreparationBudgetKind::PathDepth => ("Fitting path depth", "segments"),
            PreparationBudgetKind::PathBytes => ("Fitting path length", "bytes"),
            PreparationBudgetKind::ExpandedTargets => ("Expanded fitting targets", "targets"),
            PreparationBudgetKind::TraversalNodes => ("Fitting traversal", "node visits"),
            PreparationBudgetKind::Turns => ("Input reductions", "turns"),
            PreparationBudgetKind::WorkBytes => ("Cumulative preparation work", "bytes"),
            PreparationBudgetKind::RequestBytes => ("Serialized request size", "bytes"),
            PreparationBudgetKind::EstimatedTokens => {
                ("Estimated request size", "estimated tokens")
            }
            PreparationBudgetKind::ResidentBytes => ("Resident request size", "bytes"),
            PreparationBudgetKind::Criteria => ("Question criteria", "criteria"),
        };
        details.push(detail(
            label,
            format!(
                "{} {unit}; local limit {} {unit}",
                budget.observed, budget.maximum
            ),
        ));
    }
    details
}

// Journal receipts stay current when discovery observes later appends.
pub(crate) fn source_ready_sql(parser: usize, analyzer: usize, evidence: usize) -> String {
    source_ready_at_sql(parser, analyzer, evidence, "unixepoch()")
}

fn source_ready_at_sql(parser: usize, analyzer: usize, evidence: usize, now: &str) -> String {
    let v2 = live_capture_sql(now);
    let v1_source = crate::store::selected_source::OPENCODE_V1_SOURCE_SQL;
    format!("CASE WHEN s.agent = 'opencode' AND s.source_kind = 'providerDb'
           AND s.source_format = '\"open_code_v2_sqlite\"' THEN ({v2})
           WHEN ({v1_source}) THEN EXISTS (
            SELECT 1 FROM opencode_journal_delivery d
             WHERE d.environment_key = s.environment_key AND d.agent = s.agent AND d.session_id = s.session_id
               AND d.incarnation = s.incarnation AND d.source_label = s.source_label AND d.cwd IS s.cwd
               AND d.parser_revision IN (?{parser}, 63) AND d.cursor_json IS NOT NULL
               AND d.blocked = 0 AND d.main_session = 1
               AND (d.last_error IS NULL OR d.last_error IN ('journal-history-incomplete', 'journal-cancelled'))
               AND NOT EXISTS(SELECT 1 FROM session_relation r WHERE r.environment_key = s.environment_key
                   AND r.agent = s.agent AND r.session_id = s.session_id AND r.kind = 'forkParent'))
           WHEN s.environment_key = 'native' AND s.agent IN ('claude-code', 'codex', 'pi')
             AND s.source_kind = 'file' THEN EXISTS (
             SELECT 1 FROM native_file_delivery d
              WHERE d.environment_key = s.environment_key AND d.agent = s.agent AND d.session_id = s.session_id
                AND d.incarnation = s.incarnation AND d.source_label = s.source_label AND d.cwd IS s.cwd
                 AND d.parser_revision IN (?{parser}, 63) AND d.cursor_json IS NOT NULL
                AND d.blocked = 0 AND d.last_error IS NULL
                AND NOT EXISTS(SELECT 1 FROM session_relation r
                  WHERE r.environment_key = s.environment_key AND r.agent = s.agent
                    AND ((r.session_id = s.session_id AND r.kind = 'forkParent')
                      OR (r.related_id = s.session_id AND r.kind = 'subagent'))))
           ELSE COALESCE(e.status = 'ready' AND e.analyzed_generation = s.source_generation
              AND e.parser_revision = ?{parser} AND e.analyzer_revision = ?{analyzer}
              AND e.evidence_schema_revision = ?{evidence}, 0) END")
}

pub(crate) fn source_can_arrive_sql(parser: usize, active_since: usize) -> String {
    // Match delivery reset admission. A compatible pending claim can outlive activity.
    let route = |table: &str, reset: &str| {
        format!(
            "COALESCE((SELECT CASE WHEN d.parser_revision NOT IN (?{parser}, 62, 63) THEN 0
           WHEN {reset} THEN s.updated_at_epoch >= ?{active_since}
           ELSE d.blocked = 0 AND (d.last_error IS NULL OR d.last_error NOT IN ('native-unsafe-identity', 'native-unsafe-order', 'native-unsafe-ownership'))
             AND (d.pending = 1 OR d.next_attempt_at_epoch IS NOT NULL
            OR d.observed_generation <> s.source_generation) END
          FROM {table} d WHERE d.environment_key = s.environment_key AND d.agent = s.agent
            AND d.session_id = s.session_id), s.updated_at_epoch >= ?{active_since}, 0)"
        )
    };
    let journal = route(
        "opencode_journal_delivery",
        "d.incarnation <> s.incarnation OR d.source_label <> s.source_label
           OR d.cwd IS NOT s.cwd OR d.main_session <> 1",
    );
    let file = route(
        "native_file_delivery",
        "d.incarnation <> s.incarnation OR ((d.source_label <> s.source_label
           OR d.cwd IS NOT s.cwd) AND d.cursor_json IS NULL)",
    );
    let v2 = live_capture_sql(&format!(
        "?{active_since} + {}",
        antiburn_local::discovery::ACTIVE_SESSION_WINDOW_SECS
    ));
    let v1_source = crate::store::selected_source::OPENCODE_V1_SOURCE_SQL;
    format!(
        "s.environment_key = 'native' AND s.wsl_distro IS NULL
      AND s.cwd IS NOT NULL AND s.cwd <> ''
      AND NOT EXISTS(SELECT 1 FROM session_relation r
        WHERE r.environment_key = s.environment_key AND r.agent = s.agent
          AND ((r.session_id = s.session_id AND r.kind = 'forkParent')
            OR (r.related_id = s.session_id AND r.kind = 'subagent')))
       AND CASE WHEN s.agent = 'opencode' AND s.source_kind = 'providerDb'
          AND s.source_format = '\"open_code_v2_sqlite\"' THEN ({v2})
          WHEN ({v1_source}) THEN {journal}
        WHEN s.agent IN ('claude-code', 'codex', 'pi') AND s.source_kind = 'file' THEN {file}
        ELSE e.status IS NULL OR e.status NOT IN ('failed', 'unsupported') END"
    )
}

struct CurrentSession {
    key: SessionKey,
    ready: bool,
    checking: bool,
    data_can_arrive: bool,
    journal_owned: bool,
    catching_up: bool,
    v2_status: Option<OpenCodeV2SourceStatus>,
}

fn current_sessions(
    connection: &Connection,
    definition: &InstalledCheckDefinition,
    request: &ReportRequest,
) -> Result<Vec<CurrentSession>> {
    let scope: ActivationScope = serde_json::from_str(&definition.activation_scope_key)?;
    let ready = source_ready_at_sql(5, 6, 7, "?12");
    let can_arrive = source_can_arrive_sql(5, 11);
    let v2_status = source_status_sql("?12");
    let target = crate::store::native_tool_results::evidence::target_predicate("p", 8, 9, 10);
    let v1_source = crate::store::selected_source::OPENCODE_V1_SOURCE_SQL;
    let mut statement = connection.prepare(&format!(
        "SELECT s.agent, s.session_id, ({ready}),
                EXISTS(SELECT 1 FROM burn_check_candidate c
                  WHERE c.check_id = ?8 AND c.activation_scope_key = ?9 AND c.generation = ?10
                    AND json_extract(c.session_key_json, '$[0]') = s.environment_key
                    AND json_extract(c.session_key_json, '$[1]') = s.agent
                    AND json_extract(c.session_key_json, '$[2]') = s.session_id
                    AND c.state IN ('pending', 'claimed', 'dispatching', 'answered')),
                ({can_arrive}),
                 ({v1_source}),
                 ((NOT ({ready}) AND ({can_arrive}))
                    OR (({ready}) AND (s.updated_at_epoch >= ?11
                      OR EXISTS(SELECT 1 FROM native_tool_result_publication p
                        JOIN burn_check_activation a ON a.check_id = ?8 AND a.activation_scope_key = ?9
                          AND a.generation = ?10 AND a.enabled = 1
                        WHERE p.environment_key = s.environment_key AND p.agent = s.agent
                           AND p.session_id = s.session_id AND p.publication_sequence > a.bootstrap_sequence
                           AND {target}
                          AND (p.session_incarnation IS NULL OR
                            (p.session_incarnation = s.incarnation AND p.project_cwd = s.cwd))))
                       AND (({v1_source})
                       OR (s.agent IN ('claude-code', 'codex', 'pi') AND s.source_kind = 'file'))
                     AND NOT EXISTS(SELECT 1 FROM burn_check_session_attachment t
                       WHERE t.check_id = ?8 AND t.activation_scope_key = ?9 AND t.generation = ?10
                         AND t.branch_key = json_array(s.environment_key, s.agent, s.session_id)
                         AND t.incarnation = s.incarnation))
                   OR EXISTS(SELECT 1 FROM native_tool_result_publication p
                     JOIN burn_check_activation a ON a.check_id = ?8 AND a.activation_scope_key = ?9
                       AND a.generation = ?10 AND a.enabled = 1
                     WHERE p.environment_key = s.environment_key AND p.agent = s.agent
                        AND p.session_id = s.session_id AND p.publication_sequence > a.applied_sequence
                        AND {target}
                       AND (p.session_incarnation IS NULL OR
                         (p.session_incarnation = s.incarnation AND p.project_cwd = s.cwd)))
                    OR (({v1_source})
                     AND EXISTS(SELECT 1 FROM opencode_journal_delivery d
                      WHERE d.environment_key = s.environment_key AND d.agent = s.agent
                        AND d.session_id = s.session_id AND d.incarnation = s.incarnation
                        AND d.source_label = s.source_label AND d.cwd IS s.cwd
                        AND d.parser_revision IN (?5, 63) AND d.blocked = 0
                          AND ({can_arrive})
                          AND (d.pending = 1 OR d.observed_generation <> s.source_generation)))
                   OR (s.environment_key = 'native' AND s.agent IN ('claude-code', 'codex', 'pi')
                     AND s.source_kind = 'file' AND EXISTS(SELECT 1 FROM native_file_delivery d
                       WHERE d.environment_key = s.environment_key AND d.agent = s.agent
                         AND d.session_id = s.session_id AND d.incarnation = s.incarnation
                         AND d.source_label = s.source_label AND d.cwd IS s.cwd
                         AND d.parser_revision IN (?5, 63) AND d.blocked = 0
                          AND ({can_arrive})
                           AND (d.pending = 1 OR d.observed_generation <> s.source_generation)))),
                  ({v2_status})
           FROM session s LEFT JOIN session_evidence e USING(environment_key, agent, session_id)
           WHERE s.environment_key = ?1 AND s.cwd = ?2
             AND COALESCE(s.updated_at_epoch, s.started_at_epoch) >= ?3
             AND COALESCE(s.updated_at_epoch, s.started_at_epoch) < ?4
              AND (({can_arrive}) OR s.updated_at_epoch >= ?11 OR EXISTS(SELECT 1 FROM burn_check_candidate c
               WHERE c.check_id = ?8 AND c.activation_scope_key = ?9 AND c.generation = ?10
                  AND c.session_key_json = json_array(s.environment_key, s.agent, s.session_id)
                  AND c.state IN ('pending', 'claimed', 'dispatching', 'answered'))
                 OR EXISTS(SELECT 1 FROM native_tool_result_publication p
                  JOIN burn_check_activation a ON a.check_id = ?8 AND a.activation_scope_key = ?9
                    AND a.generation = ?10 AND a.enabled = 1
                  WHERE p.environment_key = s.environment_key AND p.agent = s.agent
                     AND p.session_id = s.session_id AND p.publication_sequence > a.applied_sequence
                     AND {target}
                    AND (p.session_incarnation IS NULL OR
                       (p.session_incarnation = s.incarnation AND p.project_cwd = s.cwd)))
                 OR EXISTS(SELECT 1 FROM opencode_v2_source_status d
                   WHERE ({OPENCODE_V2_SOURCE_SQL}) AND d.environment_key = s.environment_key
                     AND d.agent = s.agent AND d.session_id = s.session_id
                     AND d.incarnation = s.incarnation
                     AND d.source_label = s.source_label AND d.cwd = s.cwd)
                 OR EXISTS(SELECT 1 FROM opencode_v2_capture d
                   WHERE ({OPENCODE_V2_SOURCE_SQL}) AND d.environment_key = s.environment_key
                     AND d.agent = s.agent AND d.session_id = s.session_id
                     AND d.incarnation = s.incarnation
                     AND d.source_label = s.source_label AND d.cwd = s.cwd))
             AND s.environment_key = 'native' AND s.wsl_distro IS NULL
             AND s.agent IN ('claude-code', 'codex', 'pi', 'opencode')
             AND NOT EXISTS(SELECT 1 FROM session_relation r
               WHERE r.environment_key = s.environment_key AND r.agent = s.agent
                 AND ((r.session_id = s.session_id AND r.kind = 'forkParent')
                   OR (r.related_id = s.session_id AND r.kind = 'subagent')))
          ORDER BY s.agent, s.session_id",
    ))?;
    statement
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
                request.computed_at_epoch - antiburn_local::discovery::ACTIVE_SESSION_WINDOW_SECS,
                request.computed_at_epoch,
            ],
            |row| {
                Ok((
                    CurrentSession {
                        key: SessionKey::new(
                            scope.environment.clone(),
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                        ),
                        ready: row.get(2)?,
                        checking: definition.decision_name.is_some() && row.get::<_, bool>(3)?,
                        data_can_arrive: row.get(4)?,
                        journal_owned: row.get(5)?,
                        catching_up: row.get(6)?,
                        v2_status: None,
                    },
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(anyhow::Error::from)?
        .into_iter()
        .map(|(mut session, code)| {
            session.v2_status = code
                .as_deref()
                .map(OpenCodeV2SourceStatus::from_code)
                .transpose()?;
            if session.v2_status.is_some() && !session.ready {
                session.catching_up = false;
                session.checking = false;
            }
            Ok(session)
        })
        .collect()
}

fn current_branches(
    connection: &Connection,
    definition: &InstalledCheckDefinition,
    request: &ReportRequest,
) -> Result<Vec<BranchResult>> {
    if !current_activation(connection, definition)? {
        return Ok(Vec::new());
    }
    let scope: ActivationScope = serde_json::from_str(&definition.activation_scope_key)?;
    let local_starts = if definition.decision_name.is_none() {
        local_started_branches(connection, definition, &scope)?
    } else {
        BTreeSet::new()
    };
    let ready = source_ready_at_sql(6, 7, 8, "?11");
    let model_attempts = i32::from(definition.decision_name.is_some());
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
                  st.assessment_id, h.actions_json,
                  original.frozen_input_json IS NOT NULL AND original.frozen_input_json <> '{{}}',
                 CASE WHEN latest.outcome <> 'assessed' OR latest.evidence_valid = 1
                       THEN latest.outcome END, latest.actions_json,
                    CASE WHEN {model_attempts} = 0 AND latest.assessment_id IS NULL
                           AND c.state = 'answered' THEN c.preparation_json
                         WHEN {model_attempts} = 1 AND latest.outcome = 'assessed'
                           AND latest_candidate.frozen_input_json = '{{}}' THEN NULL
                         ELSE latest_candidate.preparation_json END, latest_candidate.diagnostic_json,
                  EXISTS(SELECT 1 FROM burn_check_candidate started
                    WHERE started.check_id = c.check_id AND started.activation_scope_key = c.activation_scope_key
                      AND started.generation = c.generation AND started.branch_key = c.branch_key
                      AND started.session_key_json = c.session_key_json
                      AND started.state NOT IN ('stale', 'coalesced')
                      AND (EXISTS(SELECT 1 FROM burn_check_assessment_history history
                        WHERE history.candidate_id = started.candidate_id
                          AND history.check_id = started.check_id
                          AND history.activation_scope_key = started.activation_scope_key
                          AND history.generation = started.generation AND history.branch_key = started.branch_key
                          AND ((history.outcome = 'assessed' AND history.evidence_valid = 1)
                            OR history.outcome IN ('execution_failed', 'not_assessed')))
                        OR (NOT EXISTS(SELECT 1 FROM burn_check_assessment_history history
                              WHERE history.candidate_id = started.candidate_id AND history.outcome <> 'cancelled')
                           AND EXISTS(SELECT 1 FROM burn_check_evaluation_attempt attempt
                             WHERE attempt.candidate_id = started.candidate_id
                               AND {model_attempts} = 1
                               AND attempt.state IN ('dispatch_intent', 'settled'))))
                      AND NOT EXISTS(SELECT 1 FROM burn_check_candidate_dependency d
                        LEFT JOIN burn_check_observation o ON o.check_id = started.check_id
                          AND o.activation_scope_key = started.activation_scope_key
                          AND o.observation_id = d.observation_id AND o.session_key_json = d.session_key_json
                        WHERE d.candidate_id = started.candidate_id
                          AND (o.source_format IS NOT d.source_format OR o.native_revision IS NOT d.native_revision
                             OR o.evidence_version IS NOT d.evidence_version OR o.removed IS NOT 0 OR d.removed <> 0))),
                   ({ready}),
                   (SELECT attempt.decision_name FROM burn_check_evaluation_attempt attempt
                     WHERE attempt.candidate_id = latest_candidate.candidate_id
                       AND attempt.decision_name <> '' ORDER BY attempt.rowid DESC LIMIT 1)
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
           LEFT JOIN burn_check_candidate latest_candidate ON latest_candidate.candidate_id = latest.candidate_id
          WHERE c.rank = 1 AND s.environment_key = ?9 AND s.cwd = ?10
           AND COALESCE(s.updated_at_epoch, s.started_at_epoch) >= ?4
           AND COALESCE(s.updated_at_epoch, s.started_at_epoch) < ?5
             AND (({ready}) OR ({OPENCODE_V2_SOURCE_SQL}))
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
                scope.environment,
                scope.session_cwd,
                request.computed_at_epoch,
            ],
            |row| {
                let branch: String = row.get(0)?;
                let agent: String = row.get(2)?;
                let session_id: String = row.get(3)?;
                let local_started = local_starts.contains(&(
                    branch.clone(),
                    scope.environment.clone(),
                    agent.clone(),
                    session_id.clone(),
                ));
                Ok(BranchResult {
                    branch,
                    checking: definition.availability == InstalledCheckAvailability::Available
                        && if definition.decision_name.is_none() {
                            local_started
                        } else {
                            row.get::<_, bool>(1)?
                        },
                    agent,
                    session_id,
                    incarnation: row.get(4)?,
                    observed_epoch: row.get(5)?,
                    status: row.get(6)?,
                    assessment: row.get(7)?,
                    actions: row.get(8)?,
                    has_frozen: row.get(9)?,
                    latest_outcome: row.get(10)?,
                    latest_actions: row.get(11)?,
                    latest_preparation: row.get(12)?,
                    latest_diagnostic: row.get(13)?,
                    has_started: row.get::<_, bool>(14)? || local_started,
                    source_ready: row.get(15)?,
                    latest_decision: row.get(16)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn local_started_branches(
    connection: &Connection,
    definition: &InstalledCheckDefinition,
    scope: &ActivationScope,
) -> Result<BTreeSet<(String, String, String, String)>> {
    let mut statement = connection.prepare(
        "SELECT c.branch_key, c.trigger_position, c.frozen_input_json, c.preparation_json,
                c.answer_json, c.session_key_json
         FROM burn_check_candidate c
         WHERE c.check_id = ?1 AND c.activation_scope_key = ?2 AND c.generation = ?3
           AND c.state = 'answered' AND c.claim_owner IS NOT NULL
           AND c.claim_owner <> '' AND c.claim_fence > 0
           AND NOT EXISTS(SELECT 1 FROM burn_check_evaluation_attempt a WHERE a.candidate_id = c.candidate_id)
           AND NOT EXISTS(SELECT 1 FROM burn_check_assessment_history h
             WHERE h.candidate_id = c.candidate_id AND h.outcome <> 'cancelled')
           AND NOT EXISTS(SELECT 1 FROM burn_check_candidate_dependency d
             LEFT JOIN burn_check_observation o ON o.check_id = c.check_id
               AND o.activation_scope_key = c.activation_scope_key
               AND o.observation_id = d.observation_id AND o.session_key_json = d.session_key_json
             WHERE d.candidate_id = c.candidate_id
               AND (o.source_format IS NOT d.source_format OR o.native_revision IS NOT d.native_revision
                 OR o.evidence_version IS NOT d.evidence_version OR o.removed IS NOT 0 OR d.removed <> 0))",
    )?;
    let rows = statement.query_map(
        params![
            definition.check_id,
            definition.activation_scope_key,
            definition.generation
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        },
    )?;
    let mut started = BTreeSet::new();
    for row in rows {
        let (branch, position, frozen, preparation, answer, session) = row?;
        let Some(preparation) = preparation else {
            continue;
        };
        let Ok(PreparationRecord::Local(preparation)) =
            antiburn_checks::preparation::decode_preparation_record(preparation.as_bytes())
        else {
            continue;
        };
        if preparation.validate_for_resume().is_err() {
            continue;
        }
        let Some(answer) = answer else { continue };
        let Ok(answer) = antiburn_checks::results::decode_results(answer.as_bytes())
            .and_then(|answer| answer.validate_for_declarations(&[], &[]))
        else {
            continue;
        };
        if answer != antiburn_checks::results::EvaluatedAssessment::local() {
            continue;
        }
        let Ok(candidate) = serde_json::from_str::<FrozenCandidate>(&frozen) else {
            continue;
        };
        let Ok((environment, agent, session_id)) =
            serde_json::from_str::<(String, String, String)>(&session)
        else {
            continue;
        };
        if candidate.activation.scope != *scope
            || candidate.activation.definition_id != definition.check_id
            || Some(candidate.activation.generation.0)
                != definition
                    .generation
                    .and_then(|value| u64::try_from(value).ok())
            || candidate.branch.0 != branch
            || candidate.trigger_position.0 != position
            || environment != scope.environment
            || agent.is_empty()
            || session_id.is_empty()
        {
            continue;
        }
        started.insert((branch, environment, agent, session_id));
    }
    Ok(started)
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
    SkippedInputUnavailable,
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
            | StoredActionOutcome::SkippedConditionUnavailable
            | StoredActionOutcome::SkippedInputUnavailable => {}
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
    Ok(!matches!(
        definition.availability,
        InstalledCheckAvailability::Invalid { .. }
    ) && connection.query_row(
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
    )?)
}

fn opencode_v2_diagnostic(
    status: OpenCodeV2SourceStatus,
    ready: bool,
) -> Option<InstalledCheckDiagnostic> {
    use OpenCodeV2SourceStatus::*;
    let (code, title, message) = match status {
        Live => return None,
        ServiceUnavailable => (
            "opencode-v2-service-unavailable",
            "Open OpenCode to resume live checks",
            "The registered local OpenCode V2 service is unavailable. Open OpenCode for this project, then select Rescan in Sessions settings.",
        ),
        AuthenticationRejected => (
            "opencode-v2-authentication-rejected",
            "Reconnect the OpenCode V2 service",
            "OpenCode rejected the local service credential. Restart OpenCode, then select Rescan in Sessions settings.",
        ),
        SourceMismatch => (
            "opencode-v2-source-mismatch",
            "Check the OpenCode V2 source",
            "antiburn could not verify that the local service uses this session's database. Open this project in the service for its local database, then select Rescan in Sessions settings.",
        ),
        VersionUnsupported => (
            "opencode-v2-version-unsupported",
            "Use a supported OpenCode V2 version",
            "This service, database, or event version is not supported. Update antiburn or use a supported OpenCode V2 version.",
        ),
        CaptureGap => (
            "opencode-v2-capture-gap",
            "OpenCode V2 capture missed activity",
            "Live checks cannot replay missed events. Keep OpenCode and antiburn running to capture future activity. Earlier assessments cover only their saved input.",
        ),
        SourceLimit => (
            "opencode-v2-source-limit",
            "Reduce the OpenCode V2 capture size",
            "Live capture reached a local source limit. Close unused active sessions or start a smaller session, then select Rescan in Sessions settings.",
        ),
    };
    Some(crate::installed_checks::diagnostics::payload(
        code,
        if ready {
            InstalledCheckDiagnosticSeverity::Info
        } else {
            InstalledCheckDiagnosticSeverity::Error
        },
        title,
        &format!("{message} Stored-session analysis does not require this live connection."),
        None,
        Vec::new(),
    ))
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
    let mut diagnostics = DiagnosticGroups::default();
    let mut waiting_for_events: Option<InstalledCheckWaitingFor> = None;
    let mut has_started = false;
    for definition in definitions {
        let mut blocked_sources = BTreeSet::new();
        for diagnostic in &definition.diagnostics {
            diagnostics.add(diagnostic.clone(), None)?;
        }
        if definition.availability == InstalledCheckAvailability::Available
            && current_activation(connection, definition)?
        {
            let active = current_sessions(connection, definition, request)?;
            let waiting = match waiting_for_trigger(
                connection,
                definition,
                &active,
                &mut diagnostics,
            ) {
                Ok(waiting) => waiting,
                Err(error) => {
                    ::tracing::error!(event = "installed_check_trigger_read_failed", error = %error);
                    diagnostics.add(storage_diagnostic(), None)?;
                    None
                }
            };
            if let Some(waiting) = waiting {
                waiting_for_events = Some(match waiting_for_events {
                    Some(previous) => {
                        let catching_up = previous.catching_up || waiting.catching_up;
                        let mut selected = if waiting.count < previous.count {
                            waiting
                        } else {
                            previous
                        };
                        selected.catching_up = catching_up;
                        selected
                    }
                    None => waiting,
                });
            }
            for session in active {
                if session.v2_status.is_some() && !session.ready {
                    blocked_sources
                        .insert((session.key.agent.clone(), session.key.session_id.clone()));
                }
                if let Some(status) = session.v2_status {
                    if let Some(diagnostic) = opencode_v2_diagnostic(status, session.ready) {
                        diagnostics.add(
                            diagnostic,
                            Some((session.key.agent.clone(), session.key.session_id.clone())),
                        )?;
                    }
                } else if !session.ready && !session.data_can_arrive {
                    let unsafe_admission =
                        crate::store::native_tool_results::admission::source_reason_in(
                            connection,
                            &session.key,
                        )?
                        .is_some();
                    let (title, message) = if unsafe_admission {
                        (
                            "Session records are unsafe",
                            "antiburn cannot prove record identity, order, or session ownership. Start a new supported agent session to run checks on new work.",
                        )
                    } else if session.journal_owned {
                        (
                            "Session journal is unavailable",
                            "The current session journal cannot provide usable check evidence. Check source access in Settings → Sources or start a new supported agent session.",
                        )
                    } else {
                        (
                            "Session source is unavailable",
                            "antiburn cannot read supported evidence from this session. Check source access in Settings → Sources and refresh the session index.",
                        )
                    };
                    diagnostics.add(
                        crate::installed_checks::diagnostics::payload(
                            if unsafe_admission {
                                "source-unsafe-admission"
                            } else {
                                "source-unavailable"
                            },
                            InstalledCheckDiagnosticSeverity::Error,
                            title,
                            message,
                            definition.definition_sources.first().cloned(),
                            Vec::new(),
                        ),
                        Some((session.key.agent.clone(), session.key.session_id.clone())),
                    )?;
                }
                let result = sessions
                    .entry((session.key.agent, session.key.session_id))
                    .or_default();
                result.ready |= session.ready;
                result.waiting_for_data |= session.catching_up;
                result.checking |= session.checking;
            }
        }
        for branch in current_branches(connection, definition, request)? {
            let source_blocked =
                blocked_sources.contains(&(branch.agent.clone(), branch.session_id.clone()));
            has_started |= branch.has_started && !source_blocked;
            let runtime_diagnostics = runtime_diagnostics(definition, &branch).unwrap_or_else(|error| {
                ::tracing::error!(event = "installed_check_diagnostic_read_failed", error = %error);
                vec![storage_diagnostic()]
            });
            for diagnostic in runtime_diagnostics {
                diagnostics.add(
                    diagnostic,
                    Some((branch.agent.clone(), branch.session_id.clone())),
                )?;
            }
            let counts = sessions
                .entry((branch.agent.clone(), branch.session_id.clone()))
                .or_default();
            counts.finding |= branch.status.as_deref() == Some("failure");
            counts.clean |= branch.source_ready && branch.status.as_deref() == Some("success");
            counts.checking |= branch.checking && !source_blocked;
            counts.review_failed |= matches!(
                branch.latest_outcome.as_deref(),
                Some("execution_failed" | "not_assessed")
            );
            if branch.latest_outcome.as_deref() == Some("assessed") {
                match branch
                    .latest_actions
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<StoredPlan>(json).ok())
                {
                    Some(plan) => {
                        counts.reviewed_without_status |= !plan.outcomes.iter().any(|action| {
                            matches!(action.outcome, StoredActionOutcome::Applied { .. })
                        })
                    }
                    None => diagnostics.add(
                        storage_diagnostic(),
                        Some((branch.agent.clone(), branch.session_id.clone())),
                    )?,
                }
            }
            if let Some(status) = branch
                .status
                .as_deref()
                .filter(|status| *status == "failure" || branch.source_ready)
            {
                let message = match status_message(branch.actions.as_deref(), status) {
                    Ok(message) => message,
                    Err(_) => {
                        diagnostics.add(
                            storage_diagnostic(),
                            Some((branch.agent.clone(), branch.session_id.clone())),
                        )?;
                        continue;
                    }
                };
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
                        evidence_available: branch.has_frozen,
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
        diagnostic: None,
        diagnostics: diagnostics.finish(),
        explanation: (!explanation.is_empty()).then_some(explanation),
        finding,
        clean,
        unavailable,
        review_failed,
        reviewed_without_status,
        has_started: has_started || checking_count > 0,
        checking_count,
        waiting_for_data_count,
        waiting_for_results_count,
        waiting_for_events,
        agents,
        targets,
        targets_truncated,
    })
}

fn waiting_for_trigger(
    connection: &Connection,
    definition: &InstalledCheckDefinition,
    sessions: &[CurrentSession],
    diagnostics: &mut DiagnosticGroups,
) -> Result<Option<InstalledCheckWaitingFor>> {
    let Some(trigger) = definition.trigger else {
        return Ok(None);
    };
    let checkpoint: Option<String> = connection.query_row(
        "SELECT checkpoint_json FROM burn_check_activation
         WHERE check_id = ?1 AND activation_scope_key = ?2",
        params![definition.check_id, definition.activation_scope_key],
        |row| row.get(0),
    )?;
    let progress = checkpoint
        .as_deref()
        .map(antiburn_checks::check_store::SourceProgress::decode)
        .transpose()
        .map_err(|error| anyhow::anyhow!(error.message))?
        .unwrap_or_default();
    for session in sessions {
        let branch = antiburn_checks::domain::BranchId(session.key.to_identity_json());
        if let Some(diagnostic) = progress
            .selection
            .branches
            .get(&branch)
            .and_then(|branch| branch.last_filter_diagnostic.as_ref())
            .and_then(filter_diagnostic)
        {
            diagnostics.add(
                diagnostic,
                Some((session.key.agent.clone(), session.key.session_id.clone())),
            )?;
        }
    }
    let minimum = usize::from(trigger.minimum);
    let catching_up = sessions.iter().any(|session| session.catching_up);
    let count = sessions
        .iter()
        .filter(|session| session.ready && !session.catching_up)
        .map(|session| {
            let selected = progress
                .selection
                .branches
                .get(&antiburn_checks::domain::BranchId(
                    session.key.to_identity_json(),
                ))
                .map_or(0, |branch| branch.results.len());
            minimum.saturating_sub(selected).max(1)
        })
        .min()
        .or_else(|| catching_up.then_some(minimum));
    let Some(count) = count else {
        return Ok(None);
    };
    Ok(Some(InstalledCheckWaitingFor {
        event: trigger.event,
        count: u8::try_from(count).context("trigger event count exceeds its limit")?,
        catching_up,
    }))
}

fn filter_diagnostic(
    diagnostic: &antiburn_checks::filters::FilterDiagnostic,
) -> Option<InstalledCheckDiagnostic> {
    use antiburn_checks::filters::{FilterField, FilterReason};

    let field = match diagnostic.field {
        FilterField::Model => "models",
        FilterField::FastMode => "fast_mode",
        FilterField::Paths => "paths",
        FilterField::PathsIgnore => "paths-ignore",
    };
    let (code, title, message) = match diagnostic.reason {
        FilterReason::Unavailable => (
            "filter-evidence-unavailable",
            "Recorded filter evidence is unavailable",
            "The source did not provide the field required by this filter. Review the check's filters and supported sources. Current settings do not replace missing recorded evidence.",
        ),
        FilterReason::Incomplete => (
            "filter-evidence-incomplete",
            "Recorded paths are incomplete",
            "The known paths cannot establish this filter. An ignore filter needs complete paths to rule out a veto. Review the check's path filters and supported sources.",
        ),
        FilterReason::Invalid => (
            "filter-evidence-invalid",
            "Recorded filter evidence is invalid",
            "The source field cannot safely satisfy this filter. Review source support or use a filter supported by the recorded evidence.",
        ),
        FilterReason::LimitExceeded => (
            "filter-work-limit",
            "Simplify the path filters",
            "Path matching exceeded its local work limit. Reduce the number or size of path patterns in the check file.",
        ),
        FilterReason::KnownEmpty | FilterReason::NoMatch | FilterReason::IgnoredPath => {
            return None;
        }
    };
    Some(crate::installed_checks::diagnostics::payload(
        code,
        InstalledCheckDiagnosticSeverity::Info,
        title,
        message,
        None,
        vec![crate::installed_checks::diagnostics::detail(
            "Filter", field,
        )],
    ))
}

/// Mint existing session navigation cards after the report read releases its lock.
pub(crate) fn enrich_samples(
    state: &crate::main_window::MainWindowState,
    store: &Store,
    rows: &mut [InstalledCheckReportPayload],
    now_epoch: i64,
) -> Result<()> {
    let repositories = store.repositories()?;
    let mut samples = BTreeMap::<SessionKey, BurnCheckSampleSession>::new();
    for row in rows.iter_mut() {
        for target in &mut row.targets {
            let key = SessionKey::new(
                target.session.environment_key.clone(),
                target.session.agent.clone(),
                target.session.session_id.clone(),
            );
            samples.entry(key).or_insert_with(|| target.session.clone());
        }
    }
    let keyed_payloads = crate::main_window::sample_payloads_with_keys_from_store(
        state,
        store,
        &repositories,
        &samples.into_values().collect::<Vec<_>>(),
        now_epoch,
    )
    .map_err(anyhow::Error::msg)?
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    for row in rows.iter_mut() {
        for target in &mut row.targets {
            let key = SessionKey::new(
                target.session.environment_key.clone(),
                target.session.agent.clone(),
                target.session.session_id.clone(),
            );
            target.samples = keyed_payloads.get(&key).cloned().into_iter().collect();
        }
    }
    Ok(())
}

/// Returns frozen local context, saved model input, or legacy source for the exact status.
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
    let Some(_branch) = current_branches(&transaction, definition, request)?
        .into_iter()
        .find(|branch| {
            branch.branch == locator.branch
                && branch.status.as_deref() == Some("failure")
                && branch.assessment.as_deref() == Some(&locator.assessment)
        })
    else {
        return Ok(unavailable());
    };
    let Some(frozen) = transaction
        .query_row(
            "SELECT c.frozen_input_json FROM burn_check_assessment_history h
               JOIN burn_check_candidate c ON c.candidate_id = h.candidate_id
              WHERE h.assessment_id = ?1",
            [&locator.assessment],
            |row| row.get::<_, Option<String>>(0),
        )?
        .filter(|value| value != "{}")
    else {
        return Ok(unavailable());
    };
    let preparation: Option<String> = transaction.query_row(
        "SELECT c.preparation_json FROM burn_check_assessment_history h
           JOIN burn_check_candidate c ON c.candidate_id = h.candidate_id
          WHERE h.assessment_id = ?1",
        [&locator.assessment],
        |row| row.get(0),
    )?;
    if let Some(preparation) = preparation {
        let preparation = match preparation::decode(preparation.as_bytes(), true) {
            Ok(preparation) => preparation,
            Err(_) => return Ok(unavailable()),
        };
        let prepared = match preparation {
            ReportPreparation::Local(prepared) => {
                if definition.decision_name.is_some() || prepared.validate_for_resume().is_err() {
                    return Ok(unavailable());
                }
                let Ok(candidate) = serde_json::from_str::<FrozenCandidate>(&frozen) else {
                    return Ok(unavailable());
                };
                let mut item = evidence_item(
                    BurnCheckEvidenceLabel::Context,
                    "Frozen local context",
                    "local-context".into(),
                    &candidate.context.value,
                );
                item.explanation = "These frozen values were available to the local status actions. No model request was sent.".into();
                return Ok(BurnCheckTargetEvidence {
                    status: BurnCheckEvidenceStatus::Available,
                    items: vec![item],
                    comparison: None,
                    occurrences: Vec::new(),
                });
            }
            ReportPreparation::Model(prepared) if definition.decision_name.is_some() => prepared,
            ReportPreparation::Model(_) | ReportPreparation::Rejected(_) => {
                return Ok(unavailable());
            }
        };
        let mut items = Vec::new();
        for block in &prepared.blocks {
            let reference = |kind: &str| match &block.decision {
                Some(decision) => format!("{kind}:{decision}"),
                None => kind.into(),
            };
            let title = |kind: &str| match &block.decision {
                Some(decision) => format!("{kind}: {decision}"),
                None => kind.into(),
            };
            if let Some(original) = &block.original_input {
                let mut item = evidence_item(
                    BurnCheckEvidenceLabel::Context,
                    &title("Original input"),
                    reference("original-input"),
                    original,
                );
                item.explanation = "These are the saved decision input values before fitting. They are not a separate model request.".into();
                items.push(item);
            }
            let mut input = evidence_item(
                BurnCheckEvidenceLabel::Context,
                &title("Assessed input"),
                reference("assessed-input"),
                &block.input,
            );
            input.explanation =
                "These are the decision input values used for this assessment.".into();
            if !block.report.omissions.is_empty() {
                input.limitation = Some("Input was shortened under the check's fitting rules. Omitted records and text were not assessed.".into());
            }
            items.push(input);
            if block.original_input.is_some() || !block.report.omissions.is_empty() {
                let mut omissions = evidence_item(
                    BurnCheckEvidenceLabel::Context,
                    &title("Input omissions"),
                    reference("input-omissions"),
                    &serde_json::to_value(&block.report)?,
                );
                omissions.explanation = "Counts use records for recent and original UTF-8 bytes for truncate. Retained byte counts exclude the omission marker.".into();
                items.push(omissions);
            }
        }
        if let Some(provenance) =
            crate::jev::decision_evaluator::prepared_profile_provenance(&prepared.profile)
        {
            let mut connection = evidence_item(
                BurnCheckEvidenceLabel::Context,
                "Decision connection",
                "decision-connection".into(),
                &serde_json::json!({
                    "provider": provider_name(provenance.provider),
                    "model": provenance.model,
                    "modelRevision": provenance.model_revision,
                    "connectionRevision": provenance.connection_revision,
                }),
            );
            connection.explanation = "This connection produced the saved assessment. Current Settings can use a different connection.".into();
            items.push(connection);
        }
        return Ok(BurnCheckTargetEvidence {
            status: BurnCheckEvidenceStatus::Available,
            items,
            comparison: None,
            occurrences: Vec::new(),
        });
    }
    if definition.decision_name.is_none() {
        return Ok(unavailable());
    }
    let Ok(candidate) = serde_json::from_str::<FrozenCandidate>(&frozen) else {
        return Ok(unavailable());
    };
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
                "Original source: requested task",
                format!("request:{index}"),
                request,
            ));
        }
    }
    for message in &candidate.messages {
        items.push(evidence_item(
            BurnCheckEvidenceLabel::ObservedAction,
            "Original source: action result",
            message.id.clone(),
            &message.value,
        ));
    }
    for item in &mut items {
        item.limitation = Some("Exact assessed input was not saved for this assessment. This is original source evidence; it does not show which values were sent to the model.".into());
    }
    Ok(BurnCheckTargetEvidence {
        status: BurnCheckEvidenceStatus::Available,
        items,
        comparison: None,
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
pub(crate) mod tests;
