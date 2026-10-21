//! Cursor, OpenCode, Antigravity, and Junie sessions through the real helper
//! binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use antiburn_remote::export::{BundleCompanion, BundleManifest};
use antiburn_remote::{PROTOCOL_VERSION, Snapshot};
use rusqlite::Connection;
use serde_json::{Value, json};

const CURSOR_ID: &str = "0b4b6a2e-6c1f-4d55-9a57-2f3c1d9e8a11";
const ANTIGRAVITY_ID: &str = "7d1f3c2a-1b5e-4c8d-9f0a-6e2b4a8c1d33";
const OPENCODE_ROOT: &str = "ses_root";
const OPENCODE_CHILD: &str = "ses_child";
const OPENCODE_OTHER: &str = "ses_other";
const JUNIE_ID: &str = "session-261015-120000-syn1";
const ALL_AGENTS: [&str; 6] = [
    "claude-code",
    "codex",
    "cursor",
    "opencode",
    "antigravity",
    "junie",
];

fn opencode_root(home: &Path) -> PathBuf {
    home.join("opencode-data")
}

fn call(home: &Path, request: Value) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_antiburn-remote"))
        .arg("stdio")
        .env("HOME", home)
        .env("CODEX_HOME", home.join(".codex"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("GEMINI_HOME", home.join(".gemini"))
        .env("OPENCODE_DATA_DIR", opencode_root(home))
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&request).unwrap())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn write_cursor(home: &Path) -> PathBuf {
    let directory = home.join(format!(
        ".cursor/projects/home-synthetic-project/agent-transcripts/{CURSOR_ID}"
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{CURSOR_ID}.jsonl"));
    std::fs::write(
        &path,
        concat!(
            "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Fix the build\"}]}}\n",
            "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Done\"}]}}\n"
        ),
    )
    .unwrap();
    path
}

fn write_opencode(home: &Path) {
    let root = opencode_root(home);
    std::fs::create_dir_all(&root).unwrap();
    let connection = Connection::open(root.join("opencode.db")).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE `project` (`id` text PRIMARY KEY, `worktree` text NOT NULL);
             INSERT INTO `project` VALUES ('global', '/synthetic/repo');
             CREATE TABLE `session` (
                 `id` text PRIMARY KEY, `project_id` text NOT NULL, `parent_id` text,
                 `slug` text NOT NULL, `directory` text NOT NULL, `title` text NOT NULL,
                 `version` text NOT NULL, `time_created` integer NOT NULL,
                 `time_updated` integer NOT NULL,
                 CONSTRAINT `fk_session_project_id_project_id_fk` FOREIGN KEY (`project_id`)
                     REFERENCES `project`(`id`) ON DELETE CASCADE);
             CREATE TABLE `message` (
                 `id` text PRIMARY KEY, `session_id` text NOT NULL,
                 `time_created` integer NOT NULL, `time_updated` integer NOT NULL,
                 `data` text NOT NULL,
                 CONSTRAINT `fk_message_session_id_session_id_fk` FOREIGN KEY (`session_id`)
                     REFERENCES `session`(`id`) ON DELETE CASCADE);
             CREATE TABLE `part` (
                 `id` text PRIMARY KEY, `message_id` text NOT NULL, `session_id` text NOT NULL,
                 `time_created` integer NOT NULL, `time_updated` integer NOT NULL,
                 `data` text NOT NULL,
                 CONSTRAINT `fk_part_message_id_message_id_fk` FOREIGN KEY (`message_id`)
                     REFERENCES `message`(`id`) ON DELETE CASCADE);
             CREATE TABLE `event` (`id` text PRIMARY KEY, `data` text NOT NULL);
             CREATE INDEX `message_session_idx` ON `message` (`session_id`, `time_created`, `id`);
             CREATE INDEX `part_message_id_id_idx` ON `part` (`message_id`, `id`);",
        )
        .unwrap();
    let now = now_ms();
    for (id, parent, title) in [
        (OPENCODE_ROOT, None, "Synthetic root"),
        (OPENCODE_CHILD, Some(OPENCODE_ROOT), "Synthetic child"),
        (OPENCODE_OTHER, None, "Unrelated session"),
    ] {
        connection
            .execute(
                "INSERT INTO session VALUES (?1, 'global', ?2, ?1, '/synthetic/repo', ?3, '1.19.0', ?4, ?4)",
                rusqlite::params![id, parent, title, now],
            )
            .unwrap();
        for (index, role) in ["user", "assistant"].into_iter().enumerate() {
            let message = format!("msg_{id}_{index}");
            let data = json!({
                "id": message,
                "role": role,
                "sessionID": id,
                "providerID": "anthropic",
                "modelID": "claude-sonnet-4-6",
                "tokens": {"input": 100, "output": 20, "reasoning": 0, "cache": {"read": 50, "write": 0}},
                "time": {"created": now + index as i64}
            });
            connection
                .execute(
                    "INSERT INTO message VALUES (?1, ?2, ?3, ?3, ?4)",
                    rusqlite::params![message, id, now + index as i64, data.to_string()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                    rusqlite::params![
                        format!("prt_{message}"),
                        message,
                        id,
                        now + index as i64,
                        json!({"type": "text", "text": format!("{role} text")}).to_string()
                    ],
                )
                .unwrap();
        }
    }
    connection
        .execute(
            "INSERT INTO event VALUES ('evt_1', 'large unrelated history')",
            [],
        )
        .unwrap();
    // The connection stays open in a real store. Keep the write-ahead log
    // alive, so the export must read committed rows through it.
    std::mem::forget(connection);
}

fn write_antigravity(home: &Path) {
    let root = home.join(".gemini/antigravity-cli");
    std::fs::create_dir_all(root.join("conversations")).unwrap();
    let connection =
        Connection::open(root.join(format!("conversations/{ANTIGRAVITY_ID}.db"))).unwrap();
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
    let logs = root.join(format!("brain/{ANTIGRAVITY_ID}/.system_generated/logs"));
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join("transcript.jsonl"),
        "{\"type\":\"USER_INPUT\",\"content\":\"Review the plan\"}\n",
    )
    .unwrap();
}

/// A synthetic Junie session folder with one usage record and its summary.
fn write_junie(home: &Path) -> PathBuf {
    let folder = home.join(".junie/sessions").join(JUNIE_ID);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("summary.json"),
        json!({
            "sessionId": JUNIE_ID,
            "projectDir": "/synthetic/repo",
            "taskName": "Synthetic Junie task",
        })
        .to_string(),
    )
    .unwrap();
    let journal = folder.join("events.jsonl");
    let usage = json!({
        "kind": "SessionA2uxEvent",
        "taskId": "task-261015-120000-syn1",
        "timestampMs": now_ms(),
        "event": {"state": "IN_PROGRESS", "agentEvent": {
            "kind": "LlmResponseMetadataEvent",
            "agent": {"kind": "MainAgent", "id": "main", "name": "main", "type": "LINEAR"},
            "modelUsage": [{"model": "gpt-5.4-nano", "cost": 0.00002, "inputTokens": 150,
                "cacheInputTokens": 0, "cacheCreateTokens": 0, "outputTokens": 7, "time": 0}],
        }},
    });
    std::fs::write(&journal, format!("{usage}\n")).unwrap();
    journal
}

