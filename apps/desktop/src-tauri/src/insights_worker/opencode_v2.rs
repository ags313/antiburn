//! Reads one verified OpenCode V2 event feed and publishes ordered results.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use antiburn_local::{
    analysis::{
        CaptureBaseline, SourceFormat,
        opencode_v2_capture::BoundaryKind,
        opencode_v2_capture::{CaptureGap, CapturedEvent, OpenCodeV2CaptureEpoch},
        opencode_v2_events::{Gap, OpenCodeV2EventJoiner},
        read_capture_baseline_for_scope,
    },
    discovery::SessionSource,
    model::AgentKind,
};
use anyhow::{Context, bail};
use serde_json::Value;
use tauri::async_runtime::JoinHandle;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;

use super::{LEASE_SECS, SessionKey, Store, unix_now};
use crate::store::native_tool_results::opencode_v2_status::OpenCodeV2SourceStatus;
use crate::{
    commands,
    first_run_gate::{FirstRunGate, FirstRunStage},
    opencode_live::{self, stream::EventStream},
    store::native_tool_results::source_delivery::opencode_v2::{
        OpenCodeV2CaptureClaim, OpenCodeV2CaptureStart,
    },
};

const CAPTURE_SESSION_LIMIT: usize = 256;
const CANDIDATE_SESSION_LIMIT: usize = 1024;
const CAPTURE_DATABASE_LIMIT: usize = 8;
const CAPTURE_SETTINGS_CHECK_SECS: u64 = 1;
const CAPTURE_RECHECK_SECS: u64 = 30;
const CAPTURE_RETRY_BASE_SECS: u64 = 2;
const CAPTURE_RETRY_MAX_SECS: u64 = 60;
const CAPTURE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const OPENCODE_V2_VERSION: &str = "2.0.32";

