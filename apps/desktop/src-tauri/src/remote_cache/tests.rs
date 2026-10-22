use super::*;
use antiburn_remote::export::{BundleCompanion, BundleFile};

fn manifest(data: &[u8]) -> BundleManifest {
    BundleManifest {
        session: RemoteSession {
            agent: "claude-code".into(),
            session_id: "synthetic".into(),
            title: "Synthetic".into(),
            cwd: None,
            surface: "cli".into(),
            updated_at: Some(1),
        },
        files: vec![BundleFile {
            size: data.len() as u64,
            version: "fixture".into(),
            subagent_id: None,
            label: None,
            sidecar_for: None,
            fork_parent: false,
            companion: None,
        }],
        fork_parent_session_id: None,
    }
}

fn transfer(path: &Path, manifest: &BundleManifest, data: &[u8], same: bool) {
    let header = serde_json::to_vec(manifest).unwrap();
    let mut bytes = if same {
        b"ABR2SAME".to_vec()
    } else {
        b"ABR2DATA".to_vec()
    };
    bytes.extend((header.len() as u32).to_le_bytes());
    bytes.extend(header);
    bytes.extend(data);
    fs::write(path, bytes).unwrap();
}

#[test]
fn imports_private_transcripts_and_rejects_incomplete_or_mismatched_bundles() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("bundle");
    let data = b"synthetic transcript\n";
    let manifest = manifest(data);
    transfer(&bundle, &manifest, data, false);
    let target = temp.path().join("snapshot");
    private_dir(&target).unwrap();
    assert!(!unpack(&bundle, &target, &manifest.session).unwrap().1);
    let path = target.join("synthetic.jsonl");
    assert_eq!(fs::read(&path).unwrap(), data);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let mut wrong = manifest.session.clone();
    wrong.session_id = "another".into();
    assert!(unpack(&bundle, &temp.path().join("wrong"), &wrong).is_err());
    transfer(&bundle, &manifest, b"short", false);
    assert!(unpack(&bundle, &temp.path().join("partial"), &manifest.session).is_err());
    assert_eq!(fs::read(path).unwrap(), data);
    transfer(&bundle, &manifest, b"", true);
    assert!(unpack(&bundle, &target, &manifest.session).unwrap().1);
    transfer(&bundle, &manifest, b"extra", true);
    assert!(unpack(&bundle, &target, &manifest.session).is_err());
}

fn cached_record(path: &Path) -> SessionRecord {
    SessionRecord {
        key: SessionKey::new("ssh:host", "claude-code", "synthetic"),
        source_kind: "file".into(),
        source_label: path.to_string_lossy().into_owned(),
        wsl_distro: None,
        title: None,
        title_source: None,
        cwd: None,
        surface: "cli".into(),
        updated_at_epoch: Some(1),
        activity_cursor: "fixture".into(),
        activity_source: "mtime".into(),
        subagent_count: 0,
        fork_parent_session_id: None,
        source_fingerprint: None,
    }
}

/// Store a cache record with its recorded generation directory.
fn store_cached(store: &Store, record: &SessionRecord, generation: &Path, agents: &[&str]) {
    store
        .upsert_sessions(std::slice::from_ref(record), agents)
        .unwrap();
    store.set_remote_cache_dir(&record.key, generation).unwrap();
}

#[test]
fn committed_deletion_survives_cleanup_errors_and_rejects_unowned_paths() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(temp.path()).unwrap();
    let root = temp.path().join("remote");
    let parent = cache_parent(
        &root,
        &SessionKey::new("ssh:host", "claude-code", "synthetic"),
    )
    .unwrap();
    fs::create_dir_all(&parent).unwrap();
    let broken = parent.join("generation");
    fs::write(&broken, "not a directory").unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "keep").unwrap();
    for directory in [&broken, &outside] {
        let record = cached_record(&directory.join("synthetic.jsonl"));
        store_cached(&store, &record, directory, &["claude-code"]);
        assert!(remove_cached_generation(&root, &record.key, Some(directory.clone())).is_err());
        assert!(
            delete_session(&store, Some(&root), &record.key)
                .unwrap()
                .is_some()
        );
        assert!(store.session(&record.key).unwrap().is_none());
        assert!(directory.exists());
    }
    assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(), "keep");
    let good = parent.join("valid");
    fs::create_dir(&good).unwrap();
    fs::write(good.join("synthetic.jsonl"), "fixture").unwrap();
    let record = cached_record(&good.join("synthetic.jsonl"));
    store_cached(&store, &record, &good, &["claude-code"]);
    assert!(
        delete_session(&store, Some(&root), &record.key)
            .unwrap()
            .is_some()
    );
    assert!(!good.exists());
}

