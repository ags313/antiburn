use super::*;
use crate::discovery::{DirectSessionSource, Explorers};
use serial_test::serial;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn session_dir(home: &Path, name: &str) -> PathBuf {
    let dir = sessions_dir_in(home).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn discovers_only_events_files_in_session_folders() {
    let home = TempDir::new().unwrap();
    let dir = session_dir(home.path(), "session-261001-090000-syn1");
    std::fs::write(dir.join("events.jsonl"), "{}\n").unwrap();
    std::fs::write(dir.join("state.json"), "{}").unwrap();
    std::fs::write(dir.join("summary.json"), "{}").unwrap();
    let empty = session_dir(home.path(), "session-261001-090100-syn2");
    std::fs::write(empty.join("summary.json"), "{}").unwrap();
    let other = session_dir(home.path(), "checkpoints");
    std::fs::write(other.join("events.jsonl"), "{}\n").unwrap();
    std::fs::write(sessions_dir_in(home.path()).join("index.jsonl"), "{}\n").unwrap();

    let logs = discover_recent_in(home.path(), now(), 86_400).await;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].agent_type, AgentKind::Junie);
    assert!(matches!(
        &logs[0].source,
        SessionSource::File(path) if *path == dir.join("events.jsonl")
    ));
}

#[test]
fn owns_session_events_and_recovers_the_folder_id() {
    let path = sample_log_path(Path::new("/Users/test"));
    let lower = path.to_string_lossy().to_lowercase();
    assert!(JunieExplorer.owns_path(&lower));
    assert!(!JunieExplorer.owns_path("/users/test/.junie/sessions/index.jsonl"));
    assert!(
        !JunieExplorer
            .owns_path("/users/test/.junie/sessions/session-261001-090000-syn1/state.json")
    );
    assert_eq!(
        JunieExplorer.recover_session_id_from_path(&path).as_deref(),
        Some("session-261001-090000-syn1")
    );
    assert_eq!(
        session_id_for(Path::new("/x/.junie/sessions/session-/events.jsonl")),
        None
    );
}

#[tokio::test]
async fn summary_gives_the_project_and_task_name() {
    let home = TempDir::new().unwrap();
    let dir = session_dir(home.path(), "session-261001-090000-syn1");
    let file = dir.join("events.jsonl");
    std::fs::write(
        dir.join("summary.json"),
        r#"{"sessionId":"session-261001-090000-syn1","projectDir":"/work/synthetic","taskName":"Synthetic task","status":"Done"}"#,
    )
    .unwrap();
    std::fs::write(
        sessions_dir_in(home.path()).join("index.jsonl"),
        r#"{"sessionId":"session-261001-090000-syn1","projectDir":"/work/index","taskName":"Index task"}"#,
    )
    .unwrap();
    let mut metadata = SessionMetadata {
        cwd: Some("/work/synthetic/sub".to_owned()),
        ..SessionMetadata::default()
    };
    augment_metadata(&file, &mut metadata).await;
    assert_eq!(
        metadata.session_id.as_deref(),
        Some("session-261001-090000-syn1")
    );
    assert_eq!(metadata.cwd.as_deref(), Some("/work/synthetic"));
    assert_eq!(metadata.title.as_deref(), Some("Synthetic task"));
    assert_eq!(metadata.title_source, Some(TitleSource::AiGenerated));
}

#[tokio::test]
async fn index_gives_metadata_when_the_folder_has_no_summary() {
    let home = TempDir::new().unwrap();
    let dir = session_dir(home.path(), "session-260827-102108-syn2");
    let file = dir.join("events.jsonl");
    std::fs::write(
        sessions_dir_in(home.path()).join("index.jsonl"),
        concat!(
            r#"{"sessionId":"session-other","projectDir":"/work/other","taskName":"Other"}"#,
            "\n",
            "not json\n",
            r#"{"sessionId":"session-260827-102108-syn2","projectDir":"/work/old","taskName":"Old"}"#,
            "\n",
            r#"{"sessionId":"session-260827-102108-syn2","projectDir":"/work/new","taskName":"New"}"#,
            "\n",
        ),
    )
    .unwrap();
    let mut metadata = SessionMetadata::default();
    augment_metadata(&file, &mut metadata).await;
    assert_eq!(metadata.cwd.as_deref(), Some("/work/new"));
    assert_eq!(metadata.title.as_deref(), Some("New"));
}

#[tokio::test]
async fn a_mismatched_summary_keeps_the_transcript_cwd() {
    let home = TempDir::new().unwrap();
    let dir = session_dir(home.path(), "session-261001-090000-syn1");
    let file = dir.join("events.jsonl");
    std::fs::write(
        dir.join("summary.json"),
        r#"{"sessionId":"session-someone-else","projectDir":"/work/wrong"}"#,
    )
    .unwrap();
    let mut metadata = SessionMetadata {
        cwd: Some("/work/recorded".to_owned()),
        ..SessionMetadata::default()
    };
    augment_metadata(&file, &mut metadata).await;
    assert_eq!(
        metadata.session_id.as_deref(),
        Some("session-261001-090000-syn1")
    );
    assert_eq!(metadata.cwd.as_deref(), Some("/work/recorded"));
    assert_eq!(metadata.title, None);
}

#[tokio::test(flavor = "current_thread")]
#[serial]
async fn direct_session_source_finds_folder_id_and_misses_authoritatively() {
    let home = TempDir::new().unwrap();
    let old_home = std::env::var_os("HOME");
    // SAFETY: this test owns HOME for its duration.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    let session_id = "session-261015-120000-direct";
    let dir = session_dir(home.path(), session_id);
    let journal = dir.join("events.jsonl");
    std::fs::write(&journal, "{}\n").unwrap();

    // Local ids are always the session-* folder name.
    assert_eq!(
        JunieExplorer
            .recover_session_id_from_path(&journal)
            .as_deref(),
        Some(session_id)
    );
    assert_eq!(session_id_for(&journal).as_deref(), Some(session_id));

    match JunieExplorer.direct_session_source(session_id).await {
        DirectSessionSource::Found(SessionSource::File(path)) => assert_eq!(path, journal),
        other => panic!("expected found journal, got {other:?}"),
    }
    assert!(matches!(
        JunieExplorer
            .direct_session_source("session-261015-120000-missing")
            .await,
        DirectSessionSource::Missing
    ));
    assert!(
        Explorers::DISK
            .locate_session_source(&AgentKind::Junie, "session-261015-120000-missing")
            .await
            .is_none(),
        "authoritative Missing must not fall back into an empty discovery"
    );

    match old_home {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }
}