pub(super) async fn run(app: tauri::AppHandle) {
    let gate = app.state::<FirstRunGate>();
    let mut retry_count = 0_u32;
    loop {
        gate.wait_until(FirstRunStage::Checks, || false).await;
        if !crate::installed_checks::live_delivery_ready(&app) {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let store = app.state::<Store>();
        match run_capture_epoch(&app, &store).await {
            Ok(()) => retry_count = 0,
            Err(error) => {
                let status = capture_failure_status(&error);
                match store
                    .opencode_v2_live_candidates(unix_now(), CANDIDATE_SESSION_LIMIT)
                    .and_then(|records| {
                        store.set_opencode_v2_source_status(&records, status, unix_now())
                    }) {
                    Ok(changed) if changed > 0 => {
                        let _ = app.emit(commands::CHECKS_REPORT_CHANGED_EVENT, ());
                    }
                    Err(_) => ::tracing::warn!(event = "opencode_v2_source_status_failed"),
                    _ => {}
                }
                ::tracing::warn!(event = "opencode_v2_capture_stopped", retry_count,);
                retry_count = retry_count.saturating_add(1).min(10);
                let delay = CAPTURE_RETRY_BASE_SECS
                    .saturating_mul(1_u64 << retry_count.min(5))
                    .min(CAPTURE_RETRY_MAX_SECS);
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        }
    }
}

async fn run_capture_epoch(app: &tauri::AppHandle, store: &Store) -> anyhow::Result<()> {
    let records = store.opencode_v2_live_candidates(unix_now(), CANDIDATE_SESSION_LIMIT)?;
    if records.len() == CANDIDATE_SESSION_LIMIT {
        return Err(CaptureFailure(OpenCodeV2SourceStatus::SourceLimit).into());
    }
    if records.is_empty() {
        tokio::time::sleep(Duration::from_secs(5)).await;
        return Ok(());
    }
    let sources = locate_v2_sources(&records).await?;
    let registration_path =
        opencode_live::default_registration_path().context("locating OpenCode registration")?;
    let groups = select_bound_databases(&registration_path, sources).await?;
    if groups.is_empty() {
        return Err(CaptureFailure(OpenCodeV2SourceStatus::SourceMismatch).into());
    }
    let bound_keys: HashSet<_> = groups
        .iter()
        .flat_map(|(_, records)| records.iter().map(|record| &record.key))
        .collect();
    let unbound: Vec<_> = records
        .iter()
        .filter(|record| !bound_keys.contains(&record.key))
        .cloned()
        .collect();
    if !unbound.is_empty() {
        store.set_opencode_v2_source_status(
            &unbound,
            OpenCodeV2SourceStatus::SourceMismatch,
            unix_now(),
        )?;
    }
    let records: Vec<_> = groups
        .iter()
        .flat_map(|(_, records)| records.iter().cloned())
        .collect();
    if records.len() > CAPTURE_SESSION_LIMIT {
        return Err(CaptureFailure(OpenCodeV2SourceStatus::SourceLimit).into());
    }
    let database_path = groups[0].0.clone();
    let database_identity = DatabaseIdentity::read(&database_path)?;
    let mut stream = opencode_live::stream::open_verified_event_stream(
        &registration_path,
        &database_path,
        CAPTURE_CONNECT_TIMEOUT,
    )
    .await
    .context("opening the verified OpenCode V2 event stream")?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let first = stream.next_event(&cancelled).await?;
    if first.data.get("type").and_then(Value::as_str) != Some("server.connected") {
        bail!("OpenCode V2 event feed did not confirm its subscription");
    }
    if stream.service_info().version != OPENCODE_V2_VERSION {
        return Err(CaptureFailure(OpenCodeV2SourceStatus::VersionUnsupported).into());
    }
    let registration = stream.registration().clone();
    verify_bound_databases(&registration_path, &groups, &registration).await?;
    database_identity.verify()?;

    let session_ids: Vec<_> = records
        .iter()
        .map(|record| record.key.session_id.clone())
        .collect();
    let mut capture =
        OpenCodeV2CaptureEpoch::new(session_ids.clone()).map_err(capture_gap_error)?;
    capture.confirm_subscription().map_err(capture_gap_error)?;
    let capture_cancel = cancelled.clone();
    let baseline_groups = groups.clone();
    let mut pump = FeedPump::start(stream, cancelled, session_ids.iter().cloned().collect());
    let mut baseline_task = tokio::task::spawn_blocking(move || {
        read_baselines(&baseline_groups, &|| capture_cancel.load(Ordering::SeqCst))
    });
    let baseline = loop {
        tokio::select! {
            result = &mut baseline_task => {
                break result.context("reading OpenCode V2 capture baseline")??;
            }
            event = pump.next() => {
                let event = event.context("OpenCode V2 feed ended during baseline hydration")??;
                capture.observe(event).map_err(capture_gap_error)?;
            }
        }
    };
    database_identity.verify()?;
    let buffered = capture
        .finish_hydration(baseline.clone())
        .map_err(capture_gap_error)?;
    let starts = capture_starts(store, &records, &baseline)?;
    let capture_epoch = capture.id().to_string();
    let Some(claims) =
        store.start_opencode_v2_capture_epoch(&starts, &capture_epoch, unix_now(), LEASE_SECS)?
    else {
        pump.stop().await;
        bail!("OpenCode V2 capture is leased by another worker");
    };
    let lease = CaptureLease::new(store, claims);
    for claim in &lease.claims {
        ::tracing::debug!(
            event = "opencode_v2_capture_attached",
            session_id = %claim.key.session_id,
            capture_epoch = %claim.capture_epoch,
            baseline_sequence = ?claim.baseline_sequence,
            last_sequence = ?claim.last_sequence,
            history_gap = claim.history_gap,
        );
    }
    let claims_by_session = lease
        .claims
        .iter()
        .map(|claim| (claim.key.session_id.clone(), claim.clone()))
        .collect::<HashMap<_, _>>();
    let joiners: HashMap<String, OpenCodeV2EventJoiner> = session_ids
        .iter()
        .map(|session| (session.clone(), OpenCodeV2EventJoiner::new()))
        .collect();
    let mut active = ActiveCaptures {
        capture,
        lease,
        claims_by_session,
        joiners,
        gapped_sessions: HashSet::new(),
    };
    active.record_gaps()?;

    for event in buffered {
        database_identity.verify()?;
        if active.gapped_sessions.contains(&event.session_id) {
            continue;
        }
        if let Some(key) = active.apply_captured(event, store)? {
            announce(app, &key);
        }
    }

    let mut recheck = tokio::time::interval(Duration::from_secs(CAPTURE_RECHECK_SECS));
    recheck.tick().await;
    let mut settings_check =
        tokio::time::interval(Duration::from_secs(CAPTURE_SETTINGS_CHECK_SECS));
    settings_check.tick().await;
    loop {
        tokio::select! {
            event = pump.next() => {
                database_identity.verify()?;
                let Some(event) = event else {
                    active.lease.mark_gap("event-feed-ended")?;
                    bail!("OpenCode V2 event feed ended");
                };
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        active.lease.mark_gap("event-feed-error")?;
                        return Err(error).context("reading OpenCode V2 event feed");
                    }
                };
                if let Some(key) = active.observe(event, store)? { announce(app, &key); }
            }
            _ = settings_check.tick() => {
                if !crate::installed_checks::live_delivery_ready(app) {
                    active.lease.mark_gap("checks-disabled")?;
                    pump.stop().await;
                    return Ok(());
                }
            }
            _ = recheck.tick() => {
                if !active.lease.claims.is_empty() && !store.renew_opencode_v2_capture_epoch(&active.lease.claims, unix_now(), LEASE_SECS)? {
                    active.lease.mark_gap("capture-lease-lost")?;
                    pump.stop().await;
                    bail!("OpenCode V2 capture lease was lost");
                }
                let current =
                    store.opencode_v2_live_candidates(unix_now(), CANDIDATE_SESSION_LIMIT)?;
                if current.len() == CANDIDATE_SESSION_LIMIT {
                    active.lease.mark_gap("live-scope-overflow")?;
                    pump.stop().await;
                    bail!("OpenCode V2 active session scope exceeds its limit");
                }
                let current_sources = locate_v2_sources(&current).await?;
                let current_groups = select_bound_databases(&registration_path, current_sources)
                    .await?;
                let current_bound_records: Vec<_> = current_groups
                    .iter()
                    .flat_map(|(_, records)| records.iter().cloned())
                    .collect();
                if current_bound_records.len() > CAPTURE_SESSION_LIMIT {
                    active.lease.mark_gap("live-scope-overflow")?;
                    pump.stop().await;
                    bail!("OpenCode V2 bound session scope exceeds its limit");
                }
                if scope_changed(&records, &current_bound_records) {
                    active.lease.mark_gap("live-scope-changed")?;
                    pump.stop().await;
                    bail!("OpenCode V2 live scope changed");
                }
                verify_bound_databases(&registration_path, &groups, &registration).await?;
                database_identity.verify()?;
                if !active.gapped_sessions.is_empty() {
                    recover_gapped_sessions(app, store, &groups, &database_identity, &mut pump, &mut active).await?;
                }
            }
        }
    }
}