#[test]
fn pruning_failure_does_not_undo_committed_retention() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(temp.path()).unwrap();
    let record = cached_record(&temp.path().join("synthetic.jsonl"));
    store
        .upsert_sessions(std::slice::from_ref(&record), &["claude-code"])
        .unwrap();
    fs::write(temp.path().join("transcripts"), "not a directory").unwrap();
    store
        .update_settings(|settings| settings.session_data_retention_days = 30)
        .unwrap();
    let (removed, revision) = store.apply_session_retention(4_000_000_000).unwrap();
    assert_eq!(removed, 1);
    assert!(prune_unreferenced(&store, temp.path()).is_err());
    prune_after_commit(&store, temp.path());
    assert_eq!(revision, store.revision());
    assert!(store.session(&record.key).unwrap().is_none());
    let mut settings = store.settings().unwrap();
    settings.session_data_retention_days = 90;
    let (_, saved, ()) = store
        .replace_settings_preserving_interface_scale(&settings, |_, _, _| Ok(()))
        .unwrap();
    prune_after_commit(&store, temp.path());
    assert_eq!(saved.session_data_retention_days, 90);
    assert_eq!(store.settings().unwrap().session_data_retention_days, 90);
}

#[test]
fn staging_projection_accounts_for_the_extracted_manifest_copy() {
    assert!(staging_projection_fits(
        STAGING_LIMIT_BYTES - 30,
        10,
        10,
        10,
    ));
    assert!(!staging_projection_fits(
        STAGING_LIMIT_BYTES - 29,
        10,
        10,
        10,
    ));
}

#[tokio::test]
async fn imported_transcript_uses_full_analysis_and_retains_tools_and_billable_usage() {
    let temp = tempfile::tempdir().unwrap();
    let data = br#"{"type":"user","uuid":"u1","timestamp":"2026-09-15T10:00:00Z","message":{"role":"user","content":"Inspect the file"}}
{"type":"assistant","uuid":"a1","timestamp":"2026-09-15T10:00:01Z","message":{"id":"m1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/synthetic/example.rs"}}],"usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}
{"type":"user","uuid":"u2","timestamp":"2026-09-15T10:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"Example"}]}}
{"type":"assistant","uuid":"a2","timestamp":"2026-09-15T10:00:03Z","message":{"id":"m2","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Reviewed"}],"usage":{"input_tokens":120,"output_tokens":10,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}
"#;
    let manifest = manifest(data);
    let bundle = temp.path().join("bundle");
    transfer(&bundle, &manifest, data, false);
    let target = temp.path().join("snapshot");
    unpack(&bundle, &target, &manifest.session).unwrap();
    let path = target.join(manifest.file_name(0));
    let pass = analysis::analyze_located_for_evidence(
        crate::agents::kind_from_slug("claude-code").unwrap(),
        "synthetic",
        analysis::ClaimedSource {
            fingerprint: Some(parent_fingerprint(&path).unwrap()),
            generation: 0,
        },
        PassSignal::new(),
        Some(antiburn_local::analysis::MemoryTurnRowStore::new(
            "claude-code",
            "synthetic",
        )),
        None,
        analysis::LocatedTranscripts {
            source: SessionSource::File(path),
            children: vec![],
        },
    )
    .await;
    assert!(matches!(pass.outcome, analysis::PassOutcome::Published));
    assert!(pass.evidence.is_some());
    assert!(pass.analysis.summary.is_some());
    assert_eq!(
        pass.analysis
            .metrics
            .as_ref()
            .unwrap()
            .billable_input_tokens,
        220
    );
    assert_eq!(
        pass.analysis
            .metrics
            .as_ref()
            .unwrap()
            .billable_output_tokens,
        30
    );
    assert!(!pass.analysis.inclusive_model_breakdown.is_empty());
    let encoded = serde_json::to_string(&pass.analysis.summary).unwrap();
    assert!(
        encoded.contains("Read"),
        "tool analysis must survive import"
    );
}