fn list(home: &Path, agents: Option<&[&str]>) -> Snapshot {
    list_window(home, agents, None)
}

/// A list request with an optional history window.
fn list_window(home: &Path, agents: Option<&[&str]>, lookback_secs: Option<i64>) -> Snapshot {
    let mut request = json!({"operation": "list", "version": PROTOCOL_VERSION});
    if let Some(agents) = agents {
        request["agents"] = json!(agents);
    }
    if let Some(lookback_secs) = lookback_secs {
        request["lookback_secs"] = json!(lookback_secs);
    }
    let output = call(home, request);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: Snapshot = serde_json::from_slice(&output.stdout).unwrap();
    snapshot
        .validate_for(antiburn_remote::lookback_window(lookback_secs))
        .unwrap();
    snapshot
}

struct Bundle {
    manifest: BundleManifest,
    same: bool,
    files: Vec<Vec<u8>>,
}

fn export(home: &Path, agent: &str, session_id: &str, known: Option<&str>) -> Bundle {
    let output = call(
        home,
        json!({
            "operation": "export",
            "version": PROTOCOL_VERSION,
            "agent": agent,
            "session_id": session_id,
            "known": known,
        }),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = output.stdout;
    let same = match &bytes[..8] {
        b"ABR2SAME" => true,
        b"ABR2DATA" => false,
        _ => panic!("unexpected bundle magic"),
    };
    let length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let manifest: BundleManifest = serde_json::from_slice(&bytes[12..12 + length]).unwrap();
    manifest.validate().unwrap();
    let mut offset = 12 + length;
    let mut files = Vec::new();
    if !same {
        for file in &manifest.files {
            let end = offset + file.size as usize;
            files.push(bytes[offset..end].to_vec());
            offset = end;
        }
    }
    assert_eq!(offset, bytes.len(), "no data after the bundle");
    Bundle {
        manifest,
        same,
        files,
    }
}

fn open_copy(bytes: &[u8]) -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("copy.db");
    std::fs::write(&path, bytes).unwrap();
    let connection =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    (directory, connection)
}