#[derive(Debug)]
struct CaptureFailure(OpenCodeV2SourceStatus);

impl std::fmt::Display for CaptureFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0.code())
    }
}

impl std::error::Error for CaptureFailure {}

fn capture_failure_status(error: &anyhow::Error) -> OpenCodeV2SourceStatus {
    use opencode_live::stream::EventStreamError;
    use opencode_live::{DatabaseAssociationError, ProbeError};
    for cause in error.chain() {
        if let Some(failure) = cause.downcast_ref::<CaptureFailure>() {
            return failure.0;
        }
        if let Some(registration) = cause.downcast_ref::<opencode_live::RegistrationError>() {
            return match registration {
                opencode_live::RegistrationError::TooLarge => OpenCodeV2SourceStatus::SourceLimit,
                opencode_live::RegistrationError::InvalidEndpoint => {
                    OpenCodeV2SourceStatus::SourceMismatch
                }
                _ => OpenCodeV2SourceStatus::ServiceUnavailable,
            };
        }
        if let Some(probe) = cause.downcast_ref::<ProbeError>() {
            return match probe {
                ProbeError::HttpStatus(401 | 403) => OpenCodeV2SourceStatus::AuthenticationRejected,
                ProbeError::VersionMismatch => OpenCodeV2SourceStatus::VersionUnsupported,
                ProbeError::ProcessMismatch => OpenCodeV2SourceStatus::SourceMismatch,
                _ => OpenCodeV2SourceStatus::ServiceUnavailable,
            };
        }
        if let Some(
            DatabaseAssociationError::Binding | DatabaseAssociationError::RegistrationChanged,
        ) = cause.downcast_ref::<DatabaseAssociationError>()
        {
            return OpenCodeV2SourceStatus::SourceMismatch;
        }
        if let Some(stream) = cause.downcast_ref::<EventStreamError>() {
            match stream {
                EventStreamError::HttpStatus(401 | 403) => {
                    return OpenCodeV2SourceStatus::AuthenticationRejected;
                }
                EventStreamError::CapacityExceeded
                | EventStreamError::Sse(opencode_live::SseError::FrameTooLarge) => {
                    return OpenCodeV2SourceStatus::SourceLimit;
                }
                EventStreamError::RegistrationChanged => {
                    return OpenCodeV2SourceStatus::SourceMismatch;
                }
                EventStreamError::Registration(_) => {}
                EventStreamError::Request => {
                    return OpenCodeV2SourceStatus::ServiceUnavailable;
                }
                _ => {}
            }
        }
    }
    OpenCodeV2SourceStatus::CaptureGap
}

struct ActiveCaptures<'a> {
    capture: OpenCodeV2CaptureEpoch,
    lease: CaptureLease<'a>,
    claims_by_session: HashMap<String, OpenCodeV2CaptureClaim>,
    joiners: HashMap<String, OpenCodeV2EventJoiner>,
    gapped_sessions: HashSet<String>,
}

impl ActiveCaptures<'_> {
    fn record_gaps(&mut self) -> anyhow::Result<()> {
        mark_gapped_sessions(
            &self.capture,
            &mut self.gapped_sessions,
            &mut self.lease,
            &mut self.claims_by_session,
            &mut self.joiners,
        )
    }

    fn observe(&mut self, event: Value, store: &Store) -> anyhow::Result<Option<SessionKey>> {
        match self.capture.observe(event).map_err(capture_gap_error)? {
            Some(event) => self.apply_captured(event, store),
            None => {
                self.record_gaps()?;
                Ok(None)
            }
        }
    }

    fn apply_captured(
        &mut self,
        event: CapturedEvent,
        store: &Store,
    ) -> anyhow::Result<Option<SessionKey>> {
        let outcome = if event.kind == BoundaryKind::Seed {
            process_seed_event(event, &mut self.joiners)
        } else {
            process_captured_event(event, &mut self.joiners, &self.claims_by_session, store)?
        };
        match outcome {
            ProcessResult::Advanced => Ok(None),
            ProcessResult::ResultPublished(key) => Ok(Some(key)),
            ProcessResult::SessionGap(session_id) => {
                let reason = self
                    .joiners
                    .get(&session_id)
                    .map_or("event-join-gap", |joiner| {
                        if joiner.gaps().contains(&Gap::UnsupportedVersion) {
                            "event-version-unsupported"
                        } else if joiner.gaps().contains(&Gap::CapacityExceeded) {
                            "source-limit"
                        } else {
                            "event-join-gap"
                        }
                    });
                self.lease.mark_session_gap(&session_id, reason)?;
                self.capture.mark_session_gap(&session_id);
                self.claims_by_session.remove(&session_id);
                self.joiners.remove(&session_id);
                self.gapped_sessions.insert(session_id);
                Ok(None)
            }
        }
    }
}