#[tokio::test]
async fn inferred_forks_exclude_inherited_usage_from_the_same_host_only() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(temp.path()).unwrap();
    let inherited = r#"{"type":"assistant","uuid":"inherited","timestamp":"2026-09-15T10:00:00Z","message":{"id":"first","role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"text","text":"Inherited"}],"usage":{"input_tokens":10,"output_tokens":2}}}
"#;
    let own = r#"{"type":"assistant","uuid":"own","timestamp":"2026-09-15T10:01:00Z","message":{"id":"second","role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"text","text":"New work"}],"usage":{"input_tokens":5,"output_tokens":1}}}
"#;
    let mut records = Vec::new();
    for (host, id, data) in [
        ("one", "ancestor", inherited.to_owned()),
        ("one", "synthetic", format!("{inherited}{own}")),
        ("two", "synthetic", format!("{inherited}{own}")),
    ] {
        let dir = temp.path().join(host).join(id);
        private_dir(&dir).unwrap();
        let mut manifest = manifest(data.as_bytes());
        manifest.session.session_id = id.to_owned();
        let bundle = dir.join("bundle");
        transfer(&bundle, &manifest, data.as_bytes(), false);
        unpack(&bundle, &dir, &manifest.session).unwrap();
        let path = dir.join(manifest.file_name(0));
        let record = SessionRecord {
            key: SessionKey::for_origin("claude-code", id, None, Some(host)).unwrap(),
            source_kind: "file".into(),
            source_label: path.to_string_lossy().into_owned(),
            wsl_distro: None,
            title: None,
            title_source: None,
            cwd: None,
            surface: "cli".into(),
            updated_at_epoch: Some(1),
            activity_cursor: "fixture".into(),
            activity_source: "mtime".into(),
            subagent_count: 0,
            fork_parent_session_id: None,
            source_fingerprint: Some(parent_fingerprint(&path).unwrap()),
        };
        store_cached(&store, &record, &dir, &["claude-code"]);
        records.push(record);
    }
    store
        .record_fork_parent(&records[1].key, "ancestor")
        .unwrap();
    let mut damaged = records[1].clone();
    damaged.key = SessionKey::new("ssh:one", "claude-code", "damaged");
    damaged.source_label = temp
        .path()
        .join("missing/damaged.jsonl")
        .to_string_lossy()
        .into_owned();
    damaged.updated_at_epoch = Some(2);
    store
        .upsert_sessions(std::slice::from_ref(&damaged), &["claude-code"])
        .unwrap();
    store.record_fork_parent(&damaged.key, "ancestor").unwrap();
    assert_eq!(store.recent_sessions(0, 10).unwrap()[0].key, damaged.key);
    assert_eq!(restore_fork_companions(&store).unwrap(), 1);
    assert_eq!(restore_fork_companions(&store).unwrap(), 0);
    assert_eq!(
        fs::read_to_string(&records[1].source_label).unwrap(),
        format!("{inherited}{own}"),
        "fork repair must preserve the child transcript",
    );
    for (record, expected) in [(&records[1], 5), (&records[2], 15)] {
        let (dir, manifest) = manifest_for(&store, record).unwrap();
        prepare_fork_parent(&store, record, &dir, &manifest, Some("ancestor")).unwrap();
        let pass = analysis::analyze_located_for_evidence(
            crate::agents::kind_from_slug("claude-code").unwrap(),
            "synthetic",
            analysis::ClaimedSource {
                fingerprint: record.source_fingerprint.clone(),
                generation: 0,
            },
            PassSignal::new(),
            Some(antiburn_local::analysis::MemoryTurnRowStore::new(
                "claude-code",
                "synthetic",
            )),
            Some("ancestor".into()),
            analysis::LocatedTranscripts {
                source: SessionSource::File(PathBuf::from(&record.source_label)),
                children: vec![],
            },
        )
        .await;
        assert!(matches!(pass.outcome, analysis::PassOutcome::Published));
        assert_eq!(
            pass.analysis.metrics.unwrap().billable_input_tokens,
            expected
        );
    }
    fs::remove_dir_all(Path::new(&records[0].source_label).parent().unwrap()).unwrap();
    assert!(
        Path::new(&records[1].source_label)
            .parent()
            .unwrap()
            .join("ancestor.jsonl")
            .exists()
    );
}