#[test]
fn list_adds_store_agents_only_when_the_desktop_selects_them() {
    let home = tempfile::tempdir().unwrap();
    write_cursor(home.path());
    write_opencode(home.path());
    write_antigravity(home.path());
    write_junie(home.path());

    // An older desktop sends no selection and receives only the base agents.
    assert!(list(home.path(), None).sessions.is_empty());

    let snapshot = list(home.path(), Some(&ALL_AGENTS));
    let mut found: Vec<(&str, &str)> = snapshot
        .sessions
        .iter()
        .map(|session| (session.agent.as_str(), session.session_id.as_str()))
        .collect();
    found.sort_unstable();
    assert_eq!(
        found,
        [
            ("antigravity", ANTIGRAVITY_ID),
            ("cursor", CURSOR_ID),
            ("junie", JUNIE_ID),
            ("opencode", OPENCODE_OTHER),
            ("opencode", OPENCODE_ROOT),
        ]
    );
    let junie = snapshot
        .sessions
        .iter()
        .find(|session| session.agent == "junie")
        .unwrap();
    assert_eq!(junie.title, "Synthetic Junie task");
    assert_eq!(junie.cwd.as_deref(), Some("/synthetic/repo"));
    let root = snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == OPENCODE_ROOT)
        .unwrap();
    assert_eq!(root.title, "Synthetic root");
    assert_eq!(root.cwd.as_deref(), Some("/synthetic/repo"));

    let only_cursor = list(home.path(), Some(&["cursor"]));
    assert_eq!(only_cursor.sessions.len(), 1);
    assert_eq!(only_cursor.sessions[0].agent, "cursor");
}

#[test]
fn list_rejects_an_unknown_or_repeated_agent_selection() {
    let home = tempfile::tempdir().unwrap();
    for agents in [json!(["other"]), json!(["cursor", "cursor"]), json!([])] {
        let output = call(
            home.path(),
            json!({"operation": "list", "version": PROTOCOL_VERSION, "agents": agents}),
        );
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn every_listed_cursor_transcript_exports_when_a_desktop_composer_has_the_same_id() {
    let home = tempfile::tempdir().unwrap();
    let transcript = write_cursor(home.path());
    let user = antiburn_local::discovery::app_config_dir_in("Cursor", home.path()).join("User");
    let global = user.join("globalStorage");
    let workspace = user.join("workspaceStorage/synthetic");
    std::fs::create_dir_all(&global).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let database = Connection::open(global.join("state.vscdb")).unwrap();
    database
        .execute_batch("CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT);")
        .unwrap();
    for (key, value) in [
        (
            format!("composerData:{CURSOR_ID}"),
            json!({
                "composerId": CURSOR_ID,
                "name": "Desktop composer title",
                "createdAt": now_ms() - 1000,
                "lastUpdatedAt": now_ms(),
                "fullConversationHeadersOnly": [{"bubbleId": "u1", "type": 1}]
            }),
        ),
        (
            format!("bubbleId:{CURSOR_ID}:u1"),
            json!({"type": 1, "text": "Desktop question"}),
        ),
    ] {
        database
            .execute(
                "INSERT INTO cursorDiskKV VALUES (?1, ?2)",
                rusqlite::params![key, value.to_string()],
            )
            .unwrap();
    }
    let database = Connection::open(workspace.join("state.vscdb")).unwrap();
    database
        .execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT);")
        .unwrap();
    database.execute("INSERT INTO ItemTable VALUES ('composer.composerData', ?1)", [json!({
        "allComposers": [{"composerId": CURSOR_ID, "lastUpdatedAt": now_ms(), "createdAt": now_ms() - 1000}]
    }).to_string()]).unwrap();
    // Keep the CWD absent in both sources so local discovery uses the same key.
    std::fs::write(workspace.join("workspace.json"), json!({}).to_string()).unwrap();

    let snapshot = list(home.path(), Some(&["cursor"]));
    assert_eq!(snapshot.sessions.len(), 1);
    assert_eq!(snapshot.sessions[0].session_id, CURSOR_ID);
    for session in snapshot.sessions {
        let bundle = export(home.path(), "cursor", &session.session_id, None);
        assert_eq!(
            serde_json::to_value(&bundle.manifest.session).unwrap(),
            serde_json::to_value(&session).unwrap()
        );
        let text = String::from_utf8(bundle.files[0].clone()).unwrap();
        assert_eq!(
            text.split_once('\n').unwrap().1,
            std::fs::read_to_string(&transcript).unwrap()
        );
    }
}

#[test]
fn exports_the_cursor_transcript_with_its_discovery_metadata() {
    let home = tempfile::tempdir().unwrap();
    let path = write_cursor(home.path());
    let bundle = export(home.path(), "cursor", CURSOR_ID, None);
    assert!(!bundle.same);
    assert_eq!(bundle.manifest.files.len(), 1);
    assert_eq!(bundle.manifest.file_name(0), format!("{CURSOR_ID}.jsonl"));
    let text = String::from_utf8(bundle.files[0].clone()).unwrap();
    let (metadata, transcript) = text.split_once('\n').unwrap();
    let metadata: Value = serde_json::from_str(metadata).unwrap();
    assert_eq!(metadata["sessionId"], CURSOR_ID);
    assert_eq!(metadata["cursor_source"], "agent_transcript");
    assert_eq!(transcript, std::fs::read_to_string(&path).unwrap());

    let signature = bundle.manifest.signature().unwrap();
    assert!(export(home.path(), "cursor", CURSOR_ID, Some(&signature)).same);

    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"More\"}]}}\n")
        .unwrap();
    assert!(!export(home.path(), "cursor", CURSOR_ID, Some(&signature)).same);
}