async fn recover_gapped_sessions(
    app: &tauri::AppHandle,
    store: &Store,
    groups: &[DatabaseGroup],
    identity: &DatabaseIdentity,
    pump: &mut FeedPump,
    active: &mut ActiveCaptures<'_>,
) -> anyhow::Result<()> {
    let groups: Vec<_> = groups
        .iter()
        .filter_map(|(path, records)| {
            let records: Vec<_> = records
                .iter()
                .filter(|record| active.gapped_sessions.contains(&record.key.session_id))
                .cloned()
                .collect();
            (!records.is_empty()).then(|| (path.clone(), records))
        })
        .collect();
    let records: Vec<_> = groups
        .iter()
        .flat_map(|(_, records)| records.iter().cloned())
        .collect();
    if records.is_empty() {
        return Ok(());
    }
    let mut recovery = OpenCodeV2CaptureEpoch::new(
        records
            .iter()
            .map(|record| record.key.session_id.clone())
            .collect(),
    )
    .map_err(capture_gap_error)?;
    recovery.confirm_subscription().map_err(capture_gap_error)?;
    identity.verify()?;
    let read_groups = groups.clone();
    let cancelled = pump.cancelled.clone();
    let mut baseline_task = tokio::task::spawn_blocking(move || {
        read_baselines(&read_groups, &|| cancelled.load(Ordering::SeqCst))
    });
    let baseline = loop {
        tokio::select! {
            result = &mut baseline_task => break result,
            event = pump.next() => {
                identity.verify()?;
                let event = event.context("OpenCode V2 feed ended during session recovery")??;
                if recovery.observe(event.clone()).is_err() {
                    return Ok(());
                }
                if let Some(key) = active.observe(event, store)? { announce(app, &key); }
            }
        }
    };
    identity.verify()?;
    let Ok(Ok(baseline)) = baseline else {
        ::tracing::warn!(event = "opencode_v2_session_reconcile_failed");
        return Ok(());
    };
    let buffered = match recovery.finish_hydration(baseline.clone()) {
        Ok(buffered) if recovery.gapped_sessions().next().is_none() => buffered,
        _ => return Ok(()),
    };
    let starts = capture_starts(store, &records, &baseline)?;
    let Some(claims) = store.start_opencode_v2_capture_epoch(
        &starts,
        &recovery.id().to_string(),
        unix_now(),
        LEASE_SECS,
    )?
    else {
        return Ok(());
    };
    if active.capture.resume_sessions(recovery).is_err() {
        store.mark_opencode_v2_capture_gap(&claims, "recovery-capacity", unix_now())?;
        return Ok(());
    }
    for claim in &claims {
        active.gapped_sessions.remove(&claim.key.session_id);
        active
            .joiners
            .insert(claim.key.session_id.clone(), OpenCodeV2EventJoiner::new());
        active
            .claims_by_session
            .insert(claim.key.session_id.clone(), claim.clone());
    }
    active.lease.claims.extend(claims);
    active.lease.active = true;
    for event in buffered {
        identity.verify()?;
        if active.gapped_sessions.contains(&event.session_id) {
            continue;
        }
        if let Some(key) = active.apply_captured(event, store)? {
            announce(app, &key);
        }
    }
    Ok(())
}

struct DatabaseIdentity {
    path: PathBuf,
    file: std::fs::File,
    identity: String,
}

impl DatabaseIdentity {
    fn read(path: &std::path::Path) -> anyhow::Result<Self> {
        let file =
            std::fs::File::open(path).context("opening the OpenCode V2 database identity")?;
        let identity = antiburn_local::discovery::SourceStat::from_open_std_file(&file)
            .and_then(|stat| stat.identity)
            .context("OpenCode V2 database identity is unavailable")?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
        })
    }

    fn verify(&self) -> anyhow::Result<()> {
        let current = Self::read(&self.path)?;
        let held = antiburn_local::discovery::SourceStat::from_open_std_file(&self.file)
            .and_then(|stat| stat.identity);
        if current.identity != self.identity || held.as_ref() != Some(&self.identity) {
            return Err(CaptureFailure(OpenCodeV2SourceStatus::SourceMismatch).into());
        }
        Ok(())
    }
}

async fn locate_v2_sources(
    records: &[crate::store::SessionRecord],
) -> anyhow::Result<Vec<(crate::store::SessionRecord, PathBuf)>> {
    let mut sources = Vec::with_capacity(records.len());
    for record in records {
        let Some(source) =
            super::analysis::locate(AgentKind::OpenCode, &record.key.session_id, None).await
        else {
            continue;
        };
        let SessionSource::ProviderDb {
            agent: AgentKind::OpenCode,
            db_path,
            session_id,
            source_format: Some(SourceFormat::OpenCodeSqliteV2),
        } = source
        else {
            continue;
        };
        if session_id != record.key.session_id {
            continue;
        }
        sources.push((record.clone(), db_path));
    }
    Ok(sources)
}

type DatabaseGroup = (PathBuf, Vec<crate::store::SessionRecord>);