#[test]
fn origin_identity_isolates_identical_ids_and_host_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(temp.path()).unwrap();
    let keys: Vec<_> = [None, Some("one"), Some("two")]
        .into_iter()
        .map(|host| SessionKey::for_origin("codex", "same", None, host).unwrap())
        .collect();
    let now = antiburn_remote::now();
    for key in &keys {
        let record = SessionRecord {
            key: key.clone(),
            source_kind: "file".into(),
            source_label: "/synthetic/session.jsonl".into(),
            wsl_distro: None,
            title: None,
            title_source: None,
            cwd: key.remote_host_id().map(|_| "/remote/project".to_owned()),
            surface: "cli".into(),
            updated_at_epoch: Some(now),
            activity_cursor: "1".into(),
            activity_source: "mtime".into(),
            subagent_count: 0,
            fork_parent_session_id: None,
            source_fingerprint: None,
        };
        store.upsert_sessions(&[record], &["codex"]).unwrap();
    }
    store
        .observe_provider_account("codex", "openai", &"a".repeat(64), now + 1, "provider_live")
        .unwrap();
    let bindings = store.session_bound_accounts(&keys).unwrap();
    assert!(
        bindings
            .keys()
            .any(|(key, _)| key.environment_key == "native")
    );
    assert!(
        bindings
            .keys()
            .all(|(key, _)| key.remote_host_id().is_none())
    );
    assert_eq!(
        store
            .usage_evidence(0, 0, crate::store::UsageOrigins::Local)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .usage_evidence(0, 0, crate::store::UsageOrigins::LocalAndRemote)
            .unwrap()
            .len(),
        3
    );
    store.delete_remote_host("one").unwrap();
    assert!(store.session(&keys[0]).unwrap().is_some());
    assert!(store.session(&keys[1]).unwrap().is_none());
    assert!(store.session(&keys[2]).unwrap().is_some());
}

fn store_manifest(
    agent: &str,
    session_id: &str,
    files: &[(&[u8], Option<BundleCompanion>)],
) -> BundleManifest {
    BundleManifest {
        session: RemoteSession {
            agent: agent.into(),
            session_id: session_id.into(),
            title: "Synthetic".into(),
            cwd: Some("/synthetic/repo".into()),
            surface: "cli".into(),
            updated_at: Some(1),
        },
        files: files
            .iter()
            .map(|(bytes, companion)| BundleFile {
                size: bytes.len() as u64,
                version: "synthetic".into(),
                subagent_id: None,
                label: None,
                sidecar_for: None,
                fork_parent: false,
                companion: *companion,
            })
            .collect(),
        fork_parent_session_id: None,
    }
}

/// Unpack a store bundle, then move it like a committed generation.
async fn import_store_bundle(
    temp: &Path,
    manifest: &BundleManifest,
    files: &[&[u8]],
) -> (PathBuf, String) {
    let bundle = temp.join("bundle");
    transfer(&bundle, manifest, &files.concat(), false);
    let staging = temp.join("staging");
    unpack(&bundle, &staging, &manifest.session).unwrap();
    let fingerprint = source_fingerprint(manifest, &staging.join(manifest.file_name(0)))
        .await
        .unwrap();
    let generation = temp.join("generation");
    fs::rename(&staging, &generation).unwrap();
    let primary = generation.join(manifest.file_name(0));
    // The commit renames the staging directory. The fingerprint must not
    // change, or analysis rejects the copy as a changed source.
    assert_eq!(
        source_fingerprint(manifest, &primary).await.unwrap(),
        fingerprint
    );
    (primary, fingerprint)
}

async fn analyze_store_copy(
    agent: &str,
    session_id: &str,
    primary: PathBuf,
    fingerprint: String,
) -> analysis::EvidencePass {
    let kind = crate::agents::kind_from_slug(agent).unwrap();
    analysis::analyze_located_for_evidence(
        kind,
        session_id,
        analysis::ClaimedSource {
            fingerprint: Some(fingerprint),
            generation: 0,
        },
        PassSignal::new(),
        Some(antiburn_local::analysis::MemoryTurnRowStore::new(
            agent, session_id,
        )),
        None,
        analysis::LocatedTranscripts {
            source: cached_source(kind, session_id, primary),
            children: vec![],
        },
    )
    .await
}