#[test]
fn cursor_export_resolves_workspace_json_cwd_when_decode_cannot() {
    let home = tempfile::tempdir().unwrap();
    // Path does not exist on disk, so the filesystem decoder cannot recover it.
    let ghost_cwd = "/Users/ghost-antiburn-caf3-never-created";
    let workspace_key = "Users-ghost-antiburn-caf3-never-created";
    let transcript_dir = home.path().join(format!(
        ".cursor/projects/{workspace_key}/agent-transcripts/{CURSOR_ID}"
    ));
    std::fs::create_dir_all(&transcript_dir).unwrap();
    std::fs::write(
        transcript_dir.join(format!("{CURSOR_ID}.jsonl")),
        "{\"role\":\"user\"}\n",
    )
    .unwrap();
    let meta_dir = home
        .path()
        .join(".cursor/chats/workspacehash")
        .join(CURSOR_ID);
    std::fs::create_dir_all(&meta_dir).unwrap();
    std::fs::write(
        meta_dir.join("meta.json"),
        json!({
            "title": "Ghost workspace task",
            "createdAtMs": 1_700_000_000_000i64,
            "updatedAtMs": 1_700_000_100_000i64,
        })
        .to_string(),
    )
    .unwrap();
    // Create Cursor workspaceStorage shapes for macOS, Linux, and Windows so
    // the fixture works on the host that runs this test.
    let workspace_roots = [
        home.path()
            .join("Library/Application Support/Cursor/User/workspaceStorage/ghost-ws"),
        home.path()
            .join(".config/Cursor/User/workspaceStorage/ghost-ws"),
        home.path()
            .join("AppData/Roaming/Cursor/User/workspaceStorage/ghost-ws"),
    ];
    for root in &workspace_roots {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("workspace.json"),
            json!({ "folder": format!("file://{ghost_cwd}") }).to_string(),
        )
        .unwrap();
    }

    let snapshot = list(home.path(), Some(&["cursor"]));
    let listed = snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == CURSOR_ID)
        .expect("listing must include the cursor session");
    assert_eq!(listed.cwd.as_deref(), Some(ghost_cwd));

    let bundle = export(home.path(), "cursor", CURSOR_ID, None);
    assert_eq!(
        bundle.manifest.session.cwd, listed.cwd,
        "locate-by-id must match listing cwd via this key's workspace.json"
    );
    let metadata: Value = serde_json::from_str(
        std::str::from_utf8(&bundle.files[0])
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["cwd"], ghost_cwd);
    assert_eq!(metadata["title"], "Ghost workspace task");
}

