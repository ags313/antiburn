//! Desktop lifetime for Burn Check definitions. Core owns execution policy.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use antiburn_checks::check_store::{AttemptFailure, CheckStore, StoreError};
use antiburn_checks::definition_lifecycle::{
    DefinitionLifecycle, EffectiveActivation, InactiveKind, LifecycleState, LifecycleTransition,
};
use antiburn_checks::diagnostics::DiagnosticCode;
use antiburn_checks::discovery::{
    CheckFile, DefinitionScope, DefinitionState, DiscoveryConfig, EffectiveDefinition,
    discover_definitions,
};
use antiburn_checks::domain::{ActivationScope, ObservationPosition, RunScope};
use antiburn_checks::engine::{
    CheckEngine, DecisionEvaluator, DecisionFuture, EvaluationFailure, RunOutcome, RunSettlement,
    StartOutcome,
};
use antiburn_checks::source::FileEventSource;
use antiburn_local::paths::protected;
use anyhow::{Context, Result, ensure};
use tauri::{Emitter, Manager};
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::check_store::StoreCheckStore;
use crate::insights_report::installed_checks::{
    InstalledCheckAvailability, InstalledCheckDefinition, InstalledCheckScope, InstalledCheckSource,
};
use crate::jev::config::SystemOneProvider;
use crate::jev::decision_evaluator::TypeSafeDecisionEvaluator;
use crate::published_evidence_reader::StorePublishedEvidenceReader;
use crate::store::Store;

mod diagnostics;
mod loader;
mod watch;

const MAX_DEFINITIONS: usize = 128;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const SOURCE_PAGE_SIZE: u16 = 64;
const SOURCE_PAGES_PER_PASS: usize = 4;
const EVALUATIONS: usize = 4;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);