fn database_bytes(build: impl FnOnce(&rusqlite::Connection)) -> Vec<u8> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    build(&connection);
    drop(connection);
    fs::read(path).unwrap()
}

#[tokio::test]
async fn an_imported_opencode_extract_publishes_its_cluster_usage() {
    let temp = tempfile::tempdir().unwrap();
    let database = database_bytes(|connection| {
        connection
            .execute_batch(
                r#"CREATE TABLE session (
                     id TEXT PRIMARY KEY, parent_id TEXT, title TEXT, directory TEXT,
                     time_created INTEGER, time_updated INTEGER);
                 CREATE TABLE message (
                     id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER,
                     time_updated INTEGER, data TEXT);
                 CREATE TABLE part (
                     id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT,
                     time_created INTEGER, time_updated INTEGER, data TEXT);
                 INSERT INTO session VALUES
                     ('ses_root', NULL, 'Root', '/synthetic/repo', 100, 130),
                     ('ses_child', 'ses_root', 'Child', '/synthetic/repo', 110, 120);
                 INSERT INTO message VALUES
                     ('msg_1', 'ses_root', 110, 110,
                      '{"role":"assistant","modelID":"model-a","tokens":{"input":12,"output":3}}'),
                     ('msg_2', 'ses_child', 115, 115,
                      '{"role":"assistant","modelID":"model-a","tokens":{"input":5,"output":1}}');"#,
            )
            .unwrap();
    });
    let manifest = store_manifest("opencode", "ses_root", &[(&database, None)]);
    assert_eq!(manifest.file_name(0), "opencode.db");
    let (primary, fingerprint) = import_store_bundle(temp.path(), &manifest, &[&database]).await;
    assert!(fingerprint.starts_with("sv1:db:"));

    let pass = analyze_store_copy("opencode", "ses_root", primary, fingerprint).await;
    assert!(matches!(pass.outcome, analysis::PassOutcome::Published));
    let metrics = pass.analysis.metrics.unwrap();
    assert_eq!(metrics.billable_input_tokens, 17);
    assert_eq!(metrics.billable_output_tokens, 4);
}

