//! Sessions from agent stores that the engine's own discovery finds.
//!
//! Cursor sessions use transcript candidates. Other stores use local discovery.
//! Before an export reads a source, the helper admits the source
//! file beneath its store root. An admitted file has no symlink component and
//! is a regular file.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use antiburn_local::discovery::{Explorers, SessionLog, SessionSource, home_dir};
use antiburn_local::model::AgentKind;
use anyhow::{Context, Result, ensure};

use crate::evidence::{AdmittedFile, TrustedRoot};
use crate::export::{BundleCompanion, MAX_BUNDLE_BYTES, safe_identity};
use crate::{MAX_CWD_CHARS, MAX_TITLE_CHARS, RemoteSession, now};

const PREPARE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(20);
const NEUTRAL_SESSION_TITLE: &str = "Untitled session";

pub(crate) enum StoreSource {
    /// A Cursor CLI agent transcript, with the metadata line that local
    /// discovery adds before it.
    CursorTranscript {
        root: PathBuf,
        relative: PathBuf,
        content: String,
    },
    OpenCode {
        database: PathBuf,
        session_id: String,
    },
    /// `root` holds `conversations/` and `brain/`.
    Antigravity { root: PathBuf, session_id: String },
    /// A Junie session journal: `relative` is
    /// `session-<id>/events.jsonl` below `~/.junie/sessions`.
    JunieJournal { root: PathBuf, relative: PathBuf },
}

pub(crate) struct StoreEntry {
    pub(crate) session: RemoteSession,
    pub(crate) source: StoreSource,
}

#[derive(Default)]
pub(crate) struct StoreDiscovery {
    pub(crate) logs: Vec<SessionLog>,
    pub(crate) skipped: usize,
    pub(crate) truncated: bool,
}

/// One file of a store bundle.
pub(crate) enum StorePayload {
    File(AdmittedFile),
    Bytes(Vec<u8>),
}

pub(crate) async fn discover(agent: AgentKind, lookback_secs: i64) -> StoreDiscovery {
    let mut discovery = StoreDiscovery::default();
    let Some(home) = home_dir() else {
        return discovery;
    };
    let logs = if agent == AgentKind::Cursor {
        antiburn_local::discovery::agents::cursor::agent_transcript_candidates(
            &home,
            now(),
            lookback_secs,
        )
        .await
    } else {
        Explorers::DISK
            .get(&agent)
            .discover_recent(now(), lookback_secs)
            .await
    };
    let cutoff = now().saturating_sub(lookback_secs);
    for log in logs {
        if log.updated_at.is_none_or(|updated| updated < cutoff) {
            continue;
        }
        if candidate_session_id(agent, &home, &log.source).is_none_or(|id| !safe_identity(&id)) {
            discovery.skipped += 1;
            continue;
        }
        discovery.logs.push(log);
    }
    discovery
}

pub(crate) async fn entries_for(logs: Vec<SessionLog>) -> Vec<RemoteSession> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let (cursor, mut logs): (Vec<_>, Vec<_>) = logs
        .into_iter()
        .partition(|log| log.agent_type == AgentKind::Cursor);
    if !cursor.is_empty() {
        logs.extend(
            antiburn_local::discovery::agents::cursor::read_selected_agent_transcripts(
                &home, cursor,
            )
            .await,
        );
    }
    let mut entries = Vec::new();
    for log in logs {
        if let Some(entry) = entry_from_log(log.agent_type, &home, &log).await {
            entries.push(entry.session);
        }
    }
    entries
}

async fn entry_for(log: SessionLog) -> Option<StoreEntry> {
    let home = home_dir()?;
    let agent = log.agent_type;
    let log = if agent == AgentKind::Cursor && matches!(log.source, SessionSource::File(_)) {
        antiburn_local::discovery::agents::cursor::read_selected_agent_transcripts(&home, vec![log])
            .await
            .into_iter()
            .next()?
    } else {
        log
    };
    entry_from_log(agent, &home, &log).await
}