/// Only metadata crosses the worker boundary. Engines and credentials stay private.
#[derive(Default)]
pub(crate) struct InstalledChecks {
    metadata: RwLock<Vec<InstalledCheckDefinition>>,
    wake: Notify,
    stopping: AtomicBool,
    credentials_restored: AtomicBool,
    requested_turn: AtomicU64,
    completed_turn: AtomicU64,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

pub(crate) fn metadata(app: &tauri::AppHandle) -> Vec<InstalledCheckDefinition> {
    app.try_state::<InstalledChecks>()
        .map(|state| {
            state
                .metadata
                .read()
                .expect("installed metadata lock")
                .clone()
        })
        .unwrap_or_default()
}

pub(crate) fn wake(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<InstalledChecks>() {
        state.requested_turn.fetch_add(1, Ordering::AcqRel);
        state.wake.notify_one();
    }
}

pub(crate) fn has_priority_work(app: &tauri::AppHandle) -> bool {
    app.try_state::<InstalledChecks>().is_some_and(|state| {
        state.requested_turn.load(Ordering::Acquire) > state.completed_turn.load(Ordering::Acquire)
    })
}

pub(crate) fn credentials_restored(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<InstalledChecks>() {
        state.credentials_restored.store(true, Ordering::Release);
        state.wake.notify_one();
    }
}

/// Stop new work and let numbered attempts settle. The final bound preserves
/// unresolved dispatch intents for Store recovery instead of permitting a resend.
pub(crate) fn shutdown(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<InstalledChecks>() else {
        return;
    };
    state.stopping.store(true, Ordering::Release);
    state.wake.notify_one();
    let task = state.task.lock().expect("installed task lock").take();
    if let Some(mut task) = task {
        tauri::async_runtime::block_on(async {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Admission {
    Ready,
    Paused(String),
    Unsupported(String),
}

fn admission(app: &tauri::AppHandle) -> Admission {
    let worker = app.state::<crate::jev::worker::WorkerHandle>();
    if worker.system_one_connection().provider != SystemOneProvider::Jev {
        Admission::Unsupported("Burn Check definitions require the Jev connection.".into())
    } else if !worker.is_available() {
        Admission::Paused("Enable Smart Burn Checks and configure Jev in Settings → Checks.".into())
    } else {
        Admission::Ready
    }
}

struct InstalledDecisionEvaluator {
    app: tauri::AppHandle,
}

impl DecisionEvaluator for InstalledDecisionEvaluator {
    fn evaluate<'a>(
        &'a self,
        activation: &'a EffectiveActivation,
        candidate: &'a antiburn_checks::domain::FrozenCandidate,
        input: &'a serde_json::Value,
        attempt_number: u8,
    ) -> DecisionFuture<'a> {
        Box::pin(async move {
            let worker = self.app.state::<crate::jev::worker::WorkerHandle>();
            let (client, generation) = worker
                .execution_client()
                .filter(|_| admission(&self.app) == Admission::Ready)
                .ok_or_else(|| EvaluationFailure {
                    kind: AttemptFailure::Permanent,
                    detail: "Burn Check admission changed before evaluation.".into(),
                    attempts: 0,
                })?;
            let app = self.app.clone();
            let activation_id = activation.id.clone();
            let project_key = StoreCheckStore::project_key(&activation.id.scope);
            let is_cancelled = move || {
                let worker = app.state::<crate::jev::worker::WorkerHandle>();
                let state = app.state::<InstalledChecks>();
                state.stopping.load(Ordering::Acquire)
                    || !worker.key_is_current(generation)
                    || admission(&app) != Admission::Ready
                    || !state
                        .metadata
                        .read()
                        .expect("installed metadata lock")
                        .iter()
                        .any(|definition| {
                            definition.check_id == activation_id.definition_id
                                && definition.project_key == project_key
                                && definition.generation
                                    == i64::try_from(activation_id.generation.0).ok()
                                && matches!(
                                    definition.availability,
                                    InstalledCheckAvailability::Available
                                )
                        })
            };
            if is_cancelled() {
                return Err(EvaluationFailure {
                    kind: AttemptFailure::Permanent,
                    detail: "Burn Check admission changed before evaluation.".into(),
                    attempts: 0,
                });
            }
            let usage_store = self.app.state::<Store>().inner().clone();
            let rejection_app = self.app.clone();
            let rejection_store = usage_store.clone();
            let rejection_generation = generation;
            let check_id = activation.id.definition_id.clone();
            let evaluator = TypeSafeDecisionEvaluator::new(client)
                .with_cancellation(is_cancelled)
                .with_authentication_rejection(move || {
                    crate::jev::worker::settle_authentication_rejection(
                        &rejection_app,
                        &rejection_store,
                        &rejection_app.state::<crate::jev::worker::WorkerHandle>(),
                        rejection_generation,
                        true,
                    )
                    .map_err(|error| error.to_string())
                })
                .with_usage(Arc::new(move |candidate_id, attempt_number, usage| {
                    usage_store
                        .record_burn_check_decision_usage(
                            candidate_id,
                            attempt_number,
                            &check_id,
                            usage,
                            time::OffsetDateTime::now_utc().unix_timestamp(),
                        )
                        .map_err(|_| "Could not record Burn Check provider usage.".to_owned())
                }));
            evaluator
                .evaluate(activation, candidate, input, attempt_number)
                .await
        })
    }
}

struct ProjectChecks<E> {
    store: Arc<StoreCheckStore>,
    engine: Arc<CheckEngine<StoreCheckStore, E>>,
    active: Vec<EffectiveActivation>,
}

/// The production helper also runs in deterministic Store/source integration tests.
struct InstalledCheckHost<E> {
    store: Arc<Store>,
    evaluator: Arc<E>,
    owner: String,
    global_directory: Option<PathBuf>,
    watch_directories: Vec<PathBuf>,
    diagnostic_log: diagnostics::DiagnosticLog,
    projects: BTreeMap<ActivationScope, ProjectChecks<E>>,
    definitions: Vec<InstalledCheckDefinition>,
    source_turn: usize,
    evaluation_turn: usize,
    live_since_epoch: i64,
}

impl<E: DecisionEvaluator + 'static> InstalledCheckHost<E> {
    fn new(
        store: Arc<Store>,
        evaluator: Arc<E>,
        global_directory: Option<PathBuf>,
    ) -> Result<Self> {
        let mut owner = [0_u8; 16];
        getrandom::fill(&mut owner).context("create installed check owner")?;
        Ok(Self {
            store,
            evaluator,
            owner: hex(&owner),
            global_directory,
            watch_directories: Vec::new(),
            diagnostic_log: diagnostics::DiagnosticLog::default(),
            projects: BTreeMap::new(),
            definitions: Vec::new(),
            source_turn: 0,
            evaluation_turn: 0,
            live_since_epoch: time::OffsetDateTime::now_utc().unix_timestamp()
                - antiburn_local::discovery::ACTIVE_SESSION_WINDOW_SECS,
        })
    }

    fn reconcile(&mut self, admission: &Admission) -> Result<()> {
        let scopes = live_projects(&self.store)?;
        let repositories = self
            .store
            .repositories()?
            .into_iter()
            .filter(|repository| repository.wsl_distro.is_none())
            .filter_map(|repository| {
                repository
                    .repo_root
                    .map(|root| (PathBuf::from(root), repository.enabled))
            })
            .filter(|(root, _)| root.is_absolute())
            .collect::<BTreeMap<_, _>>();
        let mut granted = self.store.granted_dirs()?;
        let mut directories = BTreeMap::new();
        let mut errors = diagnostics::DiagnosticBatch::new();
        self.watch_directories.clear();
        for (directory, enabled) in self
            .global_directory
            .iter()
            .cloned()
            .map(|path| (path, true))
            .chain(
                repositories
                    .iter()
                    .map(|(root, enabled)| (root.join(".antiburn/checks"), *enabled)),
            )
        {
            let checks = if enabled {
                loader::load_directory(&self.store, &directory, &granted)
            } else {
                loader::unavailable(&directory, "Repository is disabled.")
            };
            granted = self.store.granted_dirs()?;
            if enabled && !protected::cwd_resolution_blocked(&directory, &granted) {
                self.watch_directories.push(directory.clone());
            }
            diagnostics::collect(&mut errors, checks.error.as_slice());
            directories.insert(directory, checks);
        }
        let mut discovered = Vec::new();
        let mut definition_count = 0;
        let mut represented_roots = BTreeSet::new();
        for scope in &scopes {
            if !self.projects.contains_key(scope) {
                let store = Arc::new(StoreCheckStore::new(
                    Arc::clone(&self.store),
                    self.owner.clone(),
                    scope.clone(),
                )?);
                let engine = Arc::new(CheckEngine::new(
                    Arc::clone(&store),
                    Arc::clone(&self.evaluator),
                ));
                self.projects.insert(
                    scope.clone(),
                    ProjectChecks {
                        store,
                        engine,
                        active: Vec::new(),
                    },
                );
            }
            let ids = self.projects[scope]
                .store
                .definition_ids()?
                .into_iter()
                .collect::<BTreeSet<_>>();
            let root = if scope.environment == "native" {
                repositories
                    .keys()
                    .filter(|root| Path::new(&scope.project).starts_with(root))
                    .max_by_key(|root| root.components().count())
                    .cloned()
            } else {
                None
            };
            represented_roots.extend(root.iter().cloned());
            let config = DiscoveryConfig {
                global_checks_directory: self.global_directory.clone(),
                project_root: root,
            };
            let files = definition_files(&config, &directories, &ids);
            let result = discover_definitions(&config, &files);
            diagnostics::collect(&mut errors, &result.diagnostics);
            let definitions = result.definitions;
            definition_count += definitions.len();
            ensure!(
                definition_count <= MAX_DEFINITIONS,
                "installed checks exceed the {MAX_DEFINITIONS}-definition report limit"
            );
            discovered.push((scope.clone(), definitions));
        }
        let mut metadata = Vec::new();
        for (scope, definitions) in discovered {
            let project = self.projects.get_mut(&scope).expect("project inserted");
            let head = project.store.publication_head()?;
            let ids = project
                .store
                .definition_ids()?
                .into_iter()
                .chain(definitions.iter().map(|definition| definition.id.clone()))
                .collect::<BTreeSet<_>>();
            project.active.clear();
            for id in ids {
                let definition = definitions.iter().find(|definition| definition.id == id);
                let effective =
                    definition.map(|definition| admitted_definition(definition, admission));
                let compiled = effective
                    .as_ref()
                    .and_then(|definition| match &definition.state {
                        DefinitionState::Ready(compiled) => Some(Arc::new((**compiled).clone())),
                        _ => None,
                    });
                let previous = project.store.load_lifecycle(&id, compiled)?;
                let transition = DefinitionLifecycle.reconcile_scoped(
                    &previous,
                    effective.as_ref(),
                    Some(PathBuf::from(&scope.project)),
                    scope.clone(),
                    ObservationPosition(0),
                    head,
                );
                let next = previous.apply(&transition);
                let active = match &next {
                    LifecycleState::Active(active) => Some(active),
                    _ => None,
                };
                project
                    .engine
                    .persist_activation(&id, &transition, active)?;
                if let Some(active) = active {
                    project.active.push(active.clone());
                }
                if let Some(definition) = definition {
                    metadata.push(definition_metadata(&scope, definition, &next, admission)?);
                }
            }
            project.engine.recover_pending()?;
        }
        let unindexed_roots = repositories
            .keys()
            .filter(|root| !represented_roots.contains(*root))
            .cloned()
            .map(Some)
            .chain((scopes.is_empty() && repositories.is_empty()).then_some(None));
        for root in unindexed_roots {
            let config = DiscoveryConfig {
                global_checks_directory: self.global_directory.clone(),
                project_root: root.clone(),
            };
            let files = definition_files(&config, &directories, &BTreeSet::new());
            let result = discover_definitions(&config, &files);
            diagnostics::collect(&mut errors, &result.diagnostics);
            for definition in result.definitions {
                metadata.push(definition_metadata(
                    &ActivationScope {
                        environment: "native".into(),
                        project: root
                            .as_ref()
                            .map(|root| root.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        run: RunScope::Live,
                    },
                    &definition,
                    &LifecycleState::Removed {
                        generation: antiburn_checks::domain::Generation(0),
                    },
                    &Admission::Paused(
                        "No sessions found. Start an agent session in this project.".into(),
                    ),
                )?);
            }
        }
        self.diagnostic_log.publish(errors);
        ensure!(
            metadata.len() <= MAX_DEFINITIONS,
            "installed checks exceed the {MAX_DEFINITIONS}-definition report limit"
        );
        self.definitions = metadata;
        Ok(())
    }

    /// Apply a bounded number of pages. Evaluation tasks run on the async runtime
    /// while this helper catches up on a blocking worker.
    fn catch_up(&mut self) -> Result<(bool, bool)> {
        let active = self
            .projects
            .iter()
            .flat_map(|(scope, project)| {
                project
                    .active
                    .iter()
                    .map(move |activation| (scope.clone(), activation.clone()))
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        let mut more = false;
        for _ in 0..SOURCE_PAGES_PER_PASS.min(active.len()) {
            let (scope, activation) = &active[self.source_turn % active.len()];
            self.source_turn = self.source_turn.wrapping_add(1);
            let project = &self.projects[scope];
            let source = FileEventSource::new(
                StorePublishedEvidenceReader::for_live_project(
                    &self.store,
                    &scope.environment,
                    &scope.project,
                    self.live_since_epoch,
                ),
                SOURCE_PAGE_SIZE,
            )
            .map_err(|error| anyhow::anyhow!("installed source: {error:?}"))?;
            let progress = project.store.load_source_progress(activation)?;
            let page = source
                .read_batch(progress.publication_cursor)
                .map_err(|error| anyhow::anyhow!("installed source: {error:?}"))?;
            more |= page.has_more;
            if !page.events.is_empty() {
                let upserts = page
                    .events
                    .iter()
                    .filter(|event| matches!(event, antiburn_checks::source::SourceEvent::Observation { observation, .. } if matches!(&observation.kind, antiburn_checks::domain::ObservationKind::ToolResult { .. })))
                    .count();
                let gaps = page
                    .events
                    .iter()
                    .filter(|event| {
                        matches!(event, antiburn_checks::source::SourceEvent::Gap { .. })
                    })
                    .count();
                let outcome = project.engine.apply_source_page(activation, &page)?;
                let window = activation.compiled.trigger.window;
                ::tracing::debug!(
                    event = "installed_check_source_page",
                    check_id = %activation.id.definition_id,
                    generation = activation.id.generation.0,
                    source_events = page.events.len(),
                    tool_results = upserts,
                    coverage_gaps = gaps,
                    candidates = outcome.candidates,
                    window_size = window.map_or(1, |window| window.size),
                    window_minimum = window.map_or(1, |window| window.minimum),
                    has_more = page.has_more,
                );
                if upserts > 0 && outcome.candidates == 0 {
                    ::tracing::debug!(
                        event = "installed_check_no_candidate",
                        check_id = %activation.id.definition_id,
                        generation = activation.id.generation.0,
                        tool_results = upserts,
                        reason = if gaps > 0 { "partial_source_coverage" } else { "window_or_filter_not_ready" },
                    );
                }
                changed = true;
            }
            project.engine.recover_pending()?;
        }
        Ok((changed, more))
    }

    fn evaluation_jobs(&mut self, slots: usize) -> Vec<EvaluationJob<E>> {
        let active = self
            .projects
            .values()
            .flat_map(|project| {
                project.active.iter().map(move |activation| EvaluationJob {
                    engine: Arc::clone(&project.engine),
                    activation: activation.clone(),
                })
            })
            .collect::<Vec<_>>();
        if active.is_empty() {
            return Vec::new();
        }
        (0..slots)
            .map(|_| {
                let job = &active[self.evaluation_turn % active.len()];
                self.evaluation_turn = self.evaluation_turn.wrapping_add(1);
                EvaluationJob {
                    engine: Arc::clone(&job.engine),
                    activation: job.activation.clone(),
                }
            })
            .collect()
    }

    fn deactivate_active(&mut self) -> Result<()> {
        for project in self.projects.values_mut() {
            for activation in std::mem::take(&mut project.active) {
                let next_generation = activation
                    .id
                    .generation
                    .0
                    .checked_add(1)
                    .context("installed check generation overflow")?;
                let transition = LifecycleTransition::Deactivated {
                    revision: activation.revision,
                    project_root: activation.project_root.clone(),
                    scope: activation.scope,
                    source_paths: activation.source_paths,
                    inactive_kind: InactiveKind::Paused,
                    next_generation: antiburn_checks::domain::Generation(next_generation),
                };
                project.engine.persist_activation(
                    &activation.id.definition_id,
                    &transition,
                    None,
                )?;
            }
        }
        Ok(())
    }
}

struct EvaluationJob<E> {
    engine: Arc<CheckEngine<StoreCheckStore, E>>,
    activation: EffectiveActivation,
}

impl<E: DecisionEvaluator> EvaluationJob<E> {
    #[cfg(test)]
    async fn run(self) -> Result<Option<RunSettlement>, StoreError> {
        self.run_with_notification(|| {}).await
    }

    async fn run_with_notification(
        self,
        on_started: impl FnOnce(),
    ) -> Result<Option<RunSettlement>, StoreError> {
        let now = u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
            .unwrap_or_default();
        match self.engine.start_one_due(&self.activation, now)? {
            StartOutcome::Started(task) => {
                on_started();
                let result = task.await;
                match &result {
                    Ok(settlement) => ::tracing::debug!(
                        event = "installed_check_run_settled",
                        check_id = %self.activation.id.definition_id,
                        generation = self.activation.id.generation.0,
                        outcome = match &settlement.outcome {
                            RunOutcome::Accepted => "accepted",
                            RunOutcome::Deferred => "deferred",
                            RunOutcome::Stale => "stale",
                            RunOutcome::Skipped => "skipped",
                            RunOutcome::NotAssessed(_) => "not_assessed",
                            RunOutcome::Failed(_) => "failed",
                        },
                        attempts = settlement.attempts,
                    ),
                    Err(_) => ::tracing::warn!(
                        event = "installed_check_run_settlement_error",
                        check_id = %self.activation.id.definition_id,
                        generation = self.activation.id.generation.0,
                    ),
                }
                result.map(Some)
            }
            StartOutcome::Stale | StartOutcome::NotDue => Ok(None),
        }
    }
}

fn live_projects(store: &Store) -> Result<Vec<ActivationScope>> {
    let connection = store.lock();
    let mut statement = connection.prepare("SELECT DISTINCT environment_key, cwd FROM session WHERE cwd IS NOT NULL AND cwd <> '' ORDER BY environment_key, cwd LIMIT 129")?;
    let mut scopes = statement
        .query_map([], |row| {
            Ok(ActivationScope {
                environment: row.get(0)?,
                project: row.get(1)?,
                run: RunScope::Live,
            })
        })?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    let mut statement = connection.prepare(
        "SELECT DISTINCT project_key FROM burn_check_activation WHERE json_extract(project_key, '$.run') = 'Live' ORDER BY project_key LIMIT 129",
    )?;
    for key in statement.query_map([], |row| row.get::<_, String>(0))? {
        let scope: ActivationScope = serde_json::from_str(&key?)?;
        if scope.run == RunScope::Live {
            scopes.insert(scope);
        }
    }
    ensure!(
        scopes.len() <= MAX_DEFINITIONS,
        "too many installed check projects"
    );
    Ok(scopes.into_iter().collect())
}

fn definition_files(
    config: &DiscoveryConfig,
    directories: &BTreeMap<PathBuf, loader::DirectoryChecks>,
    previous_ids: &BTreeSet<String>,
) -> Vec<CheckFile> {
    let mut files = Vec::new();
    if let Some(directory) = &config.global_checks_directory {
        files.extend(directory_files(
            directory,
            &directories[directory],
            previous_ids,
        ));
    }
    if let Some(root) = &config.project_root {
        let directory = root.join(".antiburn/checks");
        let ids = previous_ids
            .iter()
            .cloned()
            .chain(
                files
                    .iter()
                    .filter_map(|file| file.path.file_stem()?.to_str().map(str::to_owned)),
            )
            .collect();
        files.extend(directory_files(&directory, &directories[&directory], &ids));
    }
    files
}

fn directory_files(
    directory: &Path,
    checks: &loader::DirectoryChecks,
    ids: &BTreeSet<String>,
) -> Vec<CheckFile> {
    match &checks.error {
        None => checks.files.clone(),
        // An incomplete listing cannot prove that a previous override was removed.
        Some(error) => ids
            .iter()
            .map(|id| CheckFile {
                path: directory.join(format!("{id}.yml")),
                content: Err(error.clone()),
                enabled: true,
            })
            .collect(),
    }
}

fn admitted_definition(
    definition: &EffectiveDefinition,
    admission: &Admission,
) -> EffectiveDefinition {
    let mut effective = definition.clone();
    if matches!(effective.state, DefinitionState::Ready(_)) && *admission != Admission::Ready {
        effective.state = DefinitionState::Paused(Vec::new());
    }
    effective
}

fn definition_metadata(
    scope: &ActivationScope,
    definition: &EffectiveDefinition,
    state: &LifecycleState,
    admission: &Admission,
) -> Result<InstalledCheckDefinition> {
    let project_key = StoreCheckStore::project_key(scope);
    let (name, description, mut availability, diagnostic) = match &definition.state {
        DefinitionState::Ready(compiled) => (
            compiled.name.clone(),
            compiled.description.clone(),
            InstalledCheckAvailability::Available,
            None,
        ),
        DefinitionState::Paused(diagnostics) => (
            definition.id.clone(),
            None,
            InstalledCheckAvailability::Paused {
                reason: Some("This check is turned off for this project.".into()),
            },
            (!diagnostics.is_empty()).then(|| {
                diagnostics
                    .iter()
                    .take(8)
                    .map(diagnostics::SafeDiagnostic::display)
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
        ),
        DefinitionState::Invalid(diagnostics) => {
            let diagnostic = diagnostics
                .iter()
                .take(8)
                .map(diagnostics::SafeDiagnostic::display)
                .collect::<Vec<_>>()
                .join("\n");
            let availability = if diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::CompileUnsupportedFeature)
            {
                InstalledCheckAvailability::Unsupported {
                    reason: Some(diagnostic.clone()),
                }
            } else {
                InstalledCheckAvailability::Invalid {
                    reason: Some(diagnostic.clone()),
                }
            };
            (definition.id.clone(), None, availability, Some(diagnostic))
        }
    };
    if matches!(availability, InstalledCheckAvailability::Available) {
        availability = match admission {
            Admission::Ready => availability,
            Admission::Paused(reason) => InstalledCheckAvailability::Paused {
                reason: Some(reason.clone()),
            },
            Admission::Unsupported(reason) => InstalledCheckAvailability::Unsupported {
                reason: Some(reason.clone()),
            },
        };
    }
    let generation = match state {
        LifecycleState::Active(active) => active.id.generation,
        LifecycleState::Inactive { generation, .. }
        | LifecycleState::Removed { generation }
        | LifecycleState::Uncompiled { generation } => *generation,
    };
    Ok(InstalledCheckDefinition {
        effective_id: serde_json::to_string(&(scope, &definition.id))?,
        check_id: definition.id.clone(),
        project_key,
        generation: if generation.0 == 0 {
            None
        } else {
            Some(i64::try_from(generation.0)?)
        },
        definition_revision: hex(&definition.content_revision),
        name,
        aliases: vec![definition.id.clone()],
        description,
        source: InstalledCheckSource {
            scope: match definition.scope {
                DefinitionScope::Global => InstalledCheckScope::Global,
                DefinitionScope::Project => InstalledCheckScope::Project,
            },
            path: definition
                .source_paths
                .first()
                .map(|path| path.to_string_lossy().into_owned()),
        },
        availability,
        diagnostic,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn start(app: &tauri::AppHandle) {
    let state = app.state::<InstalledChecks>();
    let mut task = state.task.lock().expect("installed task lock");
    assert!(task.is_none(), "start installed checks once");
    state.requested_turn.fetch_add(1, Ordering::AcqRel);
    let worker_app = app.clone();
    *task = Some(tauri::async_runtime::spawn(async move {
        if let Err(error) = run(&worker_app).await {
            ::tracing::error!(event = "installed_checks_stopped", error = %error);
        }
    }));
}

async fn run(app: &tauri::AppHandle) -> Result<()> {
    let state = app.state::<InstalledChecks>();
    let store = Arc::new(app.state::<Store>().inner().clone());
    let evaluator = Arc::new(InstalledDecisionEvaluator { app: app.clone() });
    let global = antiburn_local::paths::home_dir().map(|home| home.join(".antiburn/checks"));
    let mut host = InstalledCheckHost::new(store, evaluator, global)?;
    let mut tasks = JoinSet::new();
    let mut poll = tokio::time::interval(Duration::from_secs(1));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut lifecycle = app
        .state::<crate::session_lifecycle::SessionEvents>()
        .subscribe();
    let mut last_discovery = None::<Instant>;
    let mut previous_admission = None;
    let mut catch_up_more = false;
    let mut watcher: Option<watch::DirectoryWatcher> = None;
    let mut processed_turn = 0;
    loop {
        let mut discover = false;
        tokio::select! {
            () = state.wake.notified() => discover = true,
            _ = poll.tick() => {},
            () = tokio::task::yield_now(), if catch_up_more => {},
            event = lifecycle.recv() => {
                if matches!(event, Err(tokio::sync::broadcast::error::RecvError::Closed)) { break; }
                discover = true;
            },
            settled = tasks.join_next(), if !tasks.is_empty() => {
                match settled {
                    Some(Ok(Ok(Some(_)))) => { let _ = app.emit(crate::commands::CHECKS_REPORT_CHANGED_EVENT, ()); },
                    Some(Ok(Err(error))) => ::tracing::error!(event = "installed_check_settlement_failed", error = %error),
                    Some(Err(error)) => ::tracing::error!(event = "installed_check_task_failed", error = %error),
                    _ => {},
                }
                if tasks.is_empty() && !catch_up_more {
                    state.completed_turn.store(processed_turn, Ordering::Release);
                    crate::jev::worker::wake(app);
                }
                continue;
            },
        }
        if state.stopping.load(Ordering::Acquire) {
            break;
        }
        if !state.credentials_restored.load(Ordering::Acquire) {
            continue;
        }
        if !crate::jev::settings::yaml_discovery_enabled(&app.state::<Store>()) {
            host.deactivate_active()?;
            if let Some(watcher) = &mut watcher {
                watcher.refresh(&[]);
            }
            catch_up_more = false;
            last_discovery = None;
            previous_admission = None;
            continue;
        }
        processed_turn = state.requested_turn.load(Ordering::Acquire);
        let current_admission = admission(app);
        discover |= previous_admission.as_ref() != Some(&current_admission)
            || last_discovery.is_none_or(|last| last.elapsed() >= DISCOVERY_INTERVAL);
        let work_admission = current_admission.clone();
        let (returned_host, result, reconciled) = tauri::async_runtime::spawn_blocking(move || {
            let mut reconciled = false;
            let result = (|| {
                if discover {
                    host.reconcile(&work_admission)?;
                    reconciled = true;
                }
                host.catch_up()
            })();
            (host, result, reconciled)
        })
        .await?;
        host = returned_host;
        if reconciled {
            *state.metadata.write().expect("installed metadata lock") = host.definitions.clone();
            last_discovery = Some(Instant::now());
            previous_admission = Some(current_admission);
            crate::jev::settings::progress_changed(app);
            let _ = app.emit(crate::commands::CHECKS_REPORT_CHANGED_EVENT, ());
            if watcher.is_none() {
                let watch_app = app.clone();
                match watch::DirectoryWatcher::new(move || wake(&watch_app)) {
                    Ok(created) => watcher = Some(created),
                    Err(error) => {
                        ::tracing::error!(event = "installed_check_watch_failed", error = %error)
                    }
                }
            }
            if let Some(watcher) = &mut watcher
                && watcher.refresh(&host.watch_directories)
            {
                state.wake.notify_one();
            }
        }
        match result {
            Ok((changed, more)) => {
                if changed {
                    let _ = app.emit(crate::commands::CHECKS_REPORT_CHANGED_EVENT, ());
                }
                for job in host.evaluation_jobs(EVALUATIONS.saturating_sub(tasks.len())) {
                    let report_app = app.clone();
                    tasks.spawn(job.run_with_notification(move || {
                        let _ = report_app.emit(crate::commands::CHECKS_REPORT_CHANGED_EVENT, ());
                    }));
                }
                catch_up_more = more;
                if tasks.is_empty() && !more {
                    state
                        .completed_turn
                        .store(processed_turn, Ordering::Release);
                    crate::jev::worker::wake(app);
                }
            }
            Err(error) => {
                catch_up_more = false;
                if !reconciled {
                    last_discovery = None;
                }
                ::tracing::error!(event = "installed_check_update_failed", error = %error);
                state
                    .completed_turn
                    .store(processed_turn, Ordering::Release);
                crate::jev::worker::wake(app);
            }
        }
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            ::tracing::warn!(event = "installed_check_shutdown_failed", error = %error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
