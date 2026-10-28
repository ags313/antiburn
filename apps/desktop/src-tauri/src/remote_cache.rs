//! Private remote transcripts indexed by the ordinary evidence worker.

use crate::{
    analysis::{self, PassSignal},
    store::{FencedTurnRowStore, SessionKey, SessionRecord, Store},
};
use antiburn_local::{
    analysis::{SourceFormat, TurnRowStore},
    discovery::{
        Explorers, SessionSource,
        source_version::{
            FINGERPRINT_HEAD_BYTES, FingerprintInputs, SourceDescriptor, SourceStat, head_hash_of,
        },
    },
    model::AgentKind,
};
use antiburn_remote::{
    PROTOCOL_VERSION, RemoteSession, Request,
    export::{BundleManifest, MAX_BUNDLE_BYTES, MAX_MANIFEST_BYTES},
    remote_store::{RemoteStore, StoreFormat},
    transport,
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The cache generation directory that holds a remote session's files.
///
/// The first bundle file can sit below this directory, as an Antigravity
/// database does in `conversations/`. The store records the directory when a
/// sync commits it, so no code derives it from the path of the first file.
fn generation_dir(store: &Store, record: &SessionRecord) -> Option<PathBuf> {
    store.remote_cache_dir(&record.key).ok().flatten()
}

/// The source that analysis reads for a cached session. Database agents read
/// their cached database, and the other agents read their cached transcript.
fn cached_source(agent: AgentKind, session_id: &str, primary: PathBuf) -> SessionSource {
    match RemoteStore::for_slug(agent.slug()).map(|store| store.format) {
        Some(StoreFormat::Db) => SessionSource::ProviderDb {
            agent,
            db_path: primary,
            session_id: session_id.to_owned(),
            source_format: match agent {
                AgentKind::OpenCode => Some(SourceFormat::OpenCodeSqliteV1),
                AgentKind::Antigravity => Some(SourceFormat::AntigravitySqlite),
                _ => None,
            },
        },
        _ => SessionSource::File(primary),
    }
}

/// The fingerprint that analysis checks before it publishes. A database
/// source uses the engine's row fingerprint, and a file uses its stat and head.
async fn source_fingerprint(manifest: &BundleManifest, primary: &Path) -> Result<String> {
    let agent = crate::agents::kind_from_slug(&manifest.session.agent)
        .context("Unsupported remote agent")?;
    let source = cached_source(agent, &manifest.session.session_id, primary.to_path_buf());
    if !matches!(source, SessionSource::ProviderDb { .. }) {
        return parent_fingerprint(primary);
    }
    let descriptor = SourceDescriptor {
        agent,
        session_id: manifest.session.session_id.clone(),
        environment: Default::default(),
        source_format: analysis::source_format(agent, &source),
        source,
        surface: manifest.session.surface.clone(),
        updated_at_epoch: manifest.session.updated_at,
    };
    Explorers::DISK
        .source_version(&descriptor, None)
        .await
        .map(|version| version.fingerprint)
        .context("Cannot read the cached session database")
}

pub fn manifest_for(store: &Store, record: &SessionRecord) -> Result<(PathBuf, BundleManifest)> {
    ensure!(
        record.key.remote_host_id().is_some(),
        "Not a remote session"
    );
    let dir = generation_dir(store, record).context("Missing transcript directory")?;
    let path = dir.join("manifest.json");
    ensure!(
        fs::metadata(&path)?.len() <= MAX_MANIFEST_BYTES as u64,
        "Manifest exceeds limit"
    );
    let manifest: BundleManifest = serde_json::from_slice(&fs::read(path)?)?;
    manifest.validate()?;
    ensure!(
        manifest.session.agent == record.key.agent
            && manifest.session.session_id == record.key.session_id,
        "Cached transcript identity mismatch"
    );
    Ok((dir, manifest))
}

fn parent_fingerprint(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let stat = SourceStat::from_open_std_file(&file).context("Cannot stat transcript")?;
    let mut head = Vec::new();
    (&mut file)
        .take(FINGERPRINT_HEAD_BYTES as u64)
        .read_to_end(&mut head)?;
    Ok(FingerprintInputs {
        stat,
        head_hash: Some(head_hash_of(&head)),
    }
    .fingerprint())
}

fn safe_cache_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub fn unpack(
    bundle: &Path,
    target: &Path,
    expected: &RemoteSession,
) -> Result<(BundleManifest, bool)> {
    let mut input = std::io::BufReader::new(fs::File::open(bundle)?);
    let mut magic = [0u8; 8];
    input.read_exact(&mut magic)?;
    ensure!(
        &magic == b"ABR2SAME" || &magic == b"ABR2DATA",
        "Update the remote helper to support transcript syncing"
    );
    let mut length = [0u8; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(length <= MAX_MANIFEST_BYTES, "Manifest exceeds limit");
    let mut bytes = vec![0u8; length];
    input.read_exact(&mut bytes)?;
    let manifest: BundleManifest = serde_json::from_slice(&bytes)?;
    manifest.validate()?;
    ensure!(
        manifest.session.agent == expected.agent
            && manifest.session.session_id == expected.session_id,
        "Exported session identity mismatch"
    );
    let unchanged = &magic == b"ABR2SAME";
    if !unchanged {
        for (index, descriptor) in manifest.files.iter().enumerate() {
            let path = target.join(manifest.file_name(index));
            private_dir(path.parent().context("Missing cache directory")?)?;
            let mut options = fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            ensure!(
                std::io::copy(&mut (&mut input).take(descriptor.size), &mut file)?
                    == descriptor.size,
                "Incomplete transcript transfer"
            );
            file.sync_all()?;
        }
        fs::write(target.join("manifest.json"), bytes)?;
    }
    let mut trailing = [0u8; 1];
    ensure!(
        input.read(&mut trailing)? == 0,
        "Unexpected data after transcript bundle"
    );
    Ok((manifest, unchanged))
}

fn cache_size(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut size = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            size = size.saturating_add(cache_size(&entry.path())?);
        } else if kind.is_file() {
            size = size.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(size)
}

fn declared_bundle_size(path: &Path) -> Result<(u64, u64)> {
    let mut input = std::io::BufReader::new(fs::File::open(path)?);
    let mut magic = [0_u8; 8];
    input.read_exact(&mut magic)?;
    ensure!(
        &magic == b"ABR2SAME" || &magic == b"ABR2DATA",
        "Invalid transcript bundle"
    );
    let mut length = [0_u8; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(length <= MAX_MANIFEST_BYTES, "Manifest exceeds limit");
    let mut bytes = vec![0_u8; length];
    input.read_exact(&mut bytes)?;
    let manifest: BundleManifest = serde_json::from_slice(&bytes)?;
    manifest.validate()?;
    let files = manifest.files.iter().try_fold(0_u64, |total, file| {
        total.checked_add(file.size).context("Bundle size overflow")
    })?;
    Ok((files, length as u64))
}

const CACHE_LIMIT_BYTES: u64 = 14 * 1024 * 1024 * 1024;
const STAGING_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn staging_projection_fits(
    existing: u64,
    bundle: u64,
    extracted_files: u64,
    extracted_manifest: u64,
) -> bool {
    existing
        .saturating_add(bundle)
        .saturating_add(extracted_files)
        .saturating_add(extracted_manifest)
        <= STAGING_LIMIT_BYTES
}

fn generation_name() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncResult {
    Imported,
    Unchanged,
}

/// True when every file of a cached bundle is present at its recorded size.
fn bundle_intact(dir: &Path, manifest: &BundleManifest) -> bool {
    manifest.files.iter().enumerate().all(|(index, file)| {
        fs::metadata(dir.join(manifest.file_name(index))).is_ok_and(|meta| meta.len() == file.size)
    })
}

/// True when the cache holds an intact copy of a listed session that had no
/// activity in the default window. A long history window lists many such
/// sessions, and they seldom change, so a scan does not export them again.
/// A change to the listing entry, such as a new modification time, still
/// causes an export.
fn settled_copy(store: &Store, record: &SessionRecord, session: &RemoteSession, now: i64) -> bool {
    session
        .updated_at
        .is_some_and(|updated| updated < now.saturating_sub(antiburn_remote::LOOKBACK_SECS))
        && manifest_for(store, record).is_ok_and(|(dir, manifest)| {
            manifest.session == *session && bundle_intact(&dir, &manifest)
        })
}

/// One session from a listing, with the history window of that listing.
/// `lookback_secs` is `None` for the default window.
pub struct ListedSession<'a> {
    pub platform: &'a str,
    pub session: &'a RemoteSession,
    pub lookback_secs: Option<i64>,
}

pub async fn sync_session(
    app: &tauri::AppHandle,
    root: &Path,
    host_id: &str,
    ssh_alias: &str,
    scan_generation: u64,
    listed: ListedSession<'_>,
    store: &Store,
) -> Result<SyncResult> {
    let ListedSession {
        platform,
        session,
        lookback_secs,
    } = listed;
    let key = SessionKey::for_origin(&session.agent, &session.session_id, None, Some(host_id))
        .map_err(anyhow::Error::msg)?;
    let previous = store.session(&key)?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    if previous
        .as_ref()
        .is_some_and(|record| settled_copy(store, record, session, now))
    {
        return Ok(SyncResult::Unchanged);
    }
    let cache_root = root.join("transcripts");
    let staging_root = root.join("staging");
    private_dir(&staging_root)?;
    let cache_root_for_size = cache_root.clone();
    let staging_for_size = staging_root.clone();
    let (cache_before, staging_before) = tokio::task::spawn_blocking(move || {
        Ok::<_, anyhow::Error>((
            cache_size(&cache_root_for_size)?,
            cache_size(&staging_for_size)?,
        ))
    })
    .await??;
    ensure!(
        cache_before < CACHE_LIMIT_BYTES,
        "Remote cache reached 14 GiB; remove an unused host in Settings before syncing more sessions"
    );
    ensure!(
        staging_before
            .saturating_add(MAX_BUNDLE_BYTES)
            .saturating_add(MAX_MANIFEST_BYTES as u64)
            .saturating_add(12)
            <= STAGING_LIMIT_BYTES,
        "Remote staging reached 2 GiB; wait for another remote scan to finish"
    );
    ensure!(
        cache_before
            .saturating_add(MAX_BUNDLE_BYTES)
            .saturating_add(MAX_MANIFEST_BYTES as u64)
            <= CACHE_LIMIT_BYTES,
        "Remote cache reached 14 GiB; remove an unused host in Settings before syncing more sessions"
    );
    let known = previous
        .as_ref()
        .and_then(|record| manifest_for(store, record).ok())
        .filter(|(dir, manifest)| bundle_intact(dir, manifest))
        .and_then(|(_, manifest)| manifest.signature().ok());
    let parent = cache_parent(root, &key)?;
    private_dir(&parent)?;
    let retained = previous
        .as_ref()
        .and_then(|record| generation_dir(store, record));
    clear_before_export(&parent, previous.is_some(), retained.as_deref())?;
    let staging = staging_root.join(generation_name()?);
    let staging_cleanup = staging.clone();
    private_dir(&staging)?;
    let bundle = staging.join("transfer.bin");
    let result = async {
        transport::export_to(
            ssh_alias,
            Some(platform),
            &Request::Export {
                version: PROTOCOL_VERSION,
                agent: session.agent.clone(),
                session_id: session.session_id.clone(),
                known,
                lookback_secs,
            },
            &bundle,
        )
        .await?;
        let bundle_size = fs::metadata(&bundle)?.len();
        let (declared_size, manifest_size) = declared_bundle_size(&bundle)?;
        ensure!(
            staging_projection_fits(
                staging_before,
                bundle_size,
                declared_size,
                manifest_size,
            ),
            "Remote staging reached 2 GiB; wait for another remote scan to finish"
        );
        let bundle_copy = bundle.clone();
        let target_copy = staging.clone();
        let expected = session.clone();
        let (manifest, unchanged) =
            tokio::task::spawn_blocking(move || unpack(&bundle_copy, &target_copy, &expected))
                .await??;
        fs::remove_file(&bundle)?;
        if unchanged {
            let old = previous.as_ref().context("Missing existing snapshot")?;
            let (old_dir, old_manifest) = manifest_for(store, old)?;
            ensure!(
                manifest.signature()? == old_manifest.signature()?,
                "Unchanged response does not match the cached snapshot"
            );
            ensure!(
                bundle_intact(&old_dir, &old_manifest),
                "Cached transcript is incomplete; remove and re-add the host to resync"
            );
            fs::remove_dir_all(&staging)?;
            return Ok(SyncResult::Unchanged);
        }
        ensure!(
            crate::remote_sync::is_current(app, host_id, scan_generation),
            "scan cancelled"
        );
        let staged_size = cache_size(&staging)?;
        ensure!(
            staging_before.saturating_add(staged_size) <= STAGING_LIMIT_BYTES,
            "Remote staging reached 2 GiB; wait for another remote scan to finish"
        );
        ensure!(
            cache_before.saturating_add(staged_size) <= CACHE_LIMIT_BYTES,
            "Remote cache reached 14 GiB; remove an unused host in Settings before syncing more sessions"
        );
        let target = parent.join(generation_name()?);
        let path = target.join(manifest.file_name(0));
        let staged_path = staging.join(manifest.file_name(0));
        let record = SessionRecord {
            key,
            source_kind: "file".to_owned(),
            source_label: path.to_string_lossy().into_owned(),
            wsl_distro: None,
            title: Some(manifest.session.title.clone()),
            title_source: Some("indexed".to_owned()),
            cwd: manifest.session.cwd.clone(),
            surface: manifest.session.surface.clone(),
            updated_at_epoch: manifest.session.updated_at,
            activity_cursor: manifest.signature()?,
            activity_source: "mtime".to_owned(),
            subagent_count: manifest
                .files
                .iter()
                .filter(|file| file.subagent_id.is_some())
                .count() as u32,
            fork_parent_session_id: manifest.fork_parent_session_id.clone(),
            source_fingerprint: Some(source_fingerprint(&manifest, &staged_path).await?),
        };
        crate::remote_sync::with_commit_guard(app, host_id, scan_generation, || {
            fs::rename(&staging, &target)?;
            let key = record.key.clone();
            let committed = store
                .upsert_sessions(&[record], antiburn_remote::SUPPORTED_AGENTS)
                .and_then(|_| store.set_remote_cache_dir(&key, &target));
            if let Err(error) = committed {
                if let Err(cleanup) = fs::remove_dir_all(&target) {
                    return Err(error.context(format!(
                        "Cache rollback failed and needs lifecycle cleanup: {cleanup}"
                    )));
                }
                return Err(error);
            }
            Ok(())
        })?;
        let _ = remove_generations_except(&parent, Some(&target));
        Ok(SyncResult::Imported)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging_cleanup);
    }
    result
}

/// Remove stray generations before an export. A stored row without a recorded
/// directory is a copy from an earlier build. Keep its files until the
/// replacement commits, because the export can fail.
fn clear_before_export(parent: &Path, stored: bool, recorded: Option<&Path>) -> Result<()> {
    if stored && recorded.is_none() {
        return Ok(());
    }
    remove_generations_except(parent, recorded)
}

/// Remove every generation directory below `parent`, except `keep`.
fn remove_generations_except(parent: &Path, keep: Option<&Path>) -> Result<()> {
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && keep != Some(entry.path().as_path()) {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

fn prepare_fork_parent(
    store: &Store,
    record: &SessionRecord,
    dir: &Path,
    manifest: &BundleManifest,
    parent: Option<&str>,
) -> Result<bool> {
    let Some(parent) = parent.filter(|_| record.key.agent == "claude-code") else {
        return Ok(false);
    };
    ensure!(
        parent != record.key.session_id,
        "A session cannot be its own fork parent"
    );
    ensure!(safe_cache_identity(parent), "Invalid fork parent identity");
    if manifest.fork_parent_session_id.as_deref() == Some(parent)
        && manifest.files.iter().any(|file| file.fork_parent)
    {
        return Ok(false);
    }
    let key = SessionKey::new(&record.key.environment_key, &record.key.agent, parent);
    let Some(parent_record) = store.session(&key)? else {
        return Ok(false);
    };
    let _ = manifest_for(store, &parent_record)?;
    let target = dir.join(format!("{parent}.jsonl"));
    if parent_fingerprint(&target).ok()
        == Some(parent_fingerprint(Path::new(&parent_record.source_label))?)
    {
        return Ok(false);
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = dir.join(format!("fork-parent-{nonce}.tmp"));
    // Immutable cached generations can share the parent file without duplicating its content.
    fs::hard_link(&parent_record.source_label, &temp)?;
    if let Err(error) = fs::rename(&temp, target) {
        let _ = fs::remove_file(temp);
        return Err(error.into());
    }
    Ok(true)
}

pub fn restore_fork_companions(store: &Store) -> Result<usize> {
    let mut repaired = 0;
    for record in store.recent_sessions(0, 10_000)? {
        if record.key.remote_host_id().is_none() || record.key.agent != "claude-code" {
            continue;
        }
        let Some(parent) = store.fork_parent(&record.key)? else {
            continue;
        };
        let repair = manifest_for(store, &record).and_then(|(dir, manifest)| {
            prepare_fork_parent(store, &record, &dir, &manifest, Some(&parent))
        });
        match repair {
            Ok(true) => {
                store.requeue_session_evidence(&record.key)?;
                repaired += 1;
            }
            Ok(false) => {}
            Err(error) => {
                if error.downcast_ref::<rusqlite::Error>().is_some() {
                    return Err(error);
                }
                tracing::warn!(event = "remote_fork_repair_failed", error = %error);
            }
        }
    }
    Ok(repaired)
}

pub fn prune_after_commit(store: &Store, root: &Path) {
    if let Err(error) = prune_unreferenced(store, root) {
        tracing::warn!(event = "remote_cache_cleanup_failed", error = %error);
    }
}

pub fn delete_session(
    store: &Store,
    root: Option<&Path>,
    key: &SessionKey,
) -> Result<Option<(crate::store::Incarnation, crate::store::Revision)>> {
    ensure!(key.remote_host_id().is_some(), "Not a remote session");
    let record = store.session(key)?;
    // The row holds the directory, so read it before the row goes.
    let directory = store.remote_cache_dir(key)?;
    let removed = store.delete_session(key)?;
    if record.is_some() {
        let cleanup = root
            .context("Remote cache directory is unavailable")
            .and_then(|root| remove_cached_generation(root, key, directory));
        if let Err(error) = cleanup {
            tracing::warn!(event = "remote_cache_cleanup_failed", error = %error);
        }
    }
    Ok(removed)
}

fn cache_parent(root: &Path, key: &SessionKey) -> Result<PathBuf> {
    let host_id = key.remote_host_id().context("Not a remote session")?;
    ensure!(safe_cache_identity(host_id), "Invalid remote host identity");
    let identity = Sha256::digest(format!("{}\0{}", key.agent, key.session_id).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(root.join("transcripts").join(host_id).join(identity))
}

fn remove_cached_generation(
    root: &Path,
    key: &SessionKey,
    directory: Option<PathBuf>,
) -> Result<()> {
    let directory = directory.context("Missing transcript directory")?;
    let canonical = match fs::canonicalize(directory) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let transcripts = fs::canonicalize(root.join("transcripts"))?;
    let expected = fs::canonicalize(cache_parent(root, key)?)?;
    ensure!(
        expected.starts_with(&transcripts),
        "Remote cache path escaped storage"
    );
    ensure!(
        canonical.parent() == Some(expected.as_path()),
        "Remote cache identity mismatch"
    );
    fs::remove_dir_all(canonical)?;
    Ok(())
}

pub fn prune_unreferenced(store: &Store, root: &Path) -> Result<usize> {
    let recorded = store.remote_cache_dirs()?;
    // A copy from an earlier build has no recorded directory. Keep every
    // directory until a sync replaces that copy.
    if recorded.iter().any(Option::is_none) {
        return Ok(0);
    }
    let retained = recorded
        .into_iter()
        .flatten()
        .collect::<std::collections::HashSet<_>>();
    let transcripts = root.join("transcripts");
    if !transcripts.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for host in fs::read_dir(transcripts)? {
        let host = host?;
        if !host.file_type()?.is_dir() {
            continue;
        }
        for identity in fs::read_dir(host.path())? {
            let identity = identity?;
            if !identity.file_type()?.is_dir() {
                continue;
            }
            for generation in fs::read_dir(identity.path())? {
                let generation = generation?;
                if generation.file_type()?.is_dir() && !retained.contains(&generation.path()) {
                    fs::remove_dir_all(generation.path())?;
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

pub fn cleanup_staging(root: &Path) -> Result<()> {
    let staging = root.join("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    private_dir(&staging)
}

pub fn run_pass(
    record: SessionRecord,
    signal: PassSignal,
    fence: i64,
    store: Store,
) -> crate::insights_worker::PassFuture {
    Box::pin(async move {
        let Ok((dir, manifest)) = manifest_for(&store, &record) else {
            return analysis::unsupported_evidence_pass();
        };
        let Some(agent) = crate::agents::kind_from_slug(&record.key.agent) else {
            return analysis::unsupported_evidence_pass();
        };
        let children = manifest
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                file.subagent_id.as_ref().map(|id| {
                    (
                        id.clone(),
                        file.label.clone().unwrap_or_else(|| "Sub-agent".to_owned()),
                        dir.join(manifest.file_name(index)),
                    )
                })
            })
            .collect();
        let parent = store.fork_parent(&record.key).ok().flatten();
        if prepare_fork_parent(&store, &record, &dir, &manifest, parent.as_deref()).is_err() {
            return analysis::unavailable_evidence_pass(
                analysis::PassOutcome::Unreadable(analysis::UnreadableReason::ClaimFailed),
                None,
                None,
            );
        }
        let writer: Arc<dyn TurnRowStore> =
            Arc::new(FencedTurnRowStore::new(store, record.key.clone(), fence));
        crate::insights_worker::run_pass_with_writer(signal.clone(), writer, |writer| {
            Box::pin(async move {
                analysis::analyze_located_for_evidence(
                    agent,
                    &record.key.session_id,
                    analysis::ClaimedSource {
                        fingerprint: record.source_fingerprint,
                        generation: 0,
                    },
                    signal,
                    Some(writer),
                    parent,
                    analysis::LocatedTranscripts {
                        source: cached_source(
                            agent,
                            &record.key.session_id,
                            PathBuf::from(record.source_label),
                        ),
                        children,
                    },
                )
                .await
            })
        })
        .await
    })
}

#[cfg(test)]
mod tests;