async fn entry_from_log(agent: AgentKind, home: &Path, log: &SessionLog) -> Option<StoreEntry> {
    let source = store_source(agent, home, &log.source)?;
    let session_id = source_session_id(&source)?;
    if !safe_identity(&session_id) {
        return None;
    }
    // The same metadata read as a local scan, including the database title
    // and working directory lookups.
    let read = antiburn_local::discovery::session_log_read(log).await;
    let preview = read
        .as_ref()
        .and_then(|read| read.content.clone())
        .unwrap_or_default();
    let metadata = read.map(|read| read.metadata).unwrap_or_default();
    let title = metadata
        .title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| NEUTRAL_SESSION_TITLE.to_owned());
    let surface = match log.surface_label_with_content(&preview, home) {
        surface @ ("cli" | "ide_desktop") => surface,
        _ => "unknown",
    };
    Some(StoreEntry {
        session: RemoteSession {
            agent: agent.slug().to_owned(),
            session_id,
            title: title.chars().take(MAX_TITLE_CHARS).collect(),
            cwd: metadata
                .cwd
                .map(|cwd| cwd.chars().take(MAX_CWD_CHARS).collect()),
            surface: surface.to_owned(),
            updated_at: log.updated_at,
        },
        source,
    })
}

fn source_session_id(source: &StoreSource) -> Option<String> {
    Some(match source {
        StoreSource::CursorTranscript { relative, .. } => {
            relative.file_stem()?.to_str()?.to_owned()
        }
        StoreSource::OpenCode { session_id, .. } | StoreSource::Antigravity { session_id, .. } => {
            session_id.clone()
        }
        StoreSource::JunieJournal { relative, .. } => {
            relative.parent()?.file_name()?.to_str()?.to_owned()
        }
    })
}

fn cursor_relative(home: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(home.join(".cursor/projects")).ok()?;
    let mut components = relative.components();
    components.next()?;
    if components.next()?.as_os_str() != "agent-transcripts" || relative.extension()? != "jsonl" {
        return None;
    }
    Some(relative.to_path_buf())
}

fn candidate_session_id(agent: AgentKind, home: &Path, source: &SessionSource) -> Option<String> {
    if let (AgentKind::Cursor, SessionSource::File(path)) = (agent, source) {
        return cursor_relative(home, path)?
            .file_stem()?
            .to_str()
            .map(str::to_owned);
    }
    source_session_id(&store_source(agent, home, source)?)
}

/// Accept only the source shapes that the desktop can analyze from a copy.
fn store_source(agent: AgentKind, home: &Path, source: &SessionSource) -> Option<StoreSource> {
    match (agent, source) {
        (AgentKind::Cursor, SessionSource::Inline { label, content }) => {
            let root = home.join(".cursor").join("projects");
            let relative = cursor_relative(home, Path::new(label))?;
            Some(StoreSource::CursorTranscript {
                root,
                relative,
                content: content.clone(),
            })
        }
        (
            AgentKind::OpenCode,
            SessionSource::ProviderDb {
                db_path,
                session_id,
                ..
            },
        ) if db_path.file_name()? == "opencode.db" => Some(StoreSource::OpenCode {
            database: db_path.clone(),
            session_id: session_id.clone(),
        }),
        (
            AgentKind::Antigravity,
            SessionSource::ProviderDb {
                db_path,
                session_id,
                ..
            },
        ) => {
            let conversations = db_path.parent()?;
            if conversations.file_name()? != "conversations"
                || db_path.extension()? != "db"
                || db_path.file_stem()?.to_str()? != session_id
            {
                return None;
            }
            Some(StoreSource::Antigravity {
                root: conversations.parent()?.to_path_buf(),
                session_id: session_id.clone(),
            })
        }
        (AgentKind::Junie, SessionSource::File(path)) => {
            let root = home.join(".junie").join("sessions");
            let relative = path.strip_prefix(&root).ok()?.to_path_buf();
            let mut components = relative.components();
            let folder = components.next()?.as_os_str().to_str()?;
            let file = components.next()?.as_os_str();
            if components.next().is_some()
                || !folder.starts_with("session-")
                || file != "events.jsonl"
            {
                return None;
            }
            Some(StoreSource::JunieJournal { root, relative })
        }
        _ => None,
    }
}

