use std::fs::OpenOptions;
use std::io::Write;

use antiburn_local::analysis::file_live::{NativeFilePage, read_native_file_page};
use antiburn_local::analysis::{RawSource, SessionInput};
use serde_json::{Value, json};

use super::*;
use crate::store::native_tool_results::ToolResultPublicationOutcome;

const CLAUDE_RESULTS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../crates/antiburn-local/tests/fixtures/claude_characterization/retained_native_results.jsonl"
));

fn fixture(source: &str) -> (tempfile::TempDir, Store, SessionInput) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    std::fs::write(&path, source).unwrap();
    let store = Store::open(directory.path()).unwrap();
    store
        .upsert_sessions(
            &[SessionRecord {
                key: SessionKey::new("native", "claude-code", "scope-test"),
                source_kind: "file".into(),
                source_label: path.to_string_lossy().into_owned(),
                cwd: Some("/synthetic".into()),
                wsl_distro: None,
                title: None,
                title_source: None,
                surface: "cli".into(),
                updated_at_epoch: Some(100),
                activity_cursor: String::new(),
                activity_source: "event".into(),
                subagent_count: 0,
                fork_parent_session_id: None,
                source_fingerprint: Some("synthetic".into()),
            }],
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    store.lock().execute(
        "INSERT INTO burn_check_activation (check_id, activation_scope_key, definition_revision,
             enabled, generation, bootstrap_sequence, applied_sequence, updated_at_epoch)
         VALUES ('file-upgrade', ?1, 'synthetic', 1, 1, 0, 0, 100)",
        [json!({"environment":"native", "session_cwd":"/synthetic", "run":"Live"}).to_string()],
    ).unwrap();
    let input = SessionInput {
        agent: "claude".into(),
        session_id: "scope-test".into(),
        source: RawSource::File(path),
        source_format: SourceFormat::ClaudeJsonl,
        fork_parent_session_id: None,
    };
    (directory, store, input)
}

