//! Sessions from agent stores that the engine's own discovery finds.
//!
//! Cursor sessions use transcript candidates. Other stores use local discovery.
//! Before an export reads a source, the helper admits the source
//! file beneath its store root. An admitted file has no symlink component and
//! is a regular file.

use std::io::Read;
use std::path::{Path, PathBuf};

use antiburn_local::discovery::{
    DirectSessionSource, Explorers, SessionLog, SessionSource, home_dir,
};
use antiburn_local::model::AgentKind;
use anyhow::{Context, Result, bail, ensure};

use crate::evidence::{AdmittedFile, TrustedRoot};
use crate::export::{BundleCompanion, MAX_BUNDLE_BYTES, safe_identity};
use crate::{MAX_CWD_CHARS, MAX_TITLE_CHARS, RemoteSession, now};

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
    prepare_with(Explorers::DISK, agent, session_id, lookback_secs).await
}

/// Locate one session by id through `explorers` and admit its evidence.
///
/// OpenCode and Antigravity resolve a database row by id. Junie resolves the
/// journal path by folder id. Cursor uses
/// [`antiburn_local::discovery::agents::cursor::locate_agent_transcript_source`]
/// only (agent-transcript path shape), not
/// [`antiburn_local::discovery::AgentExplorer::direct_session_source`], so local
/// desktop locate keeps discovery priority and metadata merge. The helper
/// never runs a full recent discovery for the export.
pub(crate) async fn prepare_with(
    explorers: Explorers,
    agent: AgentKind,
    session_id: &str,
    lookback_secs: i64,
) -> Result<(RemoteSession, Vec<(StorePayload, Option<BundleCompanion>)>)> {
    ensure!(
        safe_identity(session_id),
        "Unsupported remote session identity"
    );
    let home = home_dir().context("Missing home directory")?;
    // Cursor: transcript-only locate for remote export. Does not carry
    // title-based fork observations or store.db `model` that listing-time
    // discovery merges in; listing and export metadata can differ.
    let source = match agent {
        AgentKind::Cursor => {
            match antiburn_local::discovery::agents::cursor::locate_agent_transcript_source(
                &home, session_id,
            )
            .await
            {
                Some(source) => source,
                None => {
                    bail!("Session is no longer in the recent discovery window; refresh the host");
                }
            }
        }
        _ => match explorers
            .get(&agent)
            .direct_session_source(session_id)
            .await
        {
            DirectSessionSource::Found(source) => source,
            DirectSessionSource::Missing | DirectSessionSource::Unsupported => {
                bail!("Session is no longer in the recent discovery window; refresh the host");
            }
        },
    };
    let updated_at = source_updated_at(&source).await;
    let cutoff = now().saturating_sub(lookback_secs);
    // Match listing: a missing mtime is outside the window.
    ensure!(
        !updated_at.is_none_or(|updated| updated < cutoff),
        "Session is no longer in the recent discovery window; refresh the host"
    );
    let log = SessionLog {
        environment: Default::default(),
        agent_type: agent,
        source,
        updated_at,
    };
    let entry = entry_from_log(agent, &home, &log)
        .await
        .context("Session is no longer in the recent discovery window; refresh the host")?;
    ensure!(
        entry.session.session_id == session_id,
        "Session is no longer in the recent discovery window; refresh the host"
    );
    let files = admit_store_source(entry.source).await?;
    Ok((entry.session, files))
}

async fn source_updated_at(source: &SessionSource) -> Option<i64> {
    match source {
        SessionSource::ProviderDb {
            agent: AgentKind::OpenCode,
            db_path,
            session_id,
        } => {
            // Same cluster activity timestamp listing uses. Do not use the
            // opencode.db filesystem mtime: a WAL write can leave that file old
            // while session rows stay recent.
            antiburn_local::discovery::agents::opencode::db_session_cluster_updated_at(
                db_path.clone(),
                session_id.clone(),
            )
            .await
        }
        SessionSource::ProviderDb {
            agent: AgentKind::Antigravity,
            db_path,
            session_id,
        } => {
            // Same combined db + WAL + brain transcript mtime listing uses.
            antiburn_local::discovery::agents::antigravity::db_session_updated_at(
                db_path.clone(),
                session_id.clone(),
            )
            .await
        }
        SessionSource::File(path) => file_mtime(path).await,
        SessionSource::Inline { label, .. } => file_mtime(Path::new(label)).await,
        SessionSource::ProviderDb { db_path, .. } => file_mtime(db_path).await,
    }
}

async fn file_mtime(path: &Path) -> Option<i64> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    let modified = metadata.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64,
    )
}