/// The relative path of an Antigravity brain transcript.
pub(crate) fn antigravity_transcript(session_id: &str) -> PathBuf {
    Path::new("brain")
        .join(session_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl")
}

/// Find one listed session again and read its evidence.
pub(crate) async fn prepare(
    agent: AgentKind,
    session_id: &str,
    lookback_secs: i64,
) -> Result<(RemoteSession, Vec<(StorePayload, Option<BundleCompanion>)>)> {
    prepare_with_discovery(
        agent,
        session_id,
        lookback_secs,
        PREPARE_DISCOVERY_TIMEOUT,
        discover,
    )
    .await
}

async fn prepare_with_discovery<F, Fut>(
    agent: AgentKind,
    session_id: &str,
    lookback_secs: i64,
    timeout: Duration,
    discover_store: F,
) -> Result<(RemoteSession, Vec<(StorePayload, Option<BundleCompanion>)>)>
where
    F: Fn(AgentKind, i64) -> Fut,
    Fut: std::future::Future<Output = StoreDiscovery>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let discovery = tokio::time::timeout_at(deadline, discover_store(agent, lookback_secs))
        .await
        .context("Session was not resolved within the bounded recent-session scan")?;
    ensure!(
        !discovery.truncated,
        "Session was not resolved within the bounded recent-session scan"
    );
    let home = home_dir().context("Home directory is unavailable")?;
    let log = discovery
        .logs
        .into_iter()
        .find(|log| candidate_session_id(agent, &home, &log.source).as_deref() == Some(session_id))
        .context("Session is no longer in the recent discovery window; refresh the host")?;
    let entry = tokio::time::timeout_at(deadline, entry_for(log))
        .await
        .context("Session was not resolved within the bounded recent-session scan")?
        .context("Session metadata is no longer available")?;
    let files = match entry.source {
        StoreSource::CursorTranscript {
            root,
            relative,
            content,
        } => {
            let admitted = TrustedRoot::open(&root)?.admit(&relative)?;
            ensure!(
                admitted.stat.size <= MAX_BUNDLE_BYTES,
                "Bundle exceeds 1 GiB"
            );
            let mut bytes = Vec::new();
            (&admitted.file)
                .take(admitted.stat.size.saturating_add(1))
                .read_to_end(&mut bytes)?;
            admitted.validate_unchanged()?;
            // Local discovery adds one metadata line before the transcript.
            // The copy must hold the admitted bytes after that line.
            let transcript = content.split_once('\n').map_or("", |(_, rest)| rest);
            ensure!(
                transcript.as_bytes() == bytes.as_slice(),
                "Session changed during collection; retry"
            );
            vec![(StorePayload::Bytes(content.into_bytes()), None)]
        }
        StoreSource::OpenCode {
            database,
            session_id,
        } => {
            let root = TrustedRoot::open(database.parent().context("Missing database root")?)?;
            let admitted = root.admit(Path::new("opencode.db"))?;
            let path = root.path().join("opencode.db");
            let bytes = tokio::task::spawn_blocking(move || {
                crate::database::opencode_extract(&path, &admitted, &session_id)
            })
            .await??;
            vec![(StorePayload::Bytes(bytes), None)]
        }
        StoreSource::Antigravity { root, session_id } => {
            let root = TrustedRoot::open(&root)?;
            let relative = Path::new("conversations").join(format!("{session_id}.db"));
            let admitted = root.admit(&relative)?;
            let transcript_relative = antigravity_transcript(&session_id);
            let transcript = match std::fs::symlink_metadata(root.path().join(&transcript_relative))
            {
                Ok(_) => Some(root.admit(&transcript_relative)?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            let path = root.path().join(&relative);
            let bytes =
                tokio::task::spawn_blocking(move || crate::database::snapshot(&path, &admitted))
                    .await??;
            let mut files = vec![(StorePayload::Bytes(bytes), None)];
            if let Some(transcript) = transcript {
                files.push((
                    StorePayload::File(transcript),
                    Some(BundleCompanion::AntigravityBrain),
                ));
            }
            files
        }
        StoreSource::JunieJournal { root, relative } => {
            let admitted = TrustedRoot::open(&root)?.admit(&relative)?;
            ensure!(
                admitted.stat.size <= MAX_BUNDLE_BYTES,
                "Bundle exceeds 1 GiB"
            );
            vec![(StorePayload::File(admitted), None)]
        }
    };
    Ok((entry.session, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prepare_reports_bounded_scan_error_when_store_discovery_times_out() {
        let started = std::time::Instant::now();
        let result = prepare_with_discovery(
            AgentKind::Junie,
            "session-fixture",
            crate::LOOKBACK_SECS,
            Duration::from_millis(100),
            |agent, lookback_secs| async move {
                assert_eq!(agent, AgentKind::Junie);
                assert_eq!(lookback_secs, crate::LOOKBACK_SECS);
                tokio::time::sleep(Duration::from_secs(5)).await;
                StoreDiscovery::default()
            },
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(1));
        let Err(error) = result else {
            panic!("Slow discovery must fail with the bounded-scan error");
        };
        assert_eq!(
            error.to_string(),
            "Session was not resolved within the bounded recent-session scan"
        );
    }
}
