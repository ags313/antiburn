//! JetBrains Junie session discovery.
//!
//! Junie keeps one folder per session at
//! `~/.junie/sessions/session-<yymmdd>-<hhmmss>-<id>/`. The folder holds
//! `events.jsonl`, the session journal, beside `state.json`, `summary.json`,
//! `tasks.json`, `transcript.md`, `checkpoints/`, and task folders. Only
//! `events.jsonl` is a session source.
//!
//! The folder name is the session ID. The project directory and task name
//! come from `summary.json` when the folder has one. Older folders have no
//! `summary.json`; `~/.junie/sessions/index.jsonl` can hold the same keys
//! for them. Neither file tells whether an IDE or the CLI ran the session,
//! so the surface stays unknown.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use crate::discovery::scanner::{AgentKind, SessionMetadata, TitleSource};
use crate::discovery::{
    AgentExplorer, SessionLog, SessionSource, SurfacePaths, WatchRoot, home_dir, recent_files_named,
};

/// The only file in a session folder that discovery treats as a source.
const TRANSCRIPT_FILE_NAME: &str = "events.jsonl";
const SESSION_DIR_PREFIX: &str = "session-";
/// Upper read bounds for the metadata companions. Observed `summary.json`
/// files are near 1 KiB and an index line is near 300 bytes.
const SUMMARY_MAX_BYTES: u64 = 256 * 1024;
const INDEX_MAX_BYTES: u64 = 4 * 1024 * 1024;

pub struct JunieExplorer;

#[async_trait]
impl AgentExplorer for JunieExplorer {
    async fn discover_recent(&self, now: i64, since_secs: i64) -> Vec<SessionLog> {
        let Some(home) = home_dir() else {
            return Vec::new();
        };
        discover_recent_in(&home, now, since_secs).await
    }

    /// Owns `events.jsonl` in a session folder under `~/.junie/sessions/`.
    fn owns_path(&self, path_lower: &str) -> bool {
        path_lower.contains("/.junie/sessions/session-") && path_lower.ends_with("/events.jsonl")
    }

    fn surface_paths(&self, _home: &Path) -> SurfacePaths {
        SurfacePaths::default()
    }

    fn watch_roots(&self, home: &Path) -> Vec<WatchRoot> {
        vec![WatchRoot::recursive(sessions_dir_in(home))]
    }

    fn recover_session_id_from_path(&self, file: &Path) -> Option<String> {
        session_id_for(file)
    }
}

pub(crate) fn sessions_dir_in(home: &Path) -> PathBuf {
    home.join(".junie").join("sessions")
}

async fn discover_recent_in(home: &Path, now: i64, since_secs: i64) -> Vec<SessionLog> {
    let dirs = session_dirs_in(home).await;
    recent_files_named(&dirs, now, since_secs, TRANSCRIPT_FILE_NAME)
        .await
        .into_iter()
        .map(|file| SessionLog {
            agent_type: AgentKind::Junie,
            source: SessionSource::File(file.path),
            updated_at: Some(file.mtime_epoch),
            environment: Default::default(),
        })
        .collect()
}

async fn session_dirs_in(home: &Path) -> Vec<PathBuf> {
    let Ok(mut entries) = tokio::fs::read_dir(sessions_dir_in(home)).await else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let is_session = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(SESSION_DIR_PREFIX));
        if is_session && entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
            dirs.push(entry.path());
        }
    }
    dirs
}

/// The session ID is the name of the folder that holds `events.jsonl`.
fn session_id_for(file: &Path) -> Option<String> {
    if file.file_name()?.to_str()? != TRANSCRIPT_FILE_NAME {
        return None;
    }
    let name = file.parent()?.file_name()?.to_str()?;
    (name.len() > SESSION_DIR_PREFIX.len() && name.starts_with(SESSION_DIR_PREFIX))
        .then(|| name.to_owned())
}

/// Replaces transcript-derived metadata with the session folder's own record.
///
/// The folder name always gives the session ID. `summary.json` gives the
/// project directory and task name; `index.jsonl` gives them when the folder
/// has no usable summary. When neither names a project, the CWD stays as the
/// transcript gives it.
pub(crate) async fn augment_metadata(file: &Path, metadata: &mut SessionMetadata) {
    let Some(session_id) = session_id_for(file) else {
        return;
    };
    let session_dir = file.parent().map(Path::to_path_buf);
    let sessions_dir = session_dir
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let lookup_id = session_id.clone();
    let record = tokio::task::spawn_blocking(move || {
        session_dir
            .and_then(|dir| summary_record(&dir.join("summary.json"), &lookup_id))
            .or_else(|| {
                sessions_dir.and_then(|dir| index_record(&dir.join("index.jsonl"), &lookup_id))
            })
    })
    .await
    .ok()
    .flatten();

    metadata.session_id = Some(session_id);
    let Some(record) = record else {
        return;
    };
    if let Some(cwd) = record.project_dir {
        metadata.cwd = Some(cwd);
    }
    if let Some(title) = record.task_name {
        metadata.title = Some(title);
        metadata.title_source = Some(TitleSource::AiGenerated);
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SessionRecord {
    project_dir: Option<String>,
    task_name: Option<String>,
}

fn summary_record(path: &Path, session_id: &str) -> Option<SessionRecord> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(SUMMARY_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > SUMMARY_MAX_BYTES {
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    record_for(&value, session_id)
}

/// When the index holds more than one line for a session, the last line wins.
fn index_record(path: &Path, session_id: &str) -> Option<SessionRecord> {
    let file = std::fs::File::open(path).ok()?;
    BufReader::new(file.take(INDEX_MAX_BYTES))
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter_map(|value| record_for(&value, session_id))
        .last()
}

fn record_for(value: &Value, session_id: &str) -> Option<SessionRecord> {
    if value.get("sessionId").and_then(Value::as_str) != Some(session_id) {
        return None;
    }
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Some(SessionRecord {
        project_dir: text("projectDir"),
        task_name: text("taskName"),
    })
}

#[cfg(test)]
pub(crate) fn sample_log_path(home: &Path) -> PathBuf {
    sessions_dir_in(home)
        .join("session-261001-090000-syn1")
        .join(TRANSCRIPT_FILE_NAME)
}

#[cfg(test)]
#[path = "tests/junie_tests.rs"]
mod tests;