#[test]
fn opencode_export_uses_session_activity_not_database_file_mtime() {
    const DAY: i64 = 24 * 60 * 60;
    let home = tempfile::tempdir().unwrap();
    write_opencode(home.path());
    let db_path = opencode_root(home.path()).join("opencode.db");
    let thirty_days_ago =
        std::time::SystemTime::now() - std::time::Duration::from_secs(30 * DAY as u64);
    // The main database file is old (WAL may still be recent). Listing uses
    // session/message activity, so the session stays in the default window.
    std::fs::File::options()
        .write(true)
        .open(&db_path)
        .unwrap()
        .set_modified(thirty_days_ago)
        .unwrap();

    let snapshot = list(home.path(), Some(&["opencode"]));
    let listed = snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == OPENCODE_ROOT)
        .expect("listing must keep a session with recent row activity");
    let listed_updated = listed.updated_at.expect("listing updated_at");

    let bundle = export(home.path(), "opencode", OPENCODE_ROOT, None);
    assert_eq!(
        bundle.manifest.session.updated_at,
        Some(listed_updated),
        "export must reuse the listing cluster activity timestamp"
    );
}

#[test]
fn antigravity_export_uses_combined_mtime_not_database_file_mtime() {
    const DAY: i64 = 24 * 60 * 60;
    let home = tempfile::tempdir().unwrap();
    write_antigravity(home.path());
    let db_path = home.path().join(format!(
        ".gemini/antigravity-cli/conversations/{ANTIGRAVITY_ID}.db"
    ));
    let transcript = home.path().join(format!(
        ".gemini/antigravity-cli/brain/{ANTIGRAVITY_ID}/.system_generated/logs/transcript.jsonl"
    ));
    let thirty_days_ago =
        std::time::SystemTime::now() - std::time::Duration::from_secs(30 * DAY as u64);
    // Main .db is old; WAL and/or brain transcript stay recent. Listing uses
    // max(db, wal, transcript), so the session stays in the default window.
    std::fs::File::options()
        .write(true)
        .open(&db_path)
        .unwrap()
        .set_modified(thirty_days_ago)
        .unwrap();
    let wal_path = {
        let mut path = db_path.as_os_str().to_os_string();
        path.push("-wal");
        std::path::PathBuf::from(path)
    };
    if wal_path.exists() {
        std::fs::File::options()
            .write(true)
            .open(&wal_path)
            .unwrap()
            .set_modified(std::time::SystemTime::now())
            .unwrap();
    }
    std::fs::File::options()
        .write(true)
        .open(&transcript)
        .unwrap()
        .set_modified(std::time::SystemTime::now())
        .unwrap();

    let snapshot = list(home.path(), Some(&["antigravity"]));
    let listed = snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == ANTIGRAVITY_ID)
        .expect("listing must keep a session with recent WAL or transcript");
    let listed_updated = listed.updated_at.expect("listing updated_at");

    let bundle = export(home.path(), "antigravity", ANTIGRAVITY_ID, None);
    assert_eq!(
        bundle.manifest.session.updated_at,
        Some(listed_updated),
        "export must reuse the listing combined db/WAL/transcript mtime"
    );
}