async fn select_bound_databases(
    registration_path: &std::path::Path,
    sources: Vec<(crate::store::SessionRecord, PathBuf)>,
) -> anyhow::Result<Vec<DatabaseGroup>> {
    let mut grouped = BTreeMap::<PathBuf, Vec<crate::store::SessionRecord>>::new();
    for (record, path) in sources {
        let Ok(path) = std::fs::canonicalize(path) else {
            continue;
        };
        grouped.entry(path).or_default().push(record);
    }
    if grouped.len() > CAPTURE_DATABASE_LIMIT {
        return Err(CaptureFailure(OpenCodeV2SourceStatus::SourceLimit).into());
    }

    let mut bound = Vec::new();
    for (path, records) in grouped {
        let registration_path = registration_path.to_path_buf();
        let binding_path = path.clone();
        let is_bound = tokio::task::spawn_blocking(move || {
            match opencode_live::verify_database_association(
                &registration_path,
                &binding_path,
                CAPTURE_CONNECT_TIMEOUT,
            ) {
                Ok(_) => Ok(true),
                Err(opencode_live::DatabaseAssociationError::Binding) => Ok(false),
                Err(error) => Err(anyhow::Error::new(error)),
            }
        })
        .await
        .context("checking OpenCode V2 source binding")??;
        if is_bound {
            bound.push((path, records));
        }
    }
    if bound.len() > 1 {
        return Err(CaptureFailure(OpenCodeV2SourceStatus::SourceMismatch).into());
    }
    Ok(bound)
}