#[tokio::test]
async fn an_imported_antigravity_database_publishes_with_its_brain_transcript() {
    fn varint(mut value: u64, out: &mut Vec<u8>) {
        while value >= 0x80 {
            out.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
    }
    fn scalar(field: u64, value: u64, out: &mut Vec<u8>) {
        varint(field << 3, out);
        varint(value, out);
    }
    fn bytes(field: u64, value: &[u8], out: &mut Vec<u8>) {
        varint((field << 3) | 2, out);
        varint(value.len() as u64, out);
        out.extend_from_slice(value);
    }
    let mut usage = Vec::new();
    scalar(1, 777, &mut usage);
    scalar(2, 30, &mut usage);
    scalar(3, 50, &mut usage);
    bytes(11, b"response-1", &mut usage);
    let mut chat_model = Vec::new();
    bytes(4, &usage, &mut chat_model);
    bytes(19, b"gemini-3.6-flash", &mut chat_model);
    let mut blob = Vec::new();
    bytes(1, &chat_model, &mut blob);

    let temp = tempfile::tempdir().unwrap();
    let database = database_bytes(|connection| {
        connection
            .execute_batch(
                "CREATE TABLE steps (idx INTEGER PRIMARY KEY, metadata BLOB);
                 CREATE TABLE gen_metadata (idx INTEGER PRIMARY KEY, data BLOB, size INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO steps(idx) VALUES (0), (1);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO gen_metadata(idx, data, size) VALUES (0, ?1, ?2)",
                rusqlite::params![blob, blob.len() as i64],
            )
            .unwrap();
    });
    let transcript = concat!(
        r#"{"type":"USER_INPUT","created_at":"2026-01-01T00:00:00Z","content":"hello"}"#,
        "\n",
        r#"{"type":"PLANNER_RESPONSE","created_at":"2026-01-01T00:00:01Z","content":"done"}"#,
        "\n"
    )
    .as_bytes();
    let manifest = store_manifest(
        "antigravity",
        "root",
        &[
            (&database, None),
            (transcript, Some(BundleCompanion::AntigravityBrain)),
        ],
    );
    let (primary, fingerprint) =
        import_store_bundle(temp.path(), &manifest, &[&database, transcript]).await;
    assert!(primary.ends_with("conversations/root.db"));
    assert!(
        primary
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("brain/root/.system_generated/logs/transcript.jsonl")
            .is_file()
    );

    let pass = analyze_store_copy("antigravity", "root", primary, fingerprint).await;
    assert!(matches!(pass.outcome, analysis::PassOutcome::Published));
    let metrics = pass.analysis.metrics.unwrap();
    assert_eq!(metrics.billable_input_tokens, 30);
    assert_eq!(metrics.billable_output_tokens, 50);
}

#[tokio::test]
async fn an_imported_cursor_transcript_publishes_without_usage() {
    let temp = tempfile::tempdir().unwrap();
    let transcript = concat!(
        r#"{"sessionId":"cursor-session","session_id":"cursor-session","cwd":"/synthetic/repo","cursor_source":"agent_transcript"}"#,
        "\n",
        r#"{"role":"user","message":{"content":[{"type":"text","text":"Fix the build"}]}}"#,
        "\n",
        r#"{"role":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}"#,
        "\n"
    )
    .as_bytes();
    let manifest = store_manifest("cursor", "cursor-session", &[(transcript, None)]);
    let (primary, fingerprint) = import_store_bundle(temp.path(), &manifest, &[transcript]).await;
    let pass = analyze_store_copy("cursor", "cursor-session", primary, fingerprint).await;
    assert!(matches!(pass.outcome, analysis::PassOutcome::Published));
    // Cursor CLI transcripts do not record token usage.
    assert_eq!(pass.analysis.metrics.unwrap().billable_input_tokens, 0);
}

#[tokio::test]
async fn an_intact_older_copy_of_the_same_listing_skips_the_export() {
    const DAY: i64 = 24 * 60 * 60;
    let now = 2_000_000_000;
    let temp = tempfile::tempdir().unwrap();
    let session_id = "session-260101-000000-old1";
    let journal = b"{}\n";
    let mut manifest = store_manifest("junie", session_id, &[(journal, None)]);
    manifest.session.updated_at = Some(now - 30 * DAY);
    let (primary, _) = import_store_bundle(temp.path(), &manifest, &[journal]).await;
    let store = Store::open_in_memory(temp.path()).unwrap();
    let mut record = cached_record(&primary);
    record.key = SessionKey::for_origin("junie", session_id, None, Some("host")).unwrap();
    store_cached(&store, &record, &temp.path().join("generation"), &["junie"]);
    assert!(settled_copy(&store, &record, &manifest.session, now));

    // A new modification time or a new title exports again.
    let mut changed = manifest.session.clone();
    changed.updated_at = Some(now - 29 * DAY);
    assert!(!settled_copy(&store, &record, &changed, now));
    let mut renamed = manifest.session.clone();
    renamed.title = "Renamed".into();
    assert!(!settled_copy(&store, &record, &renamed, now));
    // A session with activity in the default window exports again.
    assert!(!settled_copy(
        &store,
        &record,
        &manifest.session,
        now - 25 * DAY
    ));
    // An incomplete copy exports again.
    fs::write(&primary, b"").unwrap();
    assert!(!settled_copy(&store, &record, &manifest.session, now));
}

#[tokio::test]
async fn an_imported_junie_journal_publishes_its_usage() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "session-261015-120000-syn1";
    let journal = concat!(
        r#"{"kind":"TaskStartedEvent","taskId":"task-1","timestampMs":1790845200200}"#,
        "\n",
        r#"{"kind":"SessionA2uxEvent","taskId":"task-1","timestampMs":1790845201000,"event":{"state":"IN_PROGRESS","agentEvent":{"kind":"LlmResponseMetadataEvent","agent":{"kind":"MainAgent","id":"main","name":"main","type":"LINEAR"},"modelUsage":[{"model":"gpt-5.4-nano","cost":0.00002,"inputTokens":150,"cacheInputTokens":40,"cacheCreateTokens":0,"outputTokens":7,"time":0}]}}}"#,
        "\n"
    )
    .as_bytes();
    let manifest = store_manifest("junie", session_id, &[(journal, None)]);
    // The copy keeps the session folder, which gives the session ID.
    assert_eq!(manifest.file_name(0), format!("{session_id}/events.jsonl"));
    let (primary, fingerprint) = import_store_bundle(temp.path(), &manifest, &[journal]).await;
    let pass = analyze_store_copy("junie", session_id, primary, fingerprint).await;
    assert!(matches!(pass.outcome, analysis::PassOutcome::Published));
    let metrics = pass.analysis.metrics.unwrap();
    assert_eq!(metrics.billable_input_tokens, 150);
    assert_eq!(metrics.billable_output_tokens, 7);
}

#[test]
fn prune_and_delete_use_the_recorded_directory_of_a_nested_layout() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = Store::open_in_memory(root).unwrap();
    let key = SessionKey::for_origin("antigravity", "root", None, Some("host")).unwrap();
    let parent = cache_parent(root, &key).unwrap();
    let generation = parent.join("generation");
    // The first file sits three levels below the generation directory.
    // The old depth count would name `generation/nested` instead.
    let primary = generation.join("nested/conversations/root.db");
    fs::create_dir_all(primary.parent().unwrap()).unwrap();
    fs::write(&primary, b"synthetic").unwrap();
    let stale = parent.join("stale");
    fs::create_dir_all(&stale).unwrap();
    let mut record = cached_record(&primary);
    record.key = key;
    store_cached(&store, &record, &generation, &["antigravity"]);
    assert_eq!(
        generation_dir(&store, &record).as_deref(),
        Some(generation.as_path())
    );
    assert_eq!(prune_unreferenced(&store, root).unwrap(), 1);
    assert!(primary.is_file());
    assert!(!stale.exists());
    delete_session(&store, Some(root), &record.key)
        .unwrap()
        .unwrap();
    assert!(!generation.exists());
}