#[test]
fn exports_one_opencode_session_cluster_as_a_small_database() {
    let home = tempfile::tempdir().unwrap();
    write_opencode(home.path());
    let bundle = export(home.path(), "opencode", OPENCODE_ROOT, None);
    assert_eq!(bundle.manifest.files.len(), 1);
    assert_eq!(bundle.manifest.file_name(0), "opencode.db");
    // The copy has no write-ahead log, so its header selects the rollback
    // journal.
    assert_eq!(&bundle.files[0][18..20], &[1, 1]);

    let (_directory, copy) = open_copy(&bundle.files[0]);
    let ids = |sql: &str| -> Vec<String> {
        copy.prepare(sql)
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert_eq!(
        ids("SELECT id FROM session ORDER BY id"),
        [OPENCODE_CHILD, OPENCODE_ROOT]
    );
    assert_eq!(
        ids("SELECT DISTINCT session_id FROM message ORDER BY session_id"),
        [OPENCODE_CHILD, OPENCODE_ROOT]
    );
    assert_eq!(ids("SELECT id FROM part").len(), 4);
    // Tables that the reader does not use stay on the host.
    assert!(
        copy.prepare("SELECT 1 FROM sqlite_master WHERE name = 'event'")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );

    // Equal source rows give equal bytes, so an unchanged session sends no
    // data.
    let signature = bundle.manifest.signature().unwrap();
    assert!(export(home.path(), "opencode", OPENCODE_ROOT, Some(&signature)).same);
}

#[test]
fn exports_the_antigravity_database_and_its_brain_transcript() {
    let home = tempfile::tempdir().unwrap();
    write_antigravity(home.path());
    let bundle = export(home.path(), "antigravity", ANTIGRAVITY_ID, None);
    assert_eq!(bundle.manifest.files.len(), 2);
    assert_eq!(
        bundle.manifest.file_name(0),
        format!("conversations/{ANTIGRAVITY_ID}.db")
    );
    assert_eq!(
        bundle.manifest.files[1].companion,
        Some(BundleCompanion::AntigravityBrain)
    );
    assert_eq!(
        bundle.manifest.file_name(1),
        format!("brain/{ANTIGRAVITY_ID}/.system_generated/logs/transcript.jsonl")
    );
    let (_directory, copy) = open_copy(&bundle.files[0]);
    let version: i64 = copy
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
    let steps: i64 = copy
        .query_row("SELECT COUNT(*) FROM steps", [], |row| row.get(0))
        .unwrap();
    assert_eq!(steps, 1);

    let signature = bundle.manifest.signature().unwrap();
    assert!(export(home.path(), "antigravity", ANTIGRAVITY_ID, Some(&signature)).same);
}

#[test]
fn exports_the_junie_journal_in_its_session_folder() {
    let home = tempfile::tempdir().unwrap();
    let journal = write_junie(home.path());
    let bundle = export(home.path(), "junie", JUNIE_ID, None);
    assert!(!bundle.same);
    assert_eq!(bundle.manifest.files.len(), 1);
    // The desktop copy keeps the folder name, which gives the session ID.
    assert_eq!(
        bundle.manifest.file_name(0),
        format!("{JUNIE_ID}/events.jsonl")
    );
    assert_eq!(bundle.files[0], std::fs::read(&journal).unwrap());

    let signature = bundle.manifest.signature().unwrap();
    assert!(export(home.path(), "junie", JUNIE_ID, Some(&signature)).same);

    std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap()
        .write_all(b"{\"kind\":\"TaskStartedEvent\",\"taskId\":\"t\",\"timestampMs\":1}\n")
        .unwrap();
    assert!(!export(home.path(), "junie", JUNIE_ID, Some(&signature)).same);
}

#[test]
fn does_not_list_a_junie_file_outside_a_session_folder() {
    let home = tempfile::tempdir().unwrap();
    let stray = home.path().join(".junie/sessions/not-a-session");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("events.jsonl"), "{}\n").unwrap();
    assert!(list(home.path(), Some(&["junie"])).sessions.is_empty());
}

#[test]
fn a_history_window_lists_and_exports_an_older_session() {
    const DAY: i64 = 24 * 60 * 60;
    let home = tempfile::tempdir().unwrap();
    let journal = write_junie(home.path());
    let twenty_days_ago =
        std::time::SystemTime::now() - std::time::Duration::from_secs(20 * DAY as u64);
    std::fs::File::options()
        .write(true)
        .open(&journal)
        .unwrap()
        .set_modified(twenty_days_ago)
        .unwrap();

    // The default window is seven days, so it does not list the session.
    assert!(list(home.path(), Some(&["junie"])).sessions.is_empty());
    let history = list_window(home.path(), Some(&["junie"]), Some(30 * DAY));
    assert_eq!(history.lookback_secs, 30 * DAY);
    assert_eq!(history.sessions.len(), 1);
    assert_eq!(history.sessions[0].session_id, JUNIE_ID);

    let export = |lookback_secs: Option<i64>| {
        call(
            home.path(),
            json!({
                "operation": "export",
                "version": PROTOCOL_VERSION,
                "agent": "junie",
                "session_id": JUNIE_ID,
                "known": null,
                "lookback_secs": lookback_secs,
            }),
        )
    };
    assert!(!export(None).status.success());
    let output = export(Some(30 * DAY));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(&output.stdout[..8], b"ABR2DATA");
}

#[test]
fn a_window_outside_the_protocol_bounds_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    for lookback_secs in [60, antiburn_remote::MAX_LOOKBACK_SECS + 1] {
        let output = call(
            home.path(),
            json!({"operation": "list", "version": PROTOCOL_VERSION, "lookback_secs": lookback_secs}),
        );
        assert!(!output.status.success());
    }
}