async fn admit_store_source(
    source: StoreSource,
) -> Result<Vec<(StorePayload, Option<BundleCompanion>)>> {
    Ok(match source {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use antiburn_local::discovery::AgentExplorer;
    use async_trait::async_trait;
    use std::sync::Mutex;

    const JUNIE_TARGET: &str = "session-261015-120000-target";
    const JUNIE_OTHER_A: &str = "session-261015-120000-other-a";
    const JUNIE_OTHER_B: &str = "session-261015-120000-other-b";
    const AG_TARGET: &str = "7d1f3c2a-1b5e-4c8d-9f0a-6e2b4a8c1d33";
    const AG_OTHER: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    #[derive(Default)]
    struct ReadCounts {
        discover_calls: usize,
        locate_calls: Vec<String>,
        file_reads: Vec<String>,
    }

    static READS: Mutex<ReadCounts> = Mutex::new(ReadCounts {
        discover_calls: 0,
        locate_calls: Vec::new(),
        file_reads: Vec::new(),
    });
    /// Serializes the locate-by-id tests that share `READS` and mutate `HOME`.
    static PREPARE_LOCATE_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn record_discover() {
        READS.lock().expect("reads lock").discover_calls += 1;
    }

    fn record_locate(session_id: &str) {
        READS
            .lock()
            .expect("reads lock")
            .locate_calls
            .push(session_id.to_owned());
    }

    fn record_file_read(label: &str) {
        READS
            .lock()
            .expect("reads lock")
            .file_reads
            .push(label.to_owned());
    }

    /// Restores the process `HOME` when dropped, including on panic.
    struct HomeGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl HomeGuard {
        fn set(home: &Path) -> Self {
            let previous = std::env::var_os("HOME");
            // SAFETY: the shared prepare-locate lock owns HOME for this test.
            unsafe {
                std::env::set_var("HOME", home);
            }
            Self { previous }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => unsafe { std::env::set_var("HOME", value) },
                None => unsafe { std::env::remove_var("HOME") },
            }
        }
    }

    struct CountingJunieExplorer;

    #[async_trait]
    impl AgentExplorer for CountingJunieExplorer {
        async fn discover_recent(&self, _now: i64, _since_secs: i64) -> Vec<SessionLog> {
            record_discover();
            Vec::new()
        }

        async fn direct_session_source(&self, session_id: &str) -> DirectSessionSource {
            record_locate(session_id);
            let home = home_dir().expect("home");
            let path = home
                .join(".junie")
                .join("sessions")
                .join(session_id)
                .join("events.jsonl");
            if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                record_file_read(&path.to_string_lossy());
                let _ = tokio::fs::read_to_string(&path).await;
                DirectSessionSource::Found(SessionSource::File(path))
            } else {
                DirectSessionSource::Missing
            }
        }

        fn owns_path(&self, path_lower: &str) -> bool {
            path_lower.contains("/.junie/sessions/session-")
                && path_lower.ends_with("/events.jsonl")
        }
    }

    struct CountingAntigravityExplorer;

    #[async_trait]
    impl AgentExplorer for CountingAntigravityExplorer {
        async fn discover_recent(&self, _now: i64, _since_secs: i64) -> Vec<SessionLog> {
            record_discover();
            Vec::new()
        }

        async fn direct_session_source(&self, session_id: &str) -> DirectSessionSource {
            record_locate(session_id);
            let home = home_dir().expect("home");
            let db_path = home
                .join(".gemini/antigravity-cli/conversations")
                .join(format!("{session_id}.db"));
            if tokio::fs::try_exists(&db_path).await.unwrap_or(false) {
                record_file_read(&db_path.to_string_lossy());
                // Touch the file so an unreadable sibling would fail if opened.
                let _ = tokio::fs::metadata(&db_path).await;
                DirectSessionSource::Found(SessionSource::ProviderDb {
                    agent: AgentKind::Antigravity,
                    db_path,
                    session_id: session_id.to_owned(),
                })
            } else {
                DirectSessionSource::Unsupported
            }
        }

        fn owns_path(&self, path_lower: &str) -> bool {
            path_lower.contains("/antigravity")
        }
    }

    fn counting_lookup(agent: &AgentKind) -> &'static dyn AgentExplorer {
        match agent {
            AgentKind::Junie => &CountingJunieExplorer,
            AgentKind::Antigravity => &CountingAntigravityExplorer,
            _ => panic!("unexpected agent {agent:?}"),
        }
    }

    fn write_junie_session(home: &Path, session_id: &str, body: &str) {
        let folder = home.join(".junie").join("sessions").join(session_id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("summary.json"),
            serde_json::json!({
                "sessionId": session_id,
                "projectDir": "/synthetic/repo",
                "taskName": session_id,
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(folder.join("events.jsonl"), body).unwrap();
    }

    fn write_antigravity_session(home: &Path, session_id: &str) {
        let root = home.join(".gemini/antigravity-cli");
        std::fs::create_dir_all(root.join("conversations")).unwrap();
        let db_path = root.join(format!("conversations/{session_id}.db"));
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA user_version = 1;
                 CREATE TABLE steps (idx INTEGER PRIMARY KEY, metadata BLOB);
                 CREATE TABLE gen_metadata (idx INTEGER PRIMARY KEY, data BLOB);
                 INSERT INTO steps (idx, metadata) VALUES (0, x'');",
            )
            .unwrap();
        std::mem::forget(connection);
        let logs = root.join(format!("brain/{session_id}/.system_generated/logs"));
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("transcript.jsonl"), "{\"type\":\"USER_INPUT\"}\n").unwrap();
    }

    #[cfg(unix)]
    fn make_unreadable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o000);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[tokio::test]
    async fn prepare_locate_by_id_reads_only_junie_target() {
        let _serialize = PREPARE_LOCATE_TEST.lock().await;
        let home = tempfile::tempdir().unwrap();
        let _home = HomeGuard::set(home.path());
        write_junie_session(home.path(), JUNIE_TARGET, "{\"kind\":\"target\"}\n");
        write_junie_session(home.path(), JUNIE_OTHER_A, "{\"kind\":\"other-a\"}\n");
        write_junie_session(home.path(), JUNIE_OTHER_B, "{\"kind\":\"other-b\"}\n");
        #[cfg(unix)]
        {
            make_unreadable(
                &home
                    .path()
                    .join(".junie/sessions")
                    .join(JUNIE_OTHER_A)
                    .join("events.jsonl"),
            );
            make_unreadable(
                &home
                    .path()
                    .join(".junie/sessions")
                    .join(JUNIE_OTHER_B)
                    .join("events.jsonl"),
            );
        }
        {
            let mut counts = READS.lock().expect("reads lock");
            *counts = ReadCounts::default();
        }

        let explorers = Explorers::new(counting_lookup);
        let (session, files) = prepare_with(
            explorers,
            AgentKind::Junie,
            JUNIE_TARGET,
            crate::LOOKBACK_SECS,
        )
        .await
        .expect("prepare target");
        assert_eq!(session.session_id, JUNIE_TARGET);
        assert_eq!(files.len(), 1);

        let counts = READS.lock().expect("reads lock");
        assert_eq!(counts.discover_calls, 0, "export must not run discovery");
        assert_eq!(
            counts.locate_calls,
            vec![JUNIE_TARGET.to_owned()],
            "export must locate only the target id"
        );
        assert!(
            counts
                .file_reads
                .iter()
                .all(|path| path.contains(JUNIE_TARGET)),
            "export must not open other session journals: {:?}",
            counts.file_reads
        );
    }

    #[tokio::test]
    async fn prepare_locate_by_id_reads_only_antigravity_target() {
        let _serialize = PREPARE_LOCATE_TEST.lock().await;
        let home = tempfile::tempdir().unwrap();
        let _home = HomeGuard::set(home.path());
        write_antigravity_session(home.path(), AG_TARGET);
        write_antigravity_session(home.path(), AG_OTHER);
        #[cfg(unix)]
        {
            make_unreadable(
                &home
                    .path()
                    .join(".gemini/antigravity-cli/conversations")
                    .join(format!("{AG_OTHER}.db")),
            );
        }
        {
            let mut counts = READS.lock().expect("reads lock");
            *counts = ReadCounts::default();
        }

        let explorers = Explorers::new(counting_lookup);
        let (session, files) = prepare_with(
            explorers,
            AgentKind::Antigravity,
            AG_TARGET,
            crate::LOOKBACK_SECS,
        )
        .await
        .expect("prepare target");
        assert_eq!(session.session_id, AG_TARGET);
        assert!(!files.is_empty());

        let counts = READS.lock().expect("reads lock");
        assert_eq!(counts.discover_calls, 0, "export must not run discovery");
        assert_eq!(
            counts.locate_calls,
            vec![AG_TARGET.to_owned()],
            "export must locate only the target id"
        );
        assert!(
            counts
                .file_reads
                .iter()
                .all(|path| path.contains(AG_TARGET)),
            "export must not open other session databases: {:?}",
            counts.file_reads
        );
    }
}