fn append(input: &SessionInput, text: &str) {
    let RawSource::File(path) = &input.source else {
        unreachable!()
    };
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

fn next_page(store: &Store, input: &SessionInput, now: i64) -> (NativeFileClaim, NativeFilePage) {
    let claim = store.claim_next_native_file(now, 60).unwrap().unwrap();
    let cursor = claim.cursor().unwrap();
    let page = read_native_file_page(input, cursor.as_ref(), &|| false).unwrap();
    (claim, page)
}

fn prior_receipt(store: &Store, parser_revision: i64, now: i64) {
    store
        .lock()
        .execute(
            "UPDATE native_file_delivery SET parser_revision = ?1,
                 pending = 1, next_attempt_at_epoch = ?2",
            params![parser_revision, now],
        )
        .unwrap();
}

#[test]
fn compatible_parser_receipts_keep_partial_records_baselines_and_completed_results() {
    for parser_revision in [63, 64] {
        let lines: Vec<_> = CLAUDE_RESULTS.split_inclusive('\n').collect();
        let pending = lines[4];
        let split = pending.len() / 2;
        let prefix = format!("{}{}", lines[..4].concat(), &pending[..split]);
        let (directory, store, input) = fixture(&prefix);
        let (claim, first) = next_page(&store, &input, 100);
        assert_eq!(first.results.len(), 1);
        assert!(first.pending_tail);
        store
            .publish_native_file_page(&claim, &first, &[], 100)
            .unwrap();
        let current = store.native_tool_results_current(&claim.key).unwrap();
        let publications = store.native_tool_result_publications_after(0, 100).unwrap();
        let saved = serde_json::to_string(&first.cursor).unwrap();
        prior_receipt(&store, parser_revision, 101);
        drop(store);

        let store = Store::open(directory.path()).unwrap();
        let (claim, idle) = next_page(&store, &input, 101);
        assert!(!claim.reset);
        assert_eq!(claim.cursor_json.as_deref(), Some(saved.as_str()));
        assert_eq!(idle.bytes_read, 0);
        assert_eq!(idle.cursor.offset(), first.cursor.offset());
        assert_eq!(idle.baseline_offset, first.baseline_offset);
        assert_eq!(
            idle.cursor.evidence_baseline_offset(),
            first.cursor.evidence_baseline_offset()
        );
        assert!(idle.pending_tail);
        assert!(idle.results.is_empty() && idle.evidence.events.is_empty());
        store
            .publish_native_file_page(&claim, &idle, &[], 101)
            .unwrap();
        assert_eq!(
            store.native_tool_results_current(&claim.key).unwrap(),
            current
        );
        let after = store.native_tool_result_publications_after(0, 100).unwrap();
        assert!(after.starts_with(&publications));
        assert!(after[publications.len()..].iter().all(|publication| publication.outcome == ToolResultPublicationOutcome::CoverageGap));
        assert_eq!(
            store
                .lock()
                .query_row(
                    "SELECT parser_revision FROM native_file_delivery",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            PARSER_REVISION
        );

        append(&input, &pending[split..]);
        let (claim, completed) = next_page(&store, &input, 102);
        assert_eq!(completed.results.len(), 1);
        assert_eq!(
            completed.results[0].observation.call_id.as_deref(),
            Some("test-1")
        );
        assert!(completed.results[0].live_eligible);
        assert_eq!(completed.baseline_offset, first.baseline_offset);
        assert!(!completed.pending_tail);
        store
            .publish_native_file_page(&claim, &completed, &[], 102)
            .unwrap();
        assert_eq!(
            store.native_tool_results_current(&claim.key).unwrap().len(),
            2
        );
        let (_, idle) = next_page(&store, &input, 103);
        assert!(idle.results.is_empty() && idle.evidence.events.is_empty());
    }
}

#[test]
fn parser_64_receipt_keeps_pending_output_without_reading_or_replaying_completed_results() {
    let lines: Vec<_> = CLAUDE_RESULTS.lines().collect();
    let mut call: Value = serde_json::from_str(lines[1]).unwrap();
    call["message"]["content"] = json!(
        (0..300)
            .map(|index| json!({
                "type":"tool_use", "id":format!("call-{index}"), "name":"Read",
                "input":{"file_path":"/synthetic/file.rs"}
            }))
            .collect::<Vec<_>>()
    );
    let mut result: Value = serde_json::from_str(lines[2]).unwrap();
    result.as_object_mut().unwrap().remove("toolUseResult");
    result.as_object_mut().unwrap().remove("sourceToolUseID");
    result
        .as_object_mut()
        .unwrap()
        .remove("sourceToolAssistantUUID");
    result["message"]["content"] =
        json!((0..300).map(|index| json!({
        "type":"tool_result", "tool_use_id":format!("call-{index}"), "content":"source text"
    })).collect::<Vec<_>>());
    let source = format!("{}\n{call}\n{result}\n", lines[0]);
    let (directory, store, input) = fixture(&source);
    let mut now = 100;
    let first_pending = loop {
        let (claim, page) = next_page(&store, &input, now);
        store
            .publish_native_file_page(&claim, &page, &[], now)
            .unwrap();
        now += 1;
        if !page.results.is_empty() && page.has_more {
            break page;
        }
        assert!(page.has_more, "fixture must leave result output pending");
        assert!(
            now < 110,
            "fixture must reach result output within ten pages"
        );
    };
    let saved = serde_json::to_string(&first_pending.cursor).unwrap();
    let pending: Vec<Value> = serde_json::from_str(
        serde_json::from_str::<Value>(&saved).unwrap()["pending_evidence"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(
        !pending.is_empty(),
        "the stored receipt must contain pending output"
    );
    let key = SessionKey::new("native", "claude-code", "scope-test");
    let completed = store.native_tool_results_current(&key).unwrap();
    assert!(!completed.is_empty() && completed.len() < 300);
    prior_receipt(&store, 64, now);
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    let (claim, page) = next_page(&store, &input, now);
    assert!(!claim.reset);
    assert_eq!(claim.cursor_json.as_deref(), Some(saved.as_str()));
    assert_eq!(page.bytes_read, 0);
    assert_eq!(page.cursor.offset(), first_pending.cursor.offset());
    assert_eq!(page.baseline_offset, first_pending.baseline_offset);
    assert!(!page.results.is_empty());
    for result in &page.results {
        assert!(
            completed
                .iter()
                .all(|prior| prior.result.call_id != result.observation.call_id)
        );
    }
    store
        .publish_native_file_page(&claim, &page, &[], now)
        .unwrap();
    assert_eq!(
        store.native_tool_results_current(&key).unwrap().len(),
        completed.len() + page.results.len()
    );
}

#[test]
fn unknown_parser_receipt_keeps_the_saved_cursor_and_results_without_a_claim() {
    let (_directory, store, input) = fixture(CLAUDE_RESULTS);
    let (claim, first) = next_page(&store, &input, 100);
    store
        .publish_native_file_page(&claim, &first, &[], 100)
        .unwrap();
    let before = store.native_tool_result_publications_after(0, 100).unwrap();
    let saved = serde_json::to_string(&first.cursor).unwrap();
    prior_receipt(&store, 61, 101);
    assert!(store.claim_next_native_file(101, 60).unwrap().is_none());
    assert_eq!(
        store.native_tool_result_publications_after(0, 100).unwrap(),
        before
    );
    assert_eq!(
        store
            .lock()
            .query_row("SELECT cursor_json FROM native_file_delivery", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        saved
    );
}

fn v4_cursor(mut cursor: Value) -> Value {
    cursor["revision"] = json!(4);
    for field in [
        "evidence_baseline_offset",
        "pending_evidence",
        "admission_failure",
    ] {
        cursor.as_object_mut().unwrap().remove(field);
    }
    cursor["adapter"]["Claude"]
        .as_object_mut()
        .unwrap()
        .remove("order_unproven");
    cursor
}

#[test]
fn native_cursor_loader_requires_exact_parser_and_cursor_revision_pairs() {
    const CANARY: &str = "LEGACY_REQUEST_WITHOUT_ORIGIN";
    for (parser, revision, accepted) in [
        (62, 4, true),
        (62, 5, false),
        (63, 4, false),
        (63, 5, true),
        (64, 4, false),
        (64, 5, true),
        (65, 4, false),
        (65, 5, true),
        (64, 6, false),
        (65, 6, false),
    ] {
        let (_directory, store, input) = fixture(CLAUDE_RESULTS);
        let (claim, first) = next_page(&store, &input, 100);
        store
            .publish_native_file_page(&claim, &first, &[], 100)
            .unwrap();
        let current = store.native_tool_results_current(&claim.key).unwrap();
        let mut cursor = serde_json::to_value(&first.cursor).unwrap();
        cursor["requests"][0]["message"]["parts"][0]["text"] = json!(CANARY);
        if revision == 4 {
            cursor = v4_cursor(cursor);
        } else {
            cursor["revision"] = json!(revision);
        }
        let saved = cursor.to_string();
        prior_receipt(&store, parser, 101);
        store
            .lock()
            .execute("UPDATE native_file_delivery SET cursor_json = ?1", [&saved])
            .unwrap();
        let claim = store.claim_next_native_file(101, 60).unwrap().unwrap();
        assert!(!claim.reset);
        assert_eq!(claim.cursor_json.as_deref(), Some(saved.as_str()));
        let loaded = claim.cursor();
        assert_eq!(
            loaded.is_ok(),
            accepted,
            "parser {parser}, cursor {revision}"
        );
        if accepted {
            let loaded = loaded.unwrap().unwrap();
            assert_eq!(loaded.offset(), first.cursor.offset());
            assert_eq!(loaded.baseline_offset(), first.baseline_offset);
            assert_eq!(
                serde_json::to_string(&loaded).unwrap().contains(CANARY),
                parser == 65
            );
        } else {
            store
                .fail_native_file(&claim, NativeFileReadError::InvalidCursor, 101, 101)
                .unwrap();
        }
        assert_eq!(
            store.native_tool_results_current(&claim.key).unwrap(),
            current
        );
        assert_eq!(
            store
                .lock()
                .query_row("SELECT cursor_json FROM native_file_delivery", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            saved
        );
    }
}

#[test]
fn prior_parser_sanitizes_unpublished_claude_text_once_and_persists_the_result() {
    const CANARY: &str = "UNACCEPTED_HOST_TEXT_MUST_NOT_LEAK";
    const FRESH: &str = "New validated user text stays after restart.";
    for parser in [63, 64] {
        let lines: Vec<_> = CLAUDE_RESULTS.split_inclusive('\n').collect();
        let pending = lines[4];
        let split = pending.len() / 2;
        let prefix = format!("{}{}", lines[..4].concat(), &pending[..split]);
        let (directory, store, input) = fixture(&prefix);
        let (claim, first) = next_page(&store, &input, 100);
        store
            .publish_native_file_page(&claim, &first, &[], 100)
            .unwrap();
        let accepted = store.native_tool_results_current(&claim.key).unwrap();
        let mut cursor = serde_json::to_value(&first.cursor).unwrap();
        cursor["requests"][0]["message"]["parts"][0]["text"] = json!(CANARY);
        let mut user = first
            .evidence
            .events
            .iter()
            .find_map(|event| match &event.fact {
                antiburn_local::analysis::NativeEvidenceFact::Message(
                    antiburn_local::analysis::NativeMessageObservation::User(user),
                ) => Some(user.clone()),
                _ => None,
            })
            .unwrap();
        user.message.parts[0].text = CANARY.into();
        let pending_user = antiburn_local::analysis::NativeEvidenceEvent {
            fact: antiburn_local::analysis::NativeEvidenceFact::Message(
                antiburn_local::analysis::NativeMessageObservation::User(user),
            ),
            live_eligible: false,
        };
        cursor["pending_evidence"] = json!(serde_json::to_string(&vec![pending_user]).unwrap());
        prior_receipt(&store, parser, 101);
        store
            .lock()
            .execute(
                "UPDATE native_file_delivery SET cursor_json = ?1",
                [cursor.to_string()],
            )
            .unwrap();
        let (claim, migrated) = next_page(&store, &input, 101);
        assert!(migrated.results.is_empty() && migrated.evidence.events.is_empty());
        assert!(migrated.requests.is_empty());
        assert!(migrated.pending_tail && !migrated.history_complete);
        assert_eq!(migrated.bytes_read, 0);
        assert_eq!(migrated.cursor.offset(), first.cursor.offset());
        assert_eq!(migrated.baseline_offset, first.baseline_offset);
        store
            .publish_native_file_page(&claim, &migrated, &[], 101)
            .unwrap();
        assert_eq!(
            store.native_tool_results_current(&claim.key).unwrap(),
            accepted
        );
        drop(store);

        let store = Store::open(directory.path()).unwrap();
        let saved: String = store
            .lock()
            .query_row("SELECT cursor_json FROM native_file_delivery", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(!saved.contains(CANARY));
        append(&input, &pending[split..]);
        let (claim, completed) = next_page(&store, &input, 102);
        assert_eq!(completed.results.len(), 1);
        assert_eq!(
            completed.results[0].observation.call_id.as_deref(),
            Some("test-1")
        );
        assert!(
            completed.results[0]
                .observation
                .context
                .as_ref()
                .unwrap()
                .user_messages
                .is_empty()
        );
        store
            .publish_native_file_page(&claim, &completed, &[], 102)
            .unwrap();
        let current = store.native_tool_results_current(&claim.key).unwrap();
        assert!(current.contains(&accepted[0]));
        assert_eq!(current.len(), 2);

        let mut fresh: Value = serde_json::from_str(lines[0]).unwrap();
        fresh["uuid"] = json!("00000000-0000-0000-0000-00000000000b");
        fresh["parentUuid"] = serde_json::from_str::<Value>(pending).unwrap()["uuid"].clone();
        fresh["message"]["content"] = json!(FRESH);
        append(&input, &format!("{fresh}\n"));
        let (claim, fresh) = next_page(&store, &input, 103);
        assert_eq!(fresh.requests.len(), 1);
        assert_eq!(fresh.requests[0].message.parts[0].text, FRESH);
        store
            .publish_native_file_page(&claim, &fresh, &[], 103)
            .unwrap();
        drop(store);

        let store = Store::open(directory.path()).unwrap();
        let (_, idle) = next_page(&store, &input, 104);
        assert_eq!(idle.requests.len(), 1);
        assert_eq!(idle.requests[0].message.parts[0].text, FRESH);
        assert!(idle.results.is_empty() && idle.evidence.events.is_empty());
        assert!(
            !serde_json::to_string(&idle.cursor)
                .unwrap()
                .contains(CANARY)
        );
    }
}

fn origin_markers(store: &Store) -> Vec<i64> {
    store
        .lock()
        .prepare(
            "SELECT publication_sequence FROM native_tool_result_publication
        WHERE gap_reason = ?1 ORDER BY publication_sequence",
        )
        .unwrap()
        .query_map(
            [crate::store::native_tool_results::request_origin::RESET_REASON],
            |row| row.get(0),
        )
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn parser_65_empty_upgrade_commits_one_marker_with_the_sanitized_cursor() {
    let (directory, store, input) = fixture(CLAUDE_RESULTS);
    let offline = read_native_file_page(&input, None, &|| false).unwrap();
    let offline = offline.cursor.migrate_pre_65_claude_requests().unwrap();
    let claim = store.claim_next_native_file(100, 60).unwrap().unwrap();
    store.lock().execute("UPDATE native_file_delivery SET cursor_json = ?1, observed_generation = ?2,
        pending = 0, claim_fence = NULL, lease_expires_at_epoch = NULL, next_attempt_at_epoch = 101",
        params![serde_json::to_string(&offline).unwrap(), claim.generation]).unwrap();
    assert!(origin_markers(&store).is_empty());
    let (claim, page) = next_page(&store, &input, 101);
    assert!(!claim.notify_on_success);
    assert_eq!(page.bytes_read, 0);
    assert!(page.evidence.events.is_empty() && page.results.is_empty());
    assert!(page.requests.is_empty() && !page.history_complete);
    assert_eq!(
        claim.cursor_json.as_deref(),
        Some(serde_json::to_string(&page.cursor).unwrap().as_str())
    );
    assert!(
        store
            .publish_native_file_page(&claim, &page, &[], 101)
            .unwrap()
    );
    let markers = origin_markers(&store);
    assert_eq!(markers.len(), 1);
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    let (claim, page) = next_page(&store, &input, 102);
    assert_eq!(page.bytes_read, 0);
    assert!(page.requests.is_empty());
    assert!(page.results.is_empty() && page.evidence.events.is_empty());
    store
        .publish_native_file_page(&claim, &page, &[], 102)
        .unwrap();
    assert_eq!(origin_markers(&store), markers);
    let connection = store.lock();
    assert!(connection.query_row("SELECT parser_revision = 65 AND json_extract(cursor_json, '$.revision') = 5
        AND json_array_length(json_extract(cursor_json, '$.requests')) = 0 FROM native_file_delivery", [], |row| row.get::<_, bool>(0)).unwrap());
}

#[test]
fn parser_65_receipt_without_a_marker_sanitizes_requests_and_pending_contexts_before_read() {
    use antiburn_local::analysis::{
        NativeEvidenceEvent, NativeEvidenceFact, NativeMessageObservation,
    };

    const CANARY: &str = "UNPROVEN_PARSER_65_REQUEST_MUST_NOT_LEAK";
    let (directory, store, input) = fixture(CLAUDE_RESULTS);
    let first = read_native_file_page(&input, None, &|| false).unwrap();
    let mut request = first.requests[0].clone();
    request.message.parts[0].text = CANARY.into();
    let mut result = first.results[0].observation.clone();
    result.record_id = Some("pending-legacy-result".into());
    result.call_id = Some("pending-legacy-call".into());
    result.context.as_mut().unwrap().user_messages = vec![request.message.clone()];
    let mut cursor = serde_json::to_value(&first.cursor).unwrap();
    cursor["requests"] = json!([request]);
    cursor["pending_evidence"] = json!(
        serde_json::to_string(&vec![NativeEvidenceEvent {
            fact: NativeEvidenceFact::Message(NativeMessageObservation::ToolResult(result)),
            live_eligible: true,
        }])
        .unwrap()
    );
    let claim = store.claim_next_native_file(100, 60).unwrap().unwrap();
    {
        let mut connection = store.lock();
        let transaction = connection.transaction().unwrap();
        assert!(
            crate::store::native_tool_results::evidence::reconcile_evidence_in(
                &transaction,
                &claim.key,
                claim.generation,
                claim.fence,
                &first.evidence,
                PublicationPolicy {
                    suppress_coverage: true,
                    ..Default::default()
                },
                100_000,
            )
            .unwrap()
        );
        transaction.execute("UPDATE native_file_delivery SET cursor_json = ?1, observed_generation = ?2,
            pending = 0, claim_fence = NULL, lease_expires_at_epoch = NULL, next_attempt_at_epoch = 101",
            params![cursor.to_string(), claim.generation]).unwrap();
        transaction.commit().unwrap();
    }
    let current = store.native_tool_results_current(&claim.key).unwrap();
    let publications = store.native_tool_result_publications_after(0, 100).unwrap();
    assert!(origin_markers(&store).is_empty());
    let claim = store.claim_next_native_file(101, 60).unwrap().unwrap();
    assert_eq!(claim.parser_revision, Some(65));
    assert!(!claim.request_origin_boundary);
    assert!(claim.cursor_json.as_ref().unwrap().contains(CANARY));
    let loaded = claim.cursor().unwrap().unwrap();
    assert!(!serde_json::to_string(&loaded).unwrap().contains(CANARY));
    assert_eq!(loaded.offset(), first.cursor.offset());
    assert_eq!(loaded.baseline_offset(), first.baseline_offset);
    let page = read_native_file_page(&input, Some(&loaded), &|| false).unwrap();
    assert_eq!(page.bytes_read, 0);
    assert_eq!(page.results.len(), 1);
    assert!(page.requests.is_empty());
    assert!(!format!("{page:?}").contains(CANARY));
    assert!(
        page.results[0]
            .observation
            .context
            .as_ref()
            .unwrap()
            .user_messages
            .is_empty()
    );
    store
        .publish_native_file_page(&claim, &page, &[], 101)
        .unwrap();
    let markers = origin_markers(&store);
    assert_eq!(markers.len(), 1);
    assert!(
        store
            .native_tool_result_publications_after(0, 100)
            .unwrap()
            .starts_with(&publications)
    );
    assert!(current.iter().all(|result| {
        store
            .native_tool_results_current(&claim.key)
            .unwrap()
            .contains(result)
    }));
    let after = store
        .native_evidence_publications_after(markers[0], 100)
        .unwrap();
    assert!(!after.is_empty());
    assert!(!format!("{after:?}").contains(CANARY));
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    let (claim, idle) = next_page(&store, &input, 102);
    assert!(claim.request_origin_boundary);
    assert!(idle.results.is_empty() && idle.evidence.events.is_empty());
    assert!(!format!("{idle:?}").contains(CANARY));
    store
        .publish_native_file_page(&claim, &idle, &[], 102)
        .unwrap();
    assert_eq!(origin_markers(&store), markers);
    let lines: Vec<_> = CLAUDE_RESULTS.lines().collect();
    let mut fresh: Value = serde_json::from_str(lines[0]).unwrap();
    fresh["uuid"] = json!("validated-request-after-parser-65-boundary");
    fresh["parentUuid"] =
        serde_json::from_str::<Value>(lines.last().unwrap()).unwrap()["uuid"].clone();
    fresh["message"]["content"] = json!("Validated request after the boundary.");
    append(&input, &format!("{fresh}\n"));
    let (claim, page) = next_page(&store, &input, 103);
    assert_eq!(page.requests.len(), 1);
    assert_eq!(
        page.requests[0].message.parts[0].text,
        "Validated request after the boundary."
    );
    store
        .publish_native_file_page(&claim, &page, &[], 103)
        .unwrap();
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    let (_, page) = next_page(&store, &input, 104);
    assert_eq!(page.requests.len(), 1);
    assert_eq!(
        page.requests[0].message.parts[0].text,
        "Validated request after the boundary."
    );
    assert!(!format!("{page:?}").contains(CANARY));
    assert_eq!(origin_markers(&store), markers);
}

#[test]
fn parser_65_boundary_precedes_every_observation_in_its_first_page() {
    let (_directory, store, input) = fixture(CLAUDE_RESULTS);
    let (claim, page) = next_page(&store, &input, 100);
    assert!(claim.cursor_json.is_none());
    assert!(!page.requests.is_empty());
    let requests = serde_json::to_string(&page.requests).unwrap();
    store
        .publish_native_file_page(&claim, &page, &[], 100)
        .unwrap();
    let marker = origin_markers(&store)[0];
    let connection = store.lock();
    assert!(
        connection
            .query_row(
                "SELECT COUNT(*) > 0 AND MIN(publication_sequence) > ?1
        FROM native_tool_result_publication WHERE outcome = 'update'",
                [marker],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    assert!(
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_tool_result_publication p
        JOIN session s USING(environment_key, agent, session_id)
        WHERE p.publication_sequence = ?1 AND p.source_format = '\"claude_jsonl\"'
          AND p.session_incarnation = s.incarnation AND p.project_cwd IS s.cwd
          AND p.outcome = 'coverage_gap' AND p.gap_kind = 'segment_reset')",
                [marker],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    drop(connection);
    let (claim, idle) = next_page(&store, &input, 101);
    assert!(claim.request_origin_boundary);
    assert_eq!(serde_json::to_string(&idle.requests).unwrap(), requests);
}

#[test]
fn parser_65_partial_first_page_retains_marker_and_makes_progress_on_the_same_page() {
    let lines: Vec<_> = CLAUDE_RESULTS.lines().collect();
    let mut call: Value = serde_json::from_str(lines[1]).unwrap();
    call["message"]["content"] = json!(
        (0..300)
            .map(|index| json!({
                "type":"tool_use", "id":format!("call-{index}"), "name":"Read",
                "input":{"file_path":"/synthetic/file.rs"}
            }))
            .collect::<Vec<_>>()
    );
    let mut result: Value = serde_json::from_str(lines[2]).unwrap();
    for field in [
        "toolUseResult",
        "sourceToolUseID",
        "sourceToolAssistantUUID",
    ] {
        result.as_object_mut().unwrap().remove(field);
    }
    result["message"]["content"] =
        json!((0..300).map(|index| json!({
        "type":"tool_result", "tool_use_id":format!("call-{index}"), "content":"source text"
    })).collect::<Vec<_>>());
    let (directory, store, input) = fixture(&format!("{}\n{call}\n{result}\n", lines[0]));
    expand_publication_cost(&store);
    let (claim, page) = next_page(&store, &input, 100);
    assert_eq!(page.evidence.events.len(), 256);
    assert!(
        store
            .publish_native_file_page(&claim, &page, &[], 100)
            .unwrap()
    );
    let markers = origin_markers(&store);
    assert_eq!(markers.len(), 1);
    assert_eq!(native_update_count(&store), 128);
    assert!(
        store
            .lock()
            .query_row(
                "SELECT cursor_json IS NULL AND pending = 1
        AND claim_fence IS NULL AND lease_expires_at_epoch IS NULL FROM native_file_delivery",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    let key = claim.key.clone();
    drop(page);
    drop(claim);
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    finish_reloaded_pages(&store, &input, 101);
    assert_eq!(origin_markers(&store), markers);
    assert_eq!(store.native_tool_results_current(&key).unwrap().len(), 300);
    assert_eq!(native_update_count(&store), 601);
    assert_unique_native_updates(&store);
    assert!(
        store
            .lock()
            .query_row(
                "SELECT cursor_json IS NOT NULL AND pending = 0 FROM native_file_delivery",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
}

fn expand_publication_cost(store: &Store) {
    // Make a real reader page exceed the store budget without changing its events.
    store.lock().execute_batch("CREATE TRIGGER expand_test_publication AFTER INSERT ON native_tool_result_publication
        WHEN NEW.outcome = 'update' BEGIN
          INSERT INTO native_tool_result_publication(outcome, environment_key, agent, session_id,
            source_format, complete, source_generation, claim_fence, project_cwd, session_incarnation, gap_reason, gap_kind)
          VALUES ('coverage_gap', NEW.environment_key, NEW.agent, NEW.session_id, NEW.source_format,
            0, NEW.source_generation, NEW.claim_fence, NEW.project_cwd, NEW.session_incarnation,
            'test-page-budget', 'partial');
        END;").unwrap();
}

fn native_update_count(store: &Store) -> i64 {
    store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM native_tool_result_publication WHERE outcome = 'update'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn assert_unique_native_updates(store: &Store) {
    assert!(
        store
            .lock()
            .query_row(
                "SELECT COUNT(*) = COUNT(DISTINCT native_identity)
        FROM native_tool_result_publication WHERE outcome = 'update'",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
}

fn finish_reloaded_pages(store: &Store, input: &SessionInput, now: i64) {
    for now in now..now + 20 {
        let before = native_update_count(store);
        let (claim, page) = next_page(store, input, now);
        assert!(claim.request_origin_boundary);
        assert!(
            store
                .publish_native_file_page(&claim, &page, &[], now)
                .unwrap()
        );
        assert_unique_native_updates(store);
        if store
            .lock()
            .query_row("SELECT pending = 0 FROM native_file_delivery", [], |row| {
                row.get::<_, bool>(0)
            })
            .unwrap()
        {
            let (claim, idle) = next_page(store, input, now + 1);
            assert!(idle.evidence.events.is_empty());
            store
                .publish_native_file_page(&claim, &idle, &[], now + 1)
                .unwrap();
            return;
        }
        assert!(
            native_update_count(store) > before,
            "the reloaded page must make progress"
        );
    }
    panic!("reloaded pages must finish within the test bound");
}

fn publication_state(store: &Store) -> Vec<String> {
    let connection = store.lock();
    let mut state = Vec::new();
    for table in [
        "native_file_delivery",
        "native_tool_result_current",
        "native_tool_result_publication",
    ] {
        let columns = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .into_iter()
            .map(|column| format!("\"{column}\""))
            .collect::<Vec<_>>()
            .join(",");
        state.push(
            connection
                .query_row(
                    &format!("SELECT json_group_array(json_array({columns})) FROM {table}"),
                    [],
                    |row| row.get(0),
                )
                .unwrap(),
        );
    }
    state.push(
        connection
            .query_row(
                "SELECT json_group_array(json_array(key, value)) FROM setting
        WHERE key LIKE 'internal:claudeRequestOriginParser65:%'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
    );
    state
}

#[test]
fn parser_65_losing_publisher_cannot_change_a_reclaimed_or_rebound_source() {
    for rebind in [false, true] {
        let (directory, store, mut input) = fixture(CLAUDE_RESULTS);
        let (losing_claim, losing_page) = next_page(&store, &input, 100);
        if rebind {
            let RawSource::File(path) = &input.source else {
                unreachable!()
            };
            let moved = directory.path().join("winning.jsonl");
            std::fs::rename(path, &moved).unwrap();
            store
                .lock()
                .execute(
                    "UPDATE session SET source_label = ?1",
                    [moved.to_string_lossy().as_ref()],
                )
                .unwrap();
            input.source = RawSource::File(moved);
            let rebound = publication_state(&store);
            assert!(
                !store
                    .publish_native_file_page(&losing_claim, &losing_page, &[], 101)
                    .unwrap()
            );
            assert_eq!(publication_state(&store), rebound);
        }
        let lines: Vec<_> = CLAUDE_RESULTS.lines().collect();
        let mut fresh: Value = serde_json::from_str(lines[0]).unwrap();
        fresh["uuid"] = json!("winning-publisher-request");
        fresh["parentUuid"] =
            serde_json::from_str::<Value>(lines.last().unwrap()).unwrap()["uuid"].clone();
        fresh["message"]["content"] = json!("The winning publisher keeps this request.");
        append(&input, &format!("{fresh}\n"));
        let (winning_claim, winning_page) = next_page(&store, &input, 161);
        assert!(winning_claim.fence > losing_claim.fence);
        assert!(origin_markers(&store).is_empty());
        let before = publication_state(&store);
        assert!(
            !store
                .publish_native_file_page(&losing_claim, &losing_page, &[], 161)
                .unwrap()
        );
        assert_eq!(publication_state(&store), before);
        assert!(
            store
                .publish_native_file_page(&winning_claim, &winning_page, &[], 161)
                .unwrap()
        );
        assert_eq!(origin_markers(&store).len(), 1);
        let saved: String = store
            .lock()
            .query_row("SELECT cursor_json FROM native_file_delivery", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(saved.contains("The winning publisher keeps this request."));
        assert_ne!(winning_page.cursor.offset(), losing_page.cursor.offset());
        let won = publication_state(&store);
        assert!(
            !store
                .publish_native_file_page(&losing_claim, &losing_page, &[], 162)
                .unwrap()
        );
        assert_eq!(publication_state(&store), won);
    }
}

#[test]
fn parser_65_marker_binding_outputs_and_receipt_roll_back_together_on_a_write_failure() {
    let (_directory, store, input) = fixture(CLAUDE_RESULTS);
    let (claim, page) = next_page(&store, &input, 100);
    store.lock().execute_batch("CREATE TEMP TRIGGER reject_origin_output BEFORE INSERT ON native_tool_result_publication
        WHEN NEW.outcome = 'update' BEGIN SELECT RAISE(ABORT, 'synthetic output failure'); END;").unwrap();
    assert!(
        store
            .publish_native_file_page(&claim, &page, &[], 100)
            .is_err()
    );
    assert!(origin_markers(&store).is_empty());
    assert!(
        store
            .native_tool_results_current(&claim.key)
            .unwrap()
            .is_empty()
    );
    assert!(!store.lock().query_row("SELECT EXISTS(SELECT 1 FROM setting WHERE key LIKE 'internal:claudeRequestOriginParser65:%')", [], |row| row.get::<_, bool>(0)).unwrap());
    assert!(
        store
            .lock()
            .query_row(
                "SELECT cursor_json IS NULL AND claim_fence IS NOT NULL FROM native_file_delivery",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    store
        .lock()
        .execute_batch("DROP TRIGGER reject_origin_output")
        .unwrap();
    assert!(
        store
            .publish_native_file_page(&claim, &page, &[], 100)
            .unwrap()
    );
    assert_eq!(origin_markers(&store).len(), 1);
}

#[test]
fn parser_65_partial_upgrade_persists_a_sanitized_retry_cursor_and_finishes_after_restart() {
    use antiburn_local::analysis::{
        NativeEvidenceEvent, NativeEvidenceFact, NativeMessageObservation,
    };
    const CANARY: &str = "PARTIAL_UPGRADE_REQUEST_MUST_NOT_RETURN";
    let (directory, store, input) = fixture(CLAUDE_RESULTS);
    let first = read_native_file_page(&input, None, &|| false).unwrap();
    let mut cursor = serde_json::to_value(&first.cursor).unwrap();
    let mut request = first.requests[0].message.clone();
    request.parts[0].text = CANARY.into();
    cursor["requests"][0]["message"] = json!(request);
    let pending: Vec<_> = (0..257)
        .map(|index| {
            let mut result = first.results[0].observation.clone();
            result.record_id = Some(format!("partial-record-{index}"));
            result.call_id = Some(format!("partial-call-{index}"));
            result.context.as_mut().unwrap().user_messages = vec![request.clone()];
            NativeEvidenceEvent {
                live_eligible: false,
                fact: NativeEvidenceFact::Message(NativeMessageObservation::ToolResult(result)),
            }
        })
        .collect();
    cursor["pending_evidence"] = json!(serde_json::to_string(&pending).unwrap());
    let claim = store.claim_next_native_file(100, 60).unwrap().unwrap();
    store
        .lock()
        .execute(
            "UPDATE native_file_delivery SET cursor_json = ?1,
        parser_revision = 65, observed_generation = ?2, claim_fence = NULL,
        lease_expires_at_epoch = NULL, next_attempt_at_epoch = 101",
            params![cursor.to_string(), claim.generation],
        )
        .unwrap();
    expand_publication_cost(&store);
    let (claim, page) = next_page(&store, &input, 101);
    assert!(!format!("{page:?}").contains(CANARY));
    assert_eq!(page.results.len(), 256);
    store
        .publish_native_file_page(&claim, &page, &[], 101)
        .unwrap();
    let saved: String = store
        .lock()
        .query_row("SELECT cursor_json FROM native_file_delivery", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(!saved.contains(CANARY));
    assert_eq!(
        serde_json::from_str::<NativeFileCursor>(&saved)
            .unwrap()
            .offset(),
        first.cursor.offset()
    );
    assert_eq!(
        store.native_tool_results_current(&claim.key).unwrap().len(),
        128
    );
    let markers = origin_markers(&store);
    assert_eq!(markers.len(), 1);
    let key = claim.key.clone();
    drop(page);
    drop(claim);
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    finish_reloaded_pages(&store, &input, 102);
    assert_eq!(store.native_tool_results_current(&key).unwrap().len(), 257);
    assert_eq!(native_update_count(&store), 257);
    assert!(
        !store
            .lock()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_tool_result_publication
        WHERE instr(COALESCE(payload_json, ''), ?1) > 0)",
                [CANARY],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    assert!(!format!("{:?}", store.native_tool_results_current(&key).unwrap()).contains(CANARY));
    assert!(
        !store
            .lock()
            .query_row("SELECT cursor_json FROM native_file_delivery", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
            .contains(CANARY)
    );
    assert_unique_native_updates(&store);
    assert_eq!(origin_markers(&store), markers);
    assert!(
        store
            .lock()
            .query_row(
                "SELECT pending = 0 AND claim_fence IS NULL FROM native_file_delivery",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
}

#[test]
fn parser_65_changed_source_label_requires_a_new_marker_and_sanitizes_the_retained_cursor() {
    let (directory, store, mut input) = fixture(CLAUDE_RESULTS);
    let (claim, first) = next_page(&store, &input, 100);
    store
        .publish_native_file_page(&claim, &first, &[], 100)
        .unwrap();
    assert!(!first.requests.is_empty());
    let first_marker = origin_markers(&store)[0];
    let RawSource::File(path) = &input.source else {
        unreachable!()
    };
    let moved = directory.path().join("moved.jsonl");
    std::fs::rename(path, &moved).unwrap();
    store
        .lock()
        .execute(
            "UPDATE session SET source_label = ?1",
            [moved.to_string_lossy().as_ref()],
        )
        .unwrap();
    input.source = RawSource::File(moved);
    let (claim, page) = next_page(&store, &input, 101);
    assert!(!claim.reset && !claim.request_origin_boundary);
    assert!(claim.cursor_json.as_ref().unwrap().contains("requests"));
    assert!(page.requests.is_empty());
    assert_eq!(page.bytes_read, 0);
    assert_eq!(page.cursor.offset(), first.cursor.offset());
    store
        .publish_native_file_page(&claim, &page, &[], 101)
        .unwrap();
    let markers = origin_markers(&store);
    assert_eq!(markers.len(), 2);
    assert!(markers[1] > first_marker);
    let (claim, page) = next_page(&store, &input, 102);
    assert!(claim.request_origin_boundary);
    assert!(page.requests.is_empty());
}