fn read_baselines(
    groups: &[DatabaseGroup],
    cancel: &dyn Fn() -> bool,
) -> anyhow::Result<CaptureBaseline> {
    let mut all = CaptureBaseline::new();
    for (path, records) in groups {
        if cancel() {
            bail!("OpenCode V2 baseline read was cancelled");
        }
        let scopes: Vec<_> = records
            .iter()
            .map(|record| {
                Ok((
                    record.key.session_id.clone(),
                    record
                        .cwd
                        .clone()
                        .context("OpenCode V2 baseline session has no CWD")?,
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let baseline =
            read_capture_baseline_for_scope(path, SourceFormat::OpenCodeSqliteV2, &scopes, cancel)?;
        for (session_id, sequence) in baseline {
            if all.insert(session_id, sequence).is_some() {
                bail!("OpenCode V2 baseline has duplicate session identities");
            }
        }
    }
    Ok(all)
}

fn capture_starts(
    store: &Store,
    records: &[crate::store::SessionRecord],
    baseline: &HashMap<String, Option<u64>>,
) -> anyhow::Result<Vec<OpenCodeV2CaptureStart>> {
    let keys: Vec<_> = records.iter().map(|record| record.key.clone()).collect();
    let (presence, _) = store.session_presence_for_keys(&keys)?;
    let incarnations: HashMap<_, _> = presence
        .into_iter()
        .map(|presence| (presence.key, presence.incarnation))
        .collect();
    let generations = store.source_generation_batch(&keys)?;
    records
        .iter()
        .zip(generations)
        .map(|(record, generation)| {
            let incarnation = incarnations
                .get(&record.key)
                .copied()
                .context("OpenCode V2 session disappeared during capture setup")?;
            let source_generation =
                generation.context("OpenCode V2 source generation is unavailable")?;
            let baseline_sequence = baseline
                .get(&record.key.session_id)
                .copied()
                .with_context(|| "OpenCode V2 baseline omitted an admitted session")?;
            Ok(OpenCodeV2CaptureStart {
                key: record.key.clone(),
                incarnation,
                source_generation,
                source_label: record.source_label.clone(),
                cwd: record
                    .cwd
                    .clone()
                    .context("OpenCode V2 live session has no CWD")?,
                baseline_sequence,
            })
        })
        .collect()
}

fn process_seed_event(
    event: CapturedEvent,
    joiners: &mut HashMap<String, OpenCodeV2EventJoiner>,
) -> ProcessResult {
    let Some(joiner) = joiners.get_mut(&event.session_id) else {
        return ProcessResult::SessionGap(event.session_id);
    };
    joiner.observe(&event.event);
    if has_hard_join_gap(joiner) {
        ProcessResult::SessionGap(event.session_id)
    } else {
        ProcessResult::Advanced
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProcessResult {
    Advanced,
    ResultPublished(SessionKey),
    SessionGap(String),
}

fn process_captured_event(
    event: CapturedEvent,
    joiners: &mut HashMap<String, OpenCodeV2EventJoiner>,
    claims: &HashMap<String, OpenCodeV2CaptureClaim>,
    store: &Store,
) -> anyhow::Result<ProcessResult> {
    let Some(joiner) = joiners.get_mut(&event.session_id) else {
        return Ok(ProcessResult::SessionGap(event.session_id));
    };
    let result = joiner.observe(&event.event);
    if has_hard_join_gap(joiner) {
        return Ok(ProcessResult::SessionGap(event.session_id));
    }
    let Some(claim) = claims.get(&event.session_id) else {
        return Ok(ProcessResult::SessionGap(event.session_id));
    };
    // The event feed has no replay, and the baseline stores only sequence watermarks.
    let observation = result.map(|result| result.into_observation(false, true));
    let applied = store.advance_opencode_v2_capture(
        claim,
        event.sequence,
        observation.as_ref(),
        unix_now(),
        LEASE_SECS,
    )?;
    if !applied {
        return Ok(ProcessResult::SessionGap(event.session_id));
    }
    Ok(if observation.is_some() {
        ProcessResult::ResultPublished(claim.key.clone())
    } else {
        ProcessResult::Advanced
    })
}

fn mark_gapped_sessions(
    capture: &OpenCodeV2CaptureEpoch,
    reported: &mut HashSet<String>,
    lease: &mut CaptureLease<'_>,
    claims: &mut HashMap<String, OpenCodeV2CaptureClaim>,
    joiners: &mut HashMap<String, OpenCodeV2EventJoiner>,
) -> anyhow::Result<()> {
    let new_gaps: Vec<_> = capture
        .gapped_sessions()
        .filter(|session_id| !reported.contains(*session_id))
        .map(str::to_owned)
        .collect();
    for session_id in new_gaps {
        lease.mark_session_gap(&session_id, "session-event-sequence-gap")?;
        claims.remove(&session_id);
        joiners.remove(&session_id);
        reported.insert(session_id);
    }
    Ok(())
}

fn has_hard_join_gap(joiner: &OpenCodeV2EventJoiner) -> bool {
    joiner.gaps().iter().any(|gap| {
        matches!(
            gap,
            Gap::MissingSequence
                | Gap::SequenceDiscontinuity
                | Gap::ContinuityLost
                | Gap::ConflictingDuplicate
                | Gap::CapacityExceeded
                | Gap::UnknownShape
                | Gap::UnsupportedVersion
        )
    })
}

fn capture_gap_error(gap: CaptureGap) -> anyhow::Error {
    if gap == CaptureGap::CapacityExceeded {
        CaptureFailure(OpenCodeV2SourceStatus::SourceLimit).into()
    } else {
        anyhow::anyhow!("OpenCode V2 capture boundary failed: {gap:?}")
    }
}

fn scope_changed(
    previous: &[crate::store::SessionRecord],
    current: &[crate::store::SessionRecord],
) -> bool {
    let previous: HashSet<_> = previous
        .iter()
        .map(|record| (&record.key, &record.source_label, &record.cwd))
        .collect();
    let current: HashSet<_> = current
        .iter()
        .map(|record| (&record.key, &record.source_label, &record.cwd))
        .collect();
    previous != current
}

fn announce(app: &tauri::AppHandle, key: &SessionKey) {
    crate::installed_checks::wake(app);
    crate::session_lifecycle::report(
        app,
        crate::session_lifecycle::SyncObservation::RowChanged {
            session: key.clone(),
            facets: crate::session_lifecycle::UpdateFacets {
                analysis: true,
                ..Default::default()
            },
            at: unix_now(),
        },
    );
    let _ = app.emit(commands::CHECKS_REPORT_CHANGED_EVENT, ());
}

struct CaptureLease<'a> {
    store: &'a Store,
    claims: Vec<OpenCodeV2CaptureClaim>,
    active: bool,
}

impl<'a> CaptureLease<'a> {
    fn new(store: &'a Store, claims: Vec<OpenCodeV2CaptureClaim>) -> Self {
        Self {
            store,
            claims,
            active: true,
        }
    }

    fn mark_gap(&mut self, reason: &str) -> anyhow::Result<()> {
        self.store
            .mark_opencode_v2_capture_gap(&self.claims, reason, unix_now())?;
        self.active = false;
        Ok(())
    }

    fn mark_session_gap(&mut self, session_id: &str, reason: &str) -> anyhow::Result<()> {
        if let Some(index) = self
            .claims
            .iter()
            .position(|claim| claim.key.session_id == session_id)
        {
            let claim = self.claims[index].clone();
            self.store.mark_opencode_v2_capture_gap(
                std::slice::from_ref(&claim),
                reason,
                unix_now(),
            )?;
            self.claims.remove(index);
        }
        if self.claims.is_empty() {
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for CaptureLease<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ =
                self.store
                    .mark_opencode_v2_capture_gap(&self.claims, "worker-stopped", unix_now());
        }
    }
}

async fn verify_bound_databases(
    registration_path: &std::path::Path,
    groups: &[DatabaseGroup],
    expected: &opencode_live::ServiceRegistration,
) -> anyhow::Result<()> {
    for (database_path, _) in groups {
        let registration_path = registration_path.to_path_buf();
        let database_path = database_path.clone();
        let expected = expected.clone();
        tokio::task::spawn_blocking(move || {
            let current = opencode_live::read_registration(&registration_path)?;
            if current != expected {
                bail!("OpenCode service registration changed during capture");
            }
            opencode_live::verify_database_association(
                &registration_path,
                &database_path,
                CAPTURE_CONNECT_TIMEOUT,
            )?;
            Ok(())
        })
        .await
        .context("checking OpenCode capture binding")??;
    }
    Ok(())
}

struct FeedPump {
    receiver: mpsc::Receiver<anyhow::Result<Value>>,
    cancelled: Arc<AtomicBool>,
    task: JoinHandle<()>,
    overflowed: Arc<AtomicBool>,
}

impl FeedPump {
    fn start(
        mut stream: EventStream,
        cancelled: Arc<AtomicBool>,
        admitted: HashSet<String>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(64);
        let task_cancelled = cancelled.clone();
        let overflowed = Arc::new(AtomicBool::new(false));
        let task_overflowed = overflowed.clone();
        let task = tauri::async_runtime::spawn(async move {
            loop {
                match stream.next_event(&task_cancelled).await {
                    Ok(event) => {
                        if !event.data.get("durable").is_some_and(Value::is_object) {
                            continue;
                        }
                        let session = event
                            .data
                            .pointer("/durable/aggregateID")
                            .and_then(Value::as_str)
                            .or_else(|| {
                                event
                                    .data
                                    .pointer("/data/sessionID")
                                    .and_then(Value::as_str)
                            });
                        if !session.is_some_and(|session| admitted.contains(session)) {
                            continue;
                        }
                        match sender.try_send(Ok(event.data)) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                task_overflowed.store(true, Ordering::SeqCst);
                                break;
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => break,
                        }
                    }
                    Err(opencode_live::stream::EventStreamError::Cancelled) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error.into())).await;
                        break;
                    }
                }
            }
        });
        Self {
            receiver,
            cancelled,
            task,
            overflowed,
        }
    }

    async fn next(&mut self) -> Option<anyhow::Result<Value>> {
        if self.overflowed.load(Ordering::SeqCst) {
            return Some(Err(
                CaptureFailure(OpenCodeV2SourceStatus::SourceLimit).into()
            ));
        }
        self.receiver.recv().await
    }

    async fn stop(mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = tokio::time::timeout(Duration::from_secs(1), &mut self.task).await;
    }
}

impl Drop for FeedPump {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.task.abort();
    }
}

