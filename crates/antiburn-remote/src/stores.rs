//! Sessions from agent stores that the engine's own discovery finds.
//!
//! Cursor, OpenCode, and Antigravity sessions use the same discovery as a
//! local scan. Before an export reads a source, the helper admits the source
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
use crate::{LOOKBACK_SECS, MAX_CWD_CHARS, MAX_TITLE_CHARS, RemoteSession, now};

/// The longest that one agent's discovery can run in one request.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(20);
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
}

pub(crate) struct StoreEntry {
    pub(crate) session: RemoteSession,
    pub(crate) source: StoreSource,
}

#[derive(Default)]
pub(crate) struct StoreDiscovery {
    pub(crate) entries: Vec<StoreEntry>,
    pub(crate) skipped: usize,
    pub(crate) truncated: bool,
}

/// One file of a store bundle.
pub(crate) enum StorePayload {
    File(AdmittedFile),
    Bytes(Vec<u8>),
}

pub(crate) fn kind_for(agent: &str) -> Option<AgentKind> {
    match agent {
        "cursor" => Some(AgentKind::Cursor),
        "opencode" => Some(AgentKind::OpenCode),
        "antigravity" => Some(AgentKind::Antigravity),
        _ => None,
    }
}

pub(crate) async fn discover(agent: AgentKind) -> StoreDiscovery {
    let mut discovery = StoreDiscovery::default();
    let Some(home) = home_dir() else {
        return discovery;
    };
    let logs = match tokio::time::timeout(
        DISCOVERY_TIMEOUT,
        Explorers::DISK
            .get(&agent)
            .discover_recent(now(), LOOKBACK_SECS),
    )
    .await
    {
        Ok(logs) => logs,
        Err(_) => {
            discovery.truncated = true;
            return discovery;
        }
    };
    let cutoff = now().saturating_sub(LOOKBACK_SECS);
    for log in logs {
        if log.updated_at.is_none_or(|updated| updated < cutoff) {
            continue;
        }
        match entry_for(agent, &home, &log).await {
            Some(entry) => discovery.entries.push(entry),
            None => discovery.skipped += 1,
        }
    }
    discovery
}

async fn entry_for(agent: AgentKind, home: &Path, log: &SessionLog) -> Option<StoreEntry> {
    let source = store_source(agent, home, &log.source)?;
    let session_id = match &source {
        StoreSource::CursorTranscript { relative, .. } => {
            relative.file_stem()?.to_str()?.to_owned()
        }
        StoreSource::OpenCode { session_id, .. } | StoreSource::Antigravity { session_id, .. } => {
            session_id.clone()
        }
    };
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

/// Accept only the source shapes that the desktop can analyze from a copy.
fn store_source(agent: AgentKind, home: &Path, source: &SessionSource) -> Option<StoreSource> {
    match (agent, source) {
        (AgentKind::Cursor, SessionSource::Inline { label, content }) => {
            let root = home.join(".cursor").join("projects");
            let relative = Path::new(label).strip_prefix(&root).ok()?.to_path_buf();
            let mut components = relative.components();
            components.next()?;
            if components.next()?.as_os_str() != "agent-transcripts"
                || relative.extension()? != "jsonl"
            {
                return None;
            }
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
) -> Result<(RemoteSession, Vec<(StorePayload, Option<BundleCompanion>)>)> {
    let discovery = discover(agent).await;
    ensure!(
        !discovery.truncated,
        "Session was not resolved within the bounded recent-session scan"
    );
    let entry = discovery
        .entries
        .into_iter()
        .find(|entry| entry.session.session_id == session_id)
        .context("Session is no longer in the recent discovery window; refresh the host")?;
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
    };
    Ok((entry.session, files))
}