#[test]
fn a_failed_sync_keeps_the_cache_of_an_unrecorded_legacy_record() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = Store::open_in_memory(root).unwrap();
    let record = cached_record(&root.join("synthetic.jsonl"));
    let parent = cache_parent(root, &record.key).unwrap();
    let legacy = parent.join("generation");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("synthetic.jsonl"), "kept").unwrap();
    store
        .upsert_sessions(std::slice::from_ref(&record), &["claude-code"])
        .unwrap();
    // The export fails after this step.
    let previous = store.session(&record.key).unwrap();
    let retained = previous
        .as_ref()
        .and_then(|record| generation_dir(&store, record));
    clear_before_export(&parent, previous.is_some(), retained.as_deref()).unwrap();
    assert_eq!(
        fs::read_to_string(legacy.join("synthetic.jsonl")).unwrap(),
        "kept"
    );
    // A committed replacement removes the legacy generation.
    let target = parent.join("replacement");
    fs::create_dir_all(&target).unwrap();
    remove_generations_except(&parent, Some(&target)).unwrap();
    assert!(!legacy.exists());
    assert!(target.is_dir());
}

#[test]
fn pruning_keeps_every_directory_while_a_record_has_none() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = Store::open_in_memory(root).unwrap();
    let record = cached_record(&root.join("synthetic.jsonl"));
    let parent = cache_parent(root, &record.key).unwrap();
    let unrecorded = parent.join("generation");
    fs::create_dir_all(&unrecorded).unwrap();
    store
        .upsert_sessions(std::slice::from_ref(&record), &["claude-code"])
        .unwrap();
    assert_eq!(prune_unreferenced(&store, root).unwrap(), 0);
    assert!(unrecorded.is_dir());
}

#[test]
fn the_source_kind_follows_the_store_format() {
    for store in antiburn_remote::remote_store::REMOTE_STORES {
        let source = cached_source(store.agent, "id", PathBuf::from("primary"));
        assert_eq!(
            matches!(source, SessionSource::ProviderDb { .. }),
            store.format == StoreFormat::Db,
            "{}",
            store.agent.slug()
        );
    }
}