#[cfg(test)]
#[path = "opencode_v2_runtime_tests.rs"]
mod runtime_tests;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use antiburn_local::analysis::{
        ToolResultPosition,
        opencode_v2_capture::{BoundaryKind, CapturedEvent},
    };
    use serde_json::{Value, json};

    use super::*;
    use crate::store::SessionRecord;

    #[test]
    fn database_identity_rejects_replacement_at_the_same_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("opencode.db");
        std::fs::write(&path, "first").unwrap();
        let identity = DatabaseIdentity::read(&path).unwrap();
        identity.verify().unwrap();
        std::fs::rename(&path, directory.path().join("old.db")).unwrap();
        std::fs::write(&path, "replacement").unwrap();
        assert!(identity.verify().is_err());
    }

    #[test]
    fn source_failures_keep_closed_actionable_statuses() {
        use opencode_live::stream::EventStreamError;
        use opencode_live::{DatabaseAssociationError, ProbeError, RegistrationError, SseError};
        for (error, expected) in [
            (
                anyhow::Error::new(RegistrationError::Io),
                OpenCodeV2SourceStatus::ServiceUnavailable,
            ),
            (
                anyhow::Error::new(ProbeError::HttpStatus(401)),
                OpenCodeV2SourceStatus::AuthenticationRejected,
            ),
            (
                anyhow::Error::new(DatabaseAssociationError::Binding),
                OpenCodeV2SourceStatus::SourceMismatch,
            ),
            (
                anyhow::Error::new(ProbeError::VersionMismatch),
                OpenCodeV2SourceStatus::VersionUnsupported,
            ),
            (
                anyhow::Error::new(EventStreamError::Sse(SseError::FrameTooLarge)),
                OpenCodeV2SourceStatus::SourceLimit,
            ),
        ] {
            assert_eq!(
                capture_failure_status(&error.context("capture failed")),
                expected
            );
        }
    }

    #[test]
    fn event_version_gap_releases_capture_and_saves_typed_status_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(directory.path()).unwrap();
        let now = unix_now();
        let record = record(now);
        let claim = capture_claim(&store, &record, now);
        let mut capture = OpenCodeV2CaptureEpoch::new(vec![record.key.session_id.clone()]).unwrap();
        capture.confirm_subscription().unwrap();
        capture
            .finish_hydration(HashMap::from([(record.key.session_id.clone(), Some(0))]))
            .unwrap();
        let mut active = ActiveCaptures {
            capture,
            lease: CaptureLease::new(&store, vec![claim.clone()]),
            claims_by_session: HashMap::from([(record.key.session_id.clone(), claim)]),
            joiners: HashMap::from([(record.key.session_id.clone(), OpenCodeV2EventJoiner::new())]),
            gapped_sessions: HashSet::new(),
        };
        let mut unknown = event(
            1,
            "unsupported",
            "session.step.started",
            json!({"assistantMessageID":"step"}),
        );
        unknown["durable"]["version"] = json!(99);
        assert!(active.observe(unknown, &store).unwrap().is_none());
        let (status, lease): (String, Option<i64>) = store
            .lock()
            .query_row(
                "SELECT d.code, c.lease_expires_at_epoch FROM opencode_v2_source_status d
              JOIN opencode_v2_capture c USING(environment_key, agent, session_id)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "version-unsupported");
        assert_eq!(lease, None);
        assert!(active.lease.claims.is_empty());
        assert!(active.gapped_sessions.contains(&record.key.session_id));
    }

    fn record(now: i64) -> SessionRecord {
        SessionRecord {
            key: SessionKey::new("native", "opencode", "ses_v2"),
            source_kind: "providerDb".into(),
            source_label: "opencode:ses_v2:open_code_v2_sqlite".into(),
            wsl_distro: None,
            title: Some("V2 test".into()),
            title_source: Some("explicit".into()),
            cwd: Some("/repo".into()),
            surface: "cli".into(),
            updated_at_epoch: Some(now),
            activity_cursor: "v2-test".into(),
            activity_source: "event".into(),
            subagent_count: 0,
            fork_parent_session_id: None,
            source_fingerprint: Some("v2-fingerprint".into()),
        }
    }

    fn event(sequence: u64, id: &str, event_type: &str, extra: Value) -> Value {
        let durable_version =
            if matches!(event_type, "session.tool.success" | "session.tool.failed") {
                2
            } else {
                1
            };
        let mut data = json!({"sessionID":"ses_v2"});
        data.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({
            "id": id,
            "type": event_type,
            "created": sequence as f64,
            "durable":{"aggregateID":"ses_v2", "seq":sequence, "version":durable_version},
            "data": data,
        })
    }

    fn enabled_live_activation(store: &Store, now: i64) {
        store
            .lock()
            .execute(
                "INSERT INTO burn_check_activation (
                    check_id, activation_scope_key, definition_revision, enabled,
                    generation, bootstrap_sequence, applied_sequence, updated_at_epoch)
                 VALUES ('test-check', ?1, 'rev-1', 1, 1, 0, 0, ?2)",
                rusqlite::params![
                    serde_json::to_string(&json!({
                        "environment":"native", "session_cwd":"/repo", "run":"Live"
                    }))
                    .unwrap(),
                    now,
                ],
            )
            .unwrap();
    }

    fn capture_claim(store: &Store, record: &SessionRecord, now: i64) -> OpenCodeV2CaptureClaim {
        store
            .upsert_sessions(
                std::slice::from_ref(record),
                &crate::agents::evidence_cohort(),
            )
            .unwrap();
        enabled_live_activation(store, now);
        let (presence, _) = store
            .session_presence_for_keys(std::slice::from_ref(&record.key))
            .unwrap();
        let source_generation = store
            .source_generation_batch(std::slice::from_ref(&record.key))
            .unwrap()[0]
            .unwrap();
        let baseline = HashMap::from([("ses_v2".to_owned(), Some(0))]);
        let start = OpenCodeV2CaptureStart {
            key: record.key.clone(),
            incarnation: presence[0].incarnation,
            source_generation,
            source_label: record.source_label.clone(),
            cwd: record.cwd.clone().unwrap(),
            baseline_sequence: baseline["ses_v2"],
        };
        store
            .start_opencode_v2_capture_epoch(&[start], "worker-test-epoch", now, LEASE_SECS)
            .unwrap()
            .unwrap()
            .remove(0)
    }

    #[test]
    fn live_event_join_publishes_v2_result_and_sequence_progress() {
        let store = Store::open_in_memory(Path::new("/tmp/opencode-v2-live-worker-test")).unwrap();
        let now = unix_now();
        let record = record(now);
        let claim = capture_claim(&store, &record, now);
        let claims = HashMap::from([(record.key.session_id.clone(), claim)]);
        let mut joiners =
            HashMap::from([(record.key.session_id.clone(), OpenCodeV2EventJoiner::new())]);
        let events = [
            event(
                1,
                "inbox-enqueued",
                "session.inbox.enqueued",
                json!({
                    "inboxID":"msg_user",
                    "item":{"type":"user","payload":{"text":"read this file","files":[],"agents":[],"skills":[]},"delivery":"queue"}
                }),
            ),
            event(
                2,
                "inbox-delivered",
                "session.inbox.delivered",
                json!({"inboxID":"msg_user"}),
            ),
            event(
                3,
                "step-started",
                "session.step.started",
                json!({"assistantMessageID":"msg_assistant"}),
            ),
            event(
                4,
                "tool-started",
                "session.tool.input.started",
                json!({"assistantMessageID":"msg_assistant","id":"call_read","name":"Read"}),
            ),
            event(
                5,
                "tool-called",
                "session.tool.called",
                json!({"assistantMessageID":"msg_assistant","id":"call_read","input":{"path":"src/main.rs"},"executed":true}),
            ),
            event(
                6,
                "tool-terminal",
                "session.tool.success",
                json!({"assistantMessageID":"msg_assistant","id":"call_read","content":[{"type":"text","text":"contents"}],"executed":true}),
            ),
        ];
        let mut terminal_outcome = ProcessResult::Advanced;
        for (sequence, event) in events.into_iter().enumerate() {
            terminal_outcome = process_captured_event(
                CapturedEvent {
                    session_id: record.key.session_id.clone(),
                    sequence: sequence as u64 + 1,
                    kind: BoundaryKind::Live,
                    event,
                },
                &mut joiners,
                &claims,
                &store,
            )
            .unwrap();
        }

        assert_eq!(
            terminal_outcome,
            ProcessResult::ResultPublished(record.key.clone())
        );
        let publications = store.native_tool_result_publications_after(0, 100).unwrap();
        let result = publications
            .iter()
            .find(|publication| {
                publication.outcome
                    == crate::store::native_tool_results::ToolResultPublicationOutcome::Update
            })
            .unwrap();
        assert_eq!(
            result.source_format.as_deref(),
            Some("\"open_code_v2_sqlite\"")
        );
        assert_eq!(result.live_eligible, Some(true));
        assert_eq!(
            result.payload.as_ref().unwrap().completeness,
            antiburn_local::analysis::ToolResultCompleteness::Incomplete
        );
        assert!(
            !result
                .payload
                .as_ref()
                .unwrap()
                .context
                .as_ref()
                .unwrap()
                .complete_through_event
        );
        assert_eq!(
            result.native_position,
            Some(ToolResultPosition::OpenCodeV2Event {
                sequence: 6,
                item_index: 0,
            })
        );
        let progress: (i64, i64) = store
            .lock()
            .query_row(
                "SELECT last_sequence, history_gap FROM opencode_v2_capture
                  WHERE environment_key='native' AND agent='opencode' AND session_id='ses_v2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(progress, (6, 1));
    }
}
