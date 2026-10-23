//! Store-level pinning tests for atomic evidence publication.
//!
//! `Store::publish_projections` guards its write with the evidence row's own
//! claim fence and the session's current source generation (see `store/mod.rs`).
//! A losing pass must publish nothing it computed, and a winning pass must
//! remove every turn row an earlier, superseded pass left behind. This module
//! pins both halves directly against turn rows and a zero-baseline session,
//! a case the existing suite in `store/tests.rs` does not cover on its own.

use std::path::Path;

use antiburn_local::analysis::{
    ContentKind, ContentPart, TurnRow, TurnScope, count_turn_rows, insert_turn_rows,
};

use super::model::{PublishedEvidence, SESSION_DATA_RETENTION_DAYS_30};
use super::*;

fn store() -> Store {
    Store::open_in_memory(Path::new("/tmp/antiburn-publish-test-state")).expect("in-memory store")
}

fn session(session_id: &str, updated_at: i64) -> SessionRecord {
    SessionRecord {
        key: SessionKey::new("native", "claude-code", session_id),
        source_kind: "file".into(),
        source_label: format!("/home/avery/.claude/projects/demo/{session_id}.jsonl"),
        wsl_distro: None,
        title: Some("Wire the popover".into()),
        title_source: Some("explicit".into()),
        cwd: Some("/home/avery/code/widgets".into()),
        surface: "cli".into(),
        updated_at_epoch: Some(updated_at),
        activity_cursor: String::new(),
        activity_source: "mtime".into(),
        subagent_count: 0,
        fork_parent_session_id: None,
        source_fingerprint: None,
    }
}

fn projection_record(key: SessionKey, fingerprint: &str, generation: i64) -> AnalysisRecord {
    let revisions = crate::analysis::projection_revisions();
    AnalysisRecord {
        key,
        model_breakdown_json: "{}".into(),
        pricing_breakdown_json: "{}".into(),
        inclusive_models_json: "[]".into(),
        initial_context_json: None,
        source_summaries_json: None,
        provider_hints_json: None,
        source_fingerprint: fingerprint.into(),
        pricing_generation: 1,
        analyzed_generation: generation,
        parser_revision: revisions.parser_revision,
        analyzer_revision: revisions.analyzer_revision,
        metrics_schema_revision: revisions.metrics_schema_revision,
    }
}

fn claimed_projection(
    store: &Store,
    session_id: &str,
    now_epoch: i64,
    lease_secs: i64,
) -> (AnalysisRecord, EvidenceClaim) {
    let mut record = session(session_id, 1_000);
    let fingerprint = format!("sv1:{session_id}");
    record.source_fingerprint = Some(fingerprint.clone());
    store
        .upsert_sessions(
            std::slice::from_ref(&record),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let claim = store
        .claim_next_evidence(&["claude-code"], now_epoch, lease_secs)
        .unwrap()
        .unwrap();
    (
        projection_record(record.key, &fingerprint, claim.source_generation),
        claim,
    )
}

fn evidence_completion(
    claim: &EvidenceClaim,
    status: PublishedEvidence,
    evidence_json: String,
) -> EvidenceCompletion {
    EvidenceCompletion {
        claim_fence: claim.claim_fence,
        status,
        evidence_schema_revision: EVIDENCE_SCHEMA_REVISION,
        evidence_json,
    }
}

fn turn_row(turn_index: u64) -> TurnRow {
    TurnRow {
        source_key: "s1".into(),
        thread_id: "s1".into(),
        turn_index,
        scope: TurnScope::Main,
        child_id: None,
        role: "assistant",
        ts_ms: Some(1_000 + turn_index as i64),
        model: Some("claude-opus-4-6".into()),
        provider: None,
        api: None,
        effort: None,
        speed: None,
        input_tokens: 10,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cache_write_1h_tokens: 0,
        output_tokens: 5,
        is_compaction_boundary: false,
        message_id: None,
        uuid: None,
        parent_uuid: None,
        compaction_trigger: None,
        compaction_pre_tokens: None,
        compaction_post_tokens: None,
        has_thinking: false,
        last_tool: None,
        subagent_launches: 0,
        content: Vec::new(),
        recorded_cost_micros: None,
    }
}

fn turn_row_for(source_key: &str, turn_index: u64) -> TurnRow {
    TurnRow {
        source_key: source_key.to_owned(),
        thread_id: source_key.to_owned(),
        ..turn_row(turn_index)
    }
}

fn resume_snapshot(source_fingerprint: &str) -> StoredResume {
    let revisions = crate::analysis::resume_revisions();
    StoredResume {
        snapshot: vec![1, 2, 3],
        snapshot_revision: revisions.snapshot_revision,
        parser_revision: revisions.parser_revision,
        analyzer_revision: revisions.analyzer_revision,
        metrics_schema_revision: revisions.metrics_schema_revision,
        evidence_schema_revision: revisions.evidence_schema_revision,
        coverage_schema_revision: revisions.coverage_schema_revision,
        source_fingerprint: source_fingerprint.to_owned(),
    }
}

fn user_turn_with_content(turn_index: u64, content: &str) -> TurnRow {
    TurnRow {
        role: "user",
        content: vec![ContentPart::new(ContentKind::UserText, content)],
        ..turn_row(turn_index)
    }
}

#[test]
fn instruction_epoch_survives_appends_and_resets_for_source_replacement() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "instruction-epoch-publication", 100, 60);
    let key = record.key.clone();
    FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence)
        .write_turn_rows(&[turn_row(1), turn_row_for("second", 0)])
        .unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    let initial_sources = ["s1", "second"]
        .into_iter()
        .map(|source_key| SourcePublishOutcome {
            source_key: source_key.into(),
            mode: SourcePublishMode::Full,
            resume: Some(resume_snapshot("fp-initial")),
        })
        .collect::<Vec<_>>();
    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &initial_sources)
            .unwrap()
    );
    let (initial_positions, initial_at) = store
        .observe_burn_check_instruction_epoch(&key, 1, claim.source_generation, "rules-v1", 10_000)
        .unwrap();
    assert_eq!(initial_positions.get("s1"), Some(&1));
    assert_eq!(initial_positions.get("second"), Some(&0));

    let mut appended_session = session("instruction-epoch-publication", 1_000);
    appended_session.source_fingerprint = Some("sv1:instruction-epoch-append".into());
    store
        .upsert_sessions(
            std::slice::from_ref(&appended_session),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let append_claim = store
        .claim_next_evidence(&["claude-code"], 200, 60)
        .unwrap()
        .expect("append is pending");
    let append_record = projection_record(
        key.clone(),
        appended_session.source_fingerprint.as_deref().unwrap(),
        append_claim.source_generation,
    );
    FencedTurnRowStore::new(store.clone(), key.clone(), append_claim.claim_fence)
        .write_turn_rows(&[turn_row(2), turn_row_for("new-source", 0)])
        .unwrap();
    let append_completion = evidence_completion(
        &append_claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&append_claim.key),
    );
    let append_sources = [
        SourcePublishOutcome {
            source_key: "s1".into(),
            mode: SourcePublishMode::Resumed,
            resume: Some(resume_snapshot("fp-append")),
        },
        SourcePublishOutcome {
            source_key: "second".into(),
            mode: SourcePublishMode::Resumed,
            resume: Some(resume_snapshot("fp-append")),
        },
        SourcePublishOutcome {
            source_key: "new-source".into(),
            mode: SourcePublishMode::Full,
            resume: Some(resume_snapshot("fp-new")),
        },
    ];
    assert!(
        store
            .publish_projections(
                &append_record,
                None,
                &append_completion,
                &[],
                &append_sources,
            )
            .unwrap()
    );
    let (appended_positions, appended_at) = store
        .observe_burn_check_instruction_epoch(
            &key,
            1,
            append_claim.source_generation,
            "rules-v1",
            20_000,
        )
        .unwrap();
    assert_eq!(appended_at, initial_at);
    assert_eq!(appended_positions.get("s1"), Some(&1));
    assert_eq!(appended_positions.get("second"), Some(&0));
    assert_eq!(appended_positions.get("new-source"), Some(&0));

    let mut replaced_session = appended_session;
    replaced_session.source_fingerprint = Some("sv1:instruction-epoch-rewrite".into());
    store
        .upsert_sessions(
            std::slice::from_ref(&replaced_session),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let replacement_claim = store
        .claim_next_evidence(&["claude-code"], 300, 60)
        .unwrap()
        .expect("replacement is pending");
    let replacement_record = projection_record(
        key.clone(),
        replaced_session.source_fingerprint.as_deref().unwrap(),
        replacement_claim.source_generation,
    );
    FencedTurnRowStore::new(store.clone(), key.clone(), replacement_claim.claim_fence)
        .write_turn_rows(&[turn_row(3)])
        .unwrap();
    let replacement_completion = evidence_completion(
        &replacement_claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&replacement_claim.key),
    );
    let replacement_sources = [
        SourcePublishOutcome {
            source_key: "s1".into(),
            mode: SourcePublishMode::Full,
            resume: Some(resume_snapshot("fp-rewrite")),
        },
        SourcePublishOutcome {
            source_key: "second".into(),
            mode: SourcePublishMode::Resumed,
            resume: Some(resume_snapshot("fp-append")),
        },
        SourcePublishOutcome {
            source_key: "new-source".into(),
            mode: SourcePublishMode::Resumed,
            resume: Some(resume_snapshot("fp-new")),
        },
    ];
    assert!(
        store
            .publish_projections(
                &replacement_record,
                None,
                &replacement_completion,
                &[],
                &replacement_sources,
            )
            .unwrap()
    );
    let (replacement_positions, replacement_at) = store
        .observe_burn_check_instruction_epoch(
            &key,
            1,
            replacement_claim.source_generation,
            "rules-v1",
            30_000,
        )
        .unwrap();
    assert!(replacement_at > initial_at);
    assert_eq!(replacement_positions.get("s1"), Some(&3));

    let mut removed_session = replaced_session;
    removed_session.source_fingerprint = Some("sv1:instruction-epoch-source-removed".into());
    store
        .upsert_sessions(
            std::slice::from_ref(&removed_session),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let removal_claim = store
        .claim_next_evidence(&["claude-code"], 400, 60)
        .unwrap()
        .expect("source removal is pending");
    let removal_record = projection_record(
        key.clone(),
        removed_session.source_fingerprint.as_deref().unwrap(),
        removal_claim.source_generation,
    );
    let removal_completion = evidence_completion(
        &removal_claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&removal_claim.key),
    );
    let remaining_sources = [SourcePublishOutcome {
        source_key: "s1".into(),
        mode: SourcePublishMode::Resumed,
        resume: Some(resume_snapshot("fp-rewrite")),
    }];
    assert!(
        store
            .publish_projections(
                &removal_record,
                None,
                &removal_completion,
                &[],
                &remaining_sources,
            )
            .unwrap()
    );
    let (remaining_positions, removed_at) = store
        .observe_burn_check_instruction_epoch(
            &key,
            1,
            removal_claim.source_generation,
            "rules-v1",
            40_000,
        )
        .unwrap();
    assert!(removed_at > replacement_at);
    assert_eq!(remaining_positions.len(), 1);
    assert_eq!(remaining_positions.get("s1"), Some(&3));
}

fn insert_waiting_prompt(store: &Store, remediation_id: &str, target_key: &str) {
    store
        .lock()
        .execute(
            "INSERT INTO remediation (
                remediation_id, target_key, environment_key, agent, scope_kind, scope_key,
                state, definition_json, result_json, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, 'native', 'claude-code', 'project', 'scope',
                'waitingForPromptUse', '{\"version\":1}', '{\"version\":1}', 1, 1)",
            params![remediation_id, target_key],
        )
        .unwrap();
}

fn insert_waiting_prompt_group(
    store: &Store,
    remediation_id: &str,
    target_key: &str,
    prompt_group_id: &str,
) {
    store
        .lock()
        .execute(
            "INSERT INTO remediation (
                remediation_id, target_key, environment_key, agent, scope_kind, scope_key,
                state, definition_json, result_json, created_at_epoch, updated_at_epoch,
                origin, prompt_group_id)
             VALUES (?1, ?2, 'native', 'claude-code', 'project', 'scope',
                'waitingForPromptUse', '{\"version\":1}', '{\"version\":1}', 1, 1,
                'action', ?3)",
            params![remediation_id, target_key, prompt_group_id],
        )
        .unwrap();
}

fn insert_waiting_prompt_group_with_snapshot(
    store: &Store,
    remediation_id: &str,
    target_key: &str,
    prompt_group_id: &str,
) {
    insert_waiting_prompt_group(store, remediation_id, target_key, prompt_group_id);
    store
        .lock()
        .execute(
            "UPDATE remediation SET origin = 'action', display_snapshot_json = ?2
              WHERE remediation_id = ?1",
            params![remediation_id, r#"{"version":1,"findingId":"finding"}"#],
        )
        .unwrap();
}

/// Advances the session's source generation past the given claim, the same
/// way a concurrent, faster pass would before this pass tries to publish.
fn advance_source_generation_past(store: &Store, key: &SessionKey) {
    store
        .lock()
        .execute(
            "UPDATE session SET source_generation = source_generation + 1
              WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3",
            params![key.environment_key, key.agent, key.session_id],
        )
        .unwrap();
}

/* I1(a): a lost race publishes nothing, from a zero baseline. */

#[test]
fn a_lost_race_from_a_zero_baseline_publishes_no_evidence_no_analysis_and_no_turn_rows() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "lost-race-zero-baseline", 100, 60);
    let key = record.key.clone();
    // Nothing has ever published for this session: no session_analysis row
    // exists yet, and session_evidence still sits at its claimed, unpublished
    // state.
    assert!(store.analysis(&key).unwrap().is_none());

    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    writer.write_turn_rows(&[turn_row(0)]).unwrap();

    // A faster, concurrent pass wins the race first, which advances the
    // session's source generation past this pass's own claim. The baseline
    // is captured after this, so the comparison below isolates exactly what
    // `publish_projections` itself is and is not allowed to change.
    advance_source_generation_past(&store, &key);
    let evidence_before = store.evidence(&key).unwrap().unwrap();
    assert_eq!(evidence_before.evidence_json, None);
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );

    let published = store
        .publish_projections(&record, None, &completion, &[], &[])
        .unwrap();
    assert!(
        !published,
        "a lost race must report that it did not publish"
    );

    // No session_analysis row appeared, the evidence row is exactly what it
    // was before this pass ran, and this pass's own turn rows are gone.
    assert!(store.analysis(&key).unwrap().is_none());
    assert_eq!(store.evidence(&key).unwrap().unwrap(), evidence_before);
    let connection = store.lock();
    assert_eq!(
        count_turn_rows(&connection, &turn_session_key(&key), claim.claim_fence).unwrap(),
        0
    );
}

#[test]
fn a_lost_race_leaves_relations_untouched() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "lost-race-relations", 100, 60);
    let key = record.key.clone();
    store
        .replace_relations(
            &key,
            RelationKind::Subagent,
            &[RelationRecord {
                kind: RelationKind::Subagent,
                related_id: "kept-child".into(),
                label: None,
            }],
        )
        .unwrap();
    let relations_before = store.relations(&key).unwrap();

    advance_source_generation_past(&store, &key);
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    let losing_relations = [RelationRecord {
        kind: RelationKind::Subagent,
        related_id: "should-not-land".into(),
        label: None,
    }];

    assert!(
        !store
            .publish_projections(&record, None, &completion, &losing_relations, &[])
            .unwrap()
    );
    assert_eq!(store.relations(&key).unwrap(), relations_before);
}

/* I1(b): a won race removes every turn row from every superseded fence. */

#[test]
fn a_won_race_removes_turn_rows_from_every_superseded_fence_and_keeps_only_its_own() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "won-race-many-fences", 100, 60);
    let key = record.key.clone();

    // Two fences older than the winning one, left over from two earlier,
    // superseded passes.
    {
        let connection = store.lock();
        insert_turn_rows(
            &connection,
            &turn_session_key(&key),
            claim.claim_fence - 2,
            &[turn_row(0)],
        )
        .unwrap();
        insert_turn_rows(
            &connection,
            &turn_session_key(&key),
            claim.claim_fence - 1,
            &[turn_row(0)],
        )
        .unwrap();
    }
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    writer
        .write_turn_rows(&[turn_row(0), turn_row(1), turn_row(2)])
        .unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );

    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );

    let connection = store.lock();
    assert_eq!(
        count_turn_rows(&connection, &turn_session_key(&key), claim.claim_fence - 2).unwrap(),
        0,
        "the oldest superseded fence must be gone"
    );
    assert_eq!(
        count_turn_rows(&connection, &turn_session_key(&key), claim.claim_fence - 1).unwrap(),
        0,
        "the nearer superseded fence must be gone"
    );
    assert_eq!(
        count_turn_rows(&connection, &turn_session_key(&key), claim.claim_fence).unwrap(),
        3,
        "the winning fence's own rows must survive untouched"
    );
}

/* R1: `Store::published_turn_rows` only ever exposes `published_fence`'s own
 * rows — never a superseded fence, and never a claim in flight's partial
 * rows, even while that claim moves `status` and `claim_fence` out from
 * under it. */

fn revisions() -> ProjectionRevisions {
    ProjectionRevisions {
        parser_revision: 1,
        analyzer_revision: 1,
        metrics_schema_revision: 1,
        evidence_schema_revision: 1,
    }
}

#[test]
fn published_turn_rows_returns_the_winning_pass_rows_in_order() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "published-rows-order", 100, 60);
    let key = record.key.clone();
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    let written = [turn_row(2), turn_row(0), turn_row(1)];
    writer.write_turn_rows(&written).unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );

    let rows = store.published_turn_rows(&key).unwrap().expect("ready");
    assert_eq!(
        rows,
        vec![turn_row(0), turn_row(1), turn_row(2)],
        "rows must come back sorted by (source_key, turn_index), not insertion order"
    );
}

/// The whole point of `published_fence`: an actively-written session is
/// reclaimed and requeued constantly, so its evidence `status` is almost
/// never terminal at drilldown-open. Before `published_fence`,
/// `published_turn_rows` gated on `status`, so this in-flight claim made it
/// return `None` — the "Analyzing this session…" screen for minutes, even
/// though the last publish's rows were still sitting right there. Now it
/// keeps serving them: only the *next* winning publish moves
/// `published_fence` (and only then does it delete this fence's rows).
#[test]
fn published_turn_rows_serves_the_last_published_fence_while_a_newer_claim_is_in_flight() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "claim-in-flight", 100, 60);
    let key = record.key.clone();
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    writer.write_turn_rows(&[turn_row(0), turn_row(1)]).unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );
    let published = store.published_turn_rows(&key).unwrap();
    assert!(published.is_some());

    // A newer pass claims the session: status flips to `processing` and the
    // fence bumps, while `published_fence` still names the earlier, winning
    // fence — only a publish moves it. This pass then writes some, but not
    // all, of its own rows before this check runs.
    mark_evidence_pending_in(&store.lock(), &key).unwrap();
    let next_claim = store
        .claim_next_evidence(&["claude-code"], 200, 60)
        .unwrap()
        .expect("reclaimable");
    assert_eq!(next_claim.claim_fence, claim.claim_fence + 1);
    let next_writer = FencedTurnRowStore::new(store.clone(), key.clone(), next_claim.claim_fence);
    next_writer.write_turn_rows(&[turn_row(0)]).unwrap();

    assert_eq!(
        store.published_turn_rows(&key).unwrap(),
        published,
        "an in-flight claim's partial rows must never leak into a published_fence read"
    );
}

#[test]
fn published_turn_content_requires_a_fresh_winning_fence() {
    let store = store();
    let (mut record, claim) = claimed_projection(&store, "published-content", 100, 60);
    record.parser_revision = PARSER_REVISION;
    record.analyzer_revision = ANALYZER_REVISION;
    let key = record.key.clone();
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    let mut published_row = turn_row(0);
    published_row.content = vec![ContentPart::new(ContentKind::UserText, "published text")];
    writer.write_turn_rows(&[published_row]).unwrap();
    let mut completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    completion.evidence_schema_revision = EVIDENCE_SCHEMA_REVISION;
    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );

    let published = store
        .published_turn_content(&key)
        .unwrap()
        .expect("published content");
    assert_eq!(published.parts.len(), 1);
    assert_eq!(published.parts[0].part.text, "published text");

    advance_source_generation_past(&store, &key);
    assert_eq!(
        store.published_turn_content(&key).unwrap(),
        None,
        "a publication from an older source generation is stale"
    );

    mark_evidence_pending_in(&store.lock(), &key).unwrap();
    let next_claim = store
        .claim_next_evidence(&["claude-code"], 200, 60)
        .unwrap()
        .expect("reclaimable");
    let next_writer = FencedTurnRowStore::new(store.clone(), key.clone(), next_claim.claim_fence);
    let mut in_flight_row = turn_row(0);
    in_flight_row.content = vec![ContentPart::new(ContentKind::UserText, "in flight text")];
    next_writer.write_turn_rows(&[in_flight_row]).unwrap();

    assert_eq!(
        store.published_turn_content(&key).unwrap(),
        None,
        "a previous publication must not be used while a newer claim is in flight"
    );
}

#[test]
fn published_turn_rows_is_none_with_no_evidence_row() {
    let store = store();
    let key = SessionKey::new("native", "claude-code", "never-claimed");
    assert_eq!(store.published_turn_rows(&key).unwrap(), None);
}

#[test]
fn published_turn_rows_is_none_while_pending() {
    let store = store();
    let mut record = session("pending-status", 1_000);
    record.source_fingerprint = Some("sv1:pending-status".into());
    store
        .upsert_sessions(
            std::slice::from_ref(&record),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    assert_eq!(store.published_turn_rows(&record.key).unwrap(), None);
}

#[test]
fn published_turn_rows_is_none_when_failed() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "failed-status", 100, 60);
    let key = record.key.clone();
    assert!(
        store
            .fail_evidence(
                &claim,
                EvidenceFailure::Failed {
                    revisions: revisions(),
                },
                "source-unreadable",
            )
            .unwrap()
    );
    assert_eq!(store.published_turn_rows(&key).unwrap(), None);
}

/// For a terminal row (`ready` or `unsupported`), `published_fence` always
/// equals `claim_fence`: a winning publish stamps both from the same
/// completion in the same statement. So reading rows by `published_fence`
/// alone serves exactly the rows the old status-gated read served for these
/// two statuses — including `unsupported`, which is an insights verdict (no
/// detector was eligible for this source), not a parse-quality one: the
/// rows and the `session_analysis` record a winning pass wrote are still
/// complete for it, same as `ready`.
#[test]
fn published_fence_equals_claim_fence_for_every_terminal_status() {
    let store = store();
    for status in [PublishedEvidence::Ready, PublishedEvidence::Unsupported] {
        let session_id = format!("terminal-equivalence-{}", status.as_str());
        let (record, claim) = claimed_projection(&store, &session_id, 100, 60);
        let key = record.key.clone();
        let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
        writer.write_turn_rows(&[turn_row(0), turn_row(1)]).unwrap();
        let completion = evidence_completion(
            &claim,
            status,
            crate::store::test_support::evidence_json(&claim.key),
        );
        assert!(
            store
                .publish_projections(&record, None, &completion, &[], &[])
                .unwrap(),
            "a {} completion still publishes",
            status.as_str()
        );

        let evidence = store.evidence(&key).unwrap().unwrap();
        assert_eq!(
            evidence.published_fence,
            Some(evidence.claim_fence),
            "a winning publish stamps published_fence and claim_fence together"
        );
        assert_eq!(
            store.published_turn_rows(&key).unwrap(),
            Some(vec![turn_row(0), turn_row(1)]),
            "status `{}` names a complete row projection, same as `ready`",
            status.as_str()
        );
    }
}

#[test]
fn a_second_publish_supersedes_the_first_in_published_turn_rows() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "second-publish-supersedes", 100, 60);
    let key = record.key.clone();
    let first_writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    first_writer.write_turn_rows(&[turn_row(0)]).unwrap();
    let first_completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    assert!(
        store
            .publish_projections(&record, None, &first_completion, &[], &[])
            .unwrap()
    );
    assert_eq!(
        store.published_turn_rows(&key).unwrap().expect("ready"),
        vec![turn_row(0)]
    );

    mark_evidence_pending_in(&store.lock(), &key).unwrap();
    let next_claim = store
        .claim_next_evidence(&["claude-code"], 200, 60)
        .unwrap()
        .expect("reclaimable");
    let next_record = projection_record(
        key.clone(),
        "sv1:second-publish-supersedes",
        next_claim.source_generation,
    );
    let next_writer = FencedTurnRowStore::new(store.clone(), key.clone(), next_claim.claim_fence);
    next_writer
        .write_turn_rows(&[turn_row(0), turn_row(1)])
        .unwrap();
    let next_completion = evidence_completion(
        &next_claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&next_claim.key),
    );
    assert!(
        store
            .publish_projections(&next_record, None, &next_completion, &[], &[])
            .unwrap()
    );

    assert_eq!(
        store.published_turn_rows(&key).unwrap().expect("ready"),
        vec![turn_row(0), turn_row(1)],
        "only the new pass's rows must come back, none of the superseded fence's"
    );
}

#[test]
fn a_winning_ready_publication_activates_only_exact_markers_from_new_user_content() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "prompt-marker", 100, 60);
    let key = record.key.clone();
    insert_waiting_prompt(&store, "exact", "exact-target");
    insert_waiting_prompt(&store, "extended", "extended-target");
    insert_waiting_prompt(&store, "assistant", "assistant-target");
    insert_waiting_prompt(&store, "thinking", "thinking-target");
    insert_waiting_prompt(&store, "tool-input", "tool-input-target");
    let writer = FencedTurnRowStore::new(store.clone(), key, claim.claim_fence);
    writer
        .write_turn_rows(&[
            user_turn_with_content(0, "Remediation reference: ABR-exact"),
            user_turn_with_content(1, "Remediation reference: ABR-extended-more"),
            TurnRow {
                content: vec![ContentPart::new(
                    ContentKind::AssistantText,
                    "Remediation reference: ABR-assistant",
                )],
                ..turn_row(2)
            },
            TurnRow {
                role: "user",
                content: vec![ContentPart::new(
                    ContentKind::Thinking,
                    "Remediation reference: ABR-thinking",
                )],
                ..turn_row(3)
            },
            TurnRow {
                role: "user",
                content: vec![ContentPart::new(
                    ContentKind::ToolInput,
                    "Remediation reference: ABR-tool-input",
                )],
                ..turn_row(4)
            },
        ])
        .unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );

    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );
    let exact = store.remediation("exact").unwrap().unwrap();
    assert_eq!(exact.state, RemediationState::Watching);
    assert!(exact.effective_boundary_ms.is_some());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&exact.definition_json).unwrap()["promptAction"],
        true
    );
    assert_eq!(
        store.remediation("extended").unwrap().unwrap().state,
        RemediationState::WaitingForPromptUse
    );
    assert_eq!(
        store.remediation("assistant").unwrap().unwrap().state,
        RemediationState::WaitingForPromptUse
    );
    for remediation_id in ["thinking", "tool-input"] {
        assert_eq!(
            store.remediation(remediation_id).unwrap().unwrap().state,
            RemediationState::WaitingForPromptUse
        );
    }
}

#[test]
fn a_prompt_group_marker_activates_every_selected_action_without_using_other_content() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "prompt-group-marker", 100, 60);
    let key = record.key.clone();
    insert_waiting_prompt_group(&store, "group-one", "target-one", "shared-group");
    insert_waiting_prompt_group(&store, "group-two", "target-two", "shared-group");
    insert_waiting_prompt_group(
        &store,
        "group-extended",
        "target-three",
        "shared-group-extra",
    );
    let writer = FencedTurnRowStore::new(store.clone(), key, claim.claim_fence);
    writer
        .write_turn_rows(&[
            user_turn_with_content(0, "Remediation reference: ABR-shared-group"),
            TurnRow {
                content: vec![ContentPart::new(
                    ContentKind::AssistantText,
                    "Remediation reference: ABR-shared-group-extra",
                )],
                ..turn_row(1)
            },
        ])
        .unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );

    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );
    for remediation_id in ["group-one", "group-two"] {
        assert_eq!(
            store.remediation(remediation_id).unwrap().unwrap().state,
            RemediationState::Watching
        );
    }
    assert_eq!(
        store.remediation("group-extended").unwrap().unwrap().state,
        RemediationState::WaitingForPromptUse
    );
}

#[test]
fn prompt_activation_sets_the_action_boundary_and_dirties_only_the_activated_cycle() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "prompt-activation-boundary", 100, 60);
    let key = record.key.clone();
    insert_waiting_prompt_group_with_snapshot(&store, "activated", "target", "shared-group");
    insert_waiting_prompt_group(&store, "untouched", "other-target", "other-group");
    let writer = FencedTurnRowStore::new(store.clone(), key, claim.claim_fence);
    writer
        .write_turn_rows(&[user_turn_with_content(
            0,
            "Remediation reference: ABR-shared-group",
        )])
        .unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );

    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );

    let activated = store.remediation("activated").unwrap().unwrap();
    assert_eq!(activated.state, RemediationState::Watching);
    assert!(activated.effective_boundary_ms.is_some());
    assert!(activated.dirty_revision > activated.evaluated_revision);
    assert_eq!(
        store.remediation("untouched").unwrap().unwrap().state,
        RemediationState::WaitingForPromptUse
    );
}

/* R6: `published_fence` itself — a winning publish stamps it, a lost race
 * and every other evidence transition leave it exactly where it was. */

#[test]
fn publish_projections_stamps_published_fence_with_the_completion_claim_fence() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "publish-stamps-fence", 100, 60);
    let key = record.key.clone();
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    writer.write_turn_rows(&[turn_row(0)]).unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );

    assert_eq!(
        store.evidence(&key).unwrap().unwrap().published_fence,
        Some(claim.claim_fence)
    );
}

#[test]
fn a_lost_race_does_not_stamp_published_fence() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "lost-race-no-fence-stamp", 100, 60);
    let key = record.key.clone();
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    writer.write_turn_rows(&[turn_row(0)]).unwrap();
    advance_source_generation_past(&store, &key);
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );

    assert!(
        !store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );

    assert_eq!(store.evidence(&key).unwrap().unwrap().published_fence, None);
}

#[test]
fn a_requeue_and_a_reclaim_leave_published_fence_intact() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "requeue-claim-preserve-fence", 100, 60);
    let key = record.key.clone();
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    writer.write_turn_rows(&[turn_row(0)]).unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );
    let published_fence = store.evidence(&key).unwrap().unwrap().published_fence;
    assert_eq!(published_fence, Some(claim.claim_fence));

    // A fingerprint change requeues the session: status flips back to
    // `pending`. Only the next winning publish may move `published_fence`.
    mark_evidence_pending_in(&store.lock(), &key).unwrap();
    assert_eq!(
        store.evidence(&key).unwrap().unwrap().published_fence,
        published_fence
    );

    // The worker reclaims it: status flips to `processing` and `claim_fence`
    // bumps, while `published_fence` still names the earlier, winning fence.
    let next_claim = store
        .claim_next_evidence(&["claude-code"], 200, 60)
        .unwrap()
        .expect("reclaimable");
    assert_eq!(next_claim.claim_fence, claim.claim_fence + 1);
    assert_eq!(
        store.evidence(&key).unwrap().unwrap().published_fence,
        published_fence
    );
}

#[test]
fn fail_evidence_leaves_published_fence_intact() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "fail-preserve-fence", 100, 60);
    let key = record.key.clone();
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    writer.write_turn_rows(&[turn_row(0)]).unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );
    let published_fence = store.evidence(&key).unwrap().unwrap().published_fence;

    mark_evidence_pending_in(&store.lock(), &key).unwrap();
    let next_claim = store
        .claim_next_evidence(&["claude-code"], 200, 60)
        .unwrap()
        .expect("reclaimable");
    assert!(
        store
            .fail_evidence(
                &next_claim,
                EvidenceFailure::Retry {
                    next_attempt_at_epoch: 300,
                    counts_as_attempt: true,
                },
                "transient",
            )
            .unwrap()
    );

    assert_eq!(
        store.evidence(&key).unwrap().unwrap().published_fence,
        published_fence
    );
}

/// The published-analysis carve-out this change exists for: a session
/// requeued after a publish (an actively-written transcript, fingerprinted
/// again) still serves its last published rows, not `None`, while the fresh
/// pass is only queued and has not run yet.
#[test]
fn published_turn_rows_serves_the_last_published_fence_after_a_requeue() {
    let store = store();
    let (record, claim) = claimed_projection(&store, "requeue-after-publish", 100, 60);
    let key = record.key.clone();
    let writer = FencedTurnRowStore::new(store.clone(), key.clone(), claim.claim_fence);
    writer.write_turn_rows(&[turn_row(0), turn_row(1)]).unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    assert!(
        store
            .publish_projections(&record, None, &completion, &[], &[])
            .unwrap()
    );

    mark_evidence_pending_in(&store.lock(), &key).unwrap();
    assert_eq!(
        store.evidence(&key).unwrap().unwrap().status,
        EvidenceStatus::Pending
    );

    assert_eq!(
        store.published_turn_rows(&key).unwrap(),
        Some(vec![turn_row(0), turn_row(1)]),
        "a requeue must not hide the last published pass's rows"
    );
}

#[test]
#[ignore = "Runs the large synthetic session reproduction."]
fn large_resumed_publication_reports_contention() {
    use std::time::Instant;
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    let record = session("synthetic-large-session", 1_000);
    store
        .upsert_sessions(
            std::slice::from_ref(&record),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let start = Instant::now();
    {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        tx.execute_batch(
            "WITH RECURSIVE sequence(i) AS (
                 SELECT 0 UNION ALL SELECT i + 1 FROM sequence WHERE i < 310906
             ), rows AS (
                 SELECT i,
                        CASE WHEN i < 310841 THEN 1 ELSE 2 END AS fence,
                        CASE WHEN i < 36787 OR (i >= 310841 AND i < 310883)
                             THEN 'main' ELSE 'delegated' END AS scope,
                        CASE WHEN i < 36787 OR (i >= 310841 AND i < 310883)
                             THEN 'main'
                             WHEN i >= 310883 THEN 'child-0'
                             ELSE 'child-' || (i % 855) END AS source
                   FROM sequence
             )
             INSERT INTO turn (
                 environment_key, agent, session_id, claim_fence, source_key,
                 thread_id, turn_index, scope, child_id, role, ts_ms, model,
                 input_tokens, cache_read_tokens, cache_write_tokens,
                 output_tokens, is_compaction_boundary, uuid
             )
             SELECT 'native', 'claude-code', 'synthetic-large-session', fence,
                    source, source, i, scope,
                    CASE WHEN scope = 'main' THEN NULL ELSE source END,
                    CASE WHEN i % 2 = 0 THEN 'assistant' ELSE 'user' END,
                    1700000000000 + i * 5000, 'claude-opus-4-6',
                    1000, 2000, 100, 100, 0, 'synthetic-' || i
               FROM rows;
             INSERT INTO turn_content
                 SELECT rowid, 0, 'text', zeroblob(1275), 0 FROM turn;
             UPDATE session_evidence
                SET status = 'processing', claim_fence = 2,
                    published_fence = 1, evidence_json = '{}';",
        )
        .unwrap();
        tx.commit().unwrap();
    }
    eprintln!("REPRO seed_seconds={:.3}", start.elapsed().as_secs_f64());
    let generation: i64 = store
        .lock()
        .query_row("SELECT source_generation FROM session", [], |r| r.get(0))
        .unwrap();
    let projection = projection_record(record.key.clone(), "synthetic", generation);
    let completion = EvidenceCompletion {
        claim_fence: 2,
        status: PublishedEvidence::Ready,
        evidence_schema_revision: 1,
        evidence_json: crate::store::test_support::evidence_json(&record.key),
    };
    let sources: Vec<SourcePublishOutcome> = std::iter::once("main".to_string())
        .chain((0..855).map(|i| format!("child-{i}")))
        .map(|source_key| SourcePublishOutcome {
            source_key,
            mode: SourcePublishMode::Resumed,
            resume: None,
        })
        .collect();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| {
            let start = Instant::now();
            assert!(
                store
                    .publish_projections(&projection, None, &completion, &[], &sources)
                    .unwrap()
            );
            eprintln!("REPRO publish_seconds={:.3}", start.elapsed().as_secs_f64());
        });
        let mut maximum_wait = std::time::Duration::ZERO;
        while !task.is_finished() {
            let start = Instant::now();
            store.settings().unwrap();
            maximum_wait = maximum_wait.max(start.elapsed());
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        eprintln!(
            "REPRO publish_settings_max_wait_seconds={:.3}",
            maximum_wait.as_secs_f64()
        );
        task.join().unwrap();
    });
    {
        let connection = store.lock();
        let (rows, published, sources, content_rows): (i64, i64, i64, i64) = connection
            .query_row(
                "SELECT COUNT(*), SUM(claim_fence = 1), COUNT(DISTINCT source_key),
                        (SELECT COUNT(*) FROM turn_content) FROM turn",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (rows, published, sources, content_rows),
            (310_907, 310_907, 856, 310_907)
        );
    }
    let start = Instant::now();
    let export = crate::diagnostics_export::build(directory.path(), "0.5.2".into())
        .unwrap()
        .to_json()
        .unwrap();
    eprintln!(
        "REPRO export_seconds={:.3} export_bytes={}",
        start.elapsed().as_secs_f64(),
        export.len()
    );
}

fn timed_row(source_key: &str, turn_index: u64, ts_ms: Option<i64>) -> TurnRow {
    TurnRow {
        ts_ms,
        ..turn_row_for(source_key, turn_index)
    }
}

/// Publishes one pass of `session_id` with a new fingerprint.
fn publish_pass(
    store: &Store,
    session_id: &str,
    fingerprint: &str,
    now_epoch: i64,
    rows: &[TurnRow],
    sources: &[SourcePublishOutcome],
) -> SessionKey {
    let mut record = session(session_id, 1_000);
    record.source_fingerprint = Some(fingerprint.to_owned());
    store
        .upsert_sessions(
            std::slice::from_ref(&record),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let claim = store
        .claim_next_evidence(&["claude-code"], now_epoch, 60)
        .unwrap()
        .expect("pass is pending");
    FencedTurnRowStore::new(store.clone(), record.key.clone(), claim.claim_fence)
        .write_turn_rows(rows)
        .unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    let analysis = projection_record(record.key.clone(), fingerprint, claim.source_generation);
    assert!(
        store
            .publish_projections(&analysis, None, &completion, &[], sources)
            .unwrap()
    );
    record.key
}

fn outcome(source_key: &str, mode: SourcePublishMode) -> SourcePublishOutcome {
    SourcePublishOutcome {
        source_key: source_key.into(),
        mode,
        resume: Some(resume_snapshot("fp")),
    }
}

/// The Overview scan's distinct pairs, read from the turn rows.
fn scanned_minutes(store: &Store) -> Vec<(String, String, String, i64)> {
    let connection = store.lock();
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT t.environment_key, t.agent, t.session_id,
                    (t.ts_ms / 60000) * 60 AS minute_epoch
               FROM turn t
               JOIN session_evidence e
                 ON e.environment_key = t.environment_key
                AND e.agent = t.agent AND e.session_id = t.session_id
                AND e.published_fence = t.claim_fence
              WHERE t.ts_ms > 0 AND t.environment_key NOT LIKE 'ssh:%'
              ORDER BY 1, 2, 3, 4",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn recorded_minutes(store: &Store) -> Vec<(String, String, String, i64)> {
    let connection = store.lock();
    let mut statement = connection
        .prepare(
            "SELECT environment_key, agent, session_id, minute_epoch
               FROM session_active_minute ORDER BY 1, 2, 3, 4",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn assert_minutes_match_scan(store: &Store) -> usize {
    let scanned = scanned_minutes(store);
    assert_eq!(recorded_minutes(store), scanned);
    scanned.len()
}

const MINUTE_MS: i64 = 60_000;

#[test]
fn active_minutes_follow_publish_republish_resume_and_deletion() {
    let store = store();
    let first = [
        timed_row("s1", 0, Some(10 * MINUTE_MS + 5)),
        timed_row("s1", 1, Some(10 * MINUTE_MS + 50_000)),
        timed_row("s1", 2, Some(12 * MINUTE_MS)),
        timed_row("s1", 3, None),
        timed_row("s1", 4, Some(0)),
    ];
    let key = publish_pass(
        &store,
        "active-minutes",
        "sv1:one",
        100,
        &first,
        &[outcome("s1", SourcePublishMode::Full)],
    );
    assert_eq!(assert_minutes_match_scan(&store), 2);

    let second = [
        timed_row("s1", 0, Some(20 * MINUTE_MS)),
        timed_row("s1", 1, Some(21 * MINUTE_MS)),
    ];
    publish_pass(
        &store,
        "active-minutes",
        "sv1:two",
        200,
        &second,
        &[outcome("s1", SourcePublishMode::Full)],
    );
    assert_eq!(assert_minutes_match_scan(&store), 2);

    publish_pass(
        &store,
        "active-minutes",
        "sv1:three",
        300,
        &[
            timed_row("s1", 2, Some(30 * MINUTE_MS)),
            timed_row("s2", 0, Some(31 * MINUTE_MS)),
        ],
        &[
            outcome("s1", SourcePublishMode::Resumed),
            outcome("s2", SourcePublishMode::Full),
        ],
    );
    assert_eq!(assert_minutes_match_scan(&store), 4);

    publish_pass(
        &store,
        "active-minutes",
        "sv1:four",
        400,
        &[timed_row("s1", 0, Some(40 * MINUTE_MS))],
        &[outcome("s1", SourcePublishMode::Full)],
    );
    assert_eq!(assert_minutes_match_scan(&store), 1);

    assert!(store.delete_session(&key).unwrap().is_some());
    assert_eq!(assert_minutes_match_scan(&store), 0);
}

#[test]
fn active_minutes_leave_out_ssh_sessions_like_the_scan() {
    let store = store();
    let mut record = session("ssh-minutes", 1_000);
    record.key = SessionKey::new("ssh:host", "claude-code", "ssh-minutes");
    record.source_fingerprint = Some("sv1:ssh".into());
    store
        .upsert_sessions(
            std::slice::from_ref(&record),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let claim = store
        .claim_next_evidence(&["claude-code"], 100, 60)
        .unwrap()
        .expect("ssh pass is pending");
    FencedTurnRowStore::new(store.clone(), record.key.clone(), claim.claim_fence)
        .write_turn_rows(&[timed_row("s1", 0, Some(10 * MINUTE_MS))])
        .unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    let analysis = projection_record(record.key, "sv1:ssh", claim.source_generation);
    assert!(
        store
            .publish_projections(
                &analysis,
                None,
                &completion,
                &[],
                &[outcome("s1", SourcePublishMode::Full)],
            )
            .unwrap()
    );
    assert_eq!(assert_minutes_match_scan(&store), 0);
}

#[test]
fn active_minutes_follow_retention_pruning_and_the_full_reset() {
    let store = store();
    for session_id in ["pruned-one", "pruned-two"] {
        publish_pass(
            &store,
            session_id,
            &format!("sv1:{session_id}"),
            100,
            &[timed_row("s1", 0, Some(10 * MINUTE_MS))],
            &[outcome("s1", SourcePublishMode::Full)],
        );
    }
    assert_eq!(assert_minutes_match_scan(&store), 2);

    store
        .save_settings(&AppSettings {
            session_data_retention_days: SESSION_DATA_RETENTION_DAYS_30,
            ..AppSettings::default()
        })
        .unwrap();
    assert_eq!(store.apply_session_retention(2_000_000_000).unwrap().0, 2);
    assert_eq!(assert_minutes_match_scan(&store), 0);

    publish_pass(
        &store,
        "reset",
        "sv1:reset",
        100,
        &[timed_row("s1", 0, Some(10 * MINUTE_MS))],
        &[outcome("s1", SourcePublishMode::Full)],
    );
    assert_eq!(assert_minutes_match_scan(&store), 1);
    store.clear_local_session_data().unwrap();
    assert_eq!(assert_minutes_match_scan(&store), 0);
}

#[test]
fn active_minute_backfill_fills_existing_sessions_once() {
    let store = store();
    for index in 0..3 {
        publish_pass(
            &store,
            &format!("backfill-{index}"),
            &format!("sv1:backfill-{index}"),
            100 + index,
            &[
                timed_row("s1", 0, Some(10 * MINUTE_MS)),
                timed_row("s1", 1, Some((11 + index) * MINUTE_MS)),
            ],
            &[outcome("s1", SourcePublishMode::Full)],
        );
    }
    let expected = scanned_minutes(&store);
    assert_eq!(expected.len(), 6);
    store
        .lock()
        .execute("DELETE FROM session_active_minute", [])
        .unwrap();

    store.backfill_active_minutes().unwrap();
    assert_eq!(recorded_minutes(&store), expected);
    let marker: String = store
        .lock()
        .query_row(
            "SELECT value FROM setting WHERE key = 'internal:activeMinutesBackfillV1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(marker, "done");

    store
        .lock()
        .execute("DELETE FROM session_active_minute", [])
        .unwrap();
    store.backfill_active_minutes().unwrap();
    assert!(recorded_minutes(&store).is_empty());
}

type MinuteRow = (String, String, String, i64, String, String, [i64; 13]);

/// The table's rows, or the same rows computed from the turn rows with the
/// `turn_days` filter, grouped by UTC minute. The rows without a positive
/// timestamp group at minute -1.
fn minute_rows(store: &Store, from_table: bool) -> Vec<MinuteRow> {
    let sql = if from_table {
        "SELECT environment_key, agent, session_id, minute_epoch, model, speed,
                row_count, input_tokens, output_tokens, cache_read_tokens,
                cache_write_tokens, cache_write_1h_tokens,
                unrecorded_input_tokens, unrecorded_output_tokens,
                unrecorded_cache_read_tokens, unrecorded_cache_write_tokens,
                unrecorded_cache_write_1h_tokens, recorded_cost_micros, recorded_rows
           FROM session_minute_usage ORDER BY 1, 2, 3, 4, 5, 6"
    } else {
        "SELECT t.environment_key, t.agent, t.session_id,
                CASE WHEN t.ts_ms > 0 THEN (t.ts_ms / 60000) * 60 ELSE -1 END AS m,
                t.model, COALESCE(t.speed, '') AS sp,
                COUNT(*),
                CAST(TOTAL(t.input_tokens) AS INTEGER),
                CAST(TOTAL(t.output_tokens) AS INTEGER),
                CAST(TOTAL(t.cache_read_tokens) AS INTEGER),
                CAST(TOTAL(t.cache_write_tokens) AS INTEGER),
                CAST(TOTAL(t.cache_write_1h_tokens) AS INTEGER),
                CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.input_tokens, 0)) AS INTEGER),
                CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.output_tokens, 0)) AS INTEGER),
                CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.cache_read_tokens, 0)) AS INTEGER),
                CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.cache_write_tokens, 0)) AS INTEGER),
                CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.cache_write_1h_tokens, 0)) AS INTEGER),
                CAST(TOTAL(t.recorded_cost_micros) AS INTEGER),
                COUNT(t.recorded_cost_micros)
           FROM session_evidence e
           JOIN turn t
             ON t.environment_key = e.environment_key AND t.agent = e.agent
            AND t.session_id = e.session_id AND t.claim_fence = e.published_fence
          WHERE t.role = 'assistant' AND t.model IS NOT NULL
            AND (t.input_tokens != 0 OR t.output_tokens != 0
                 OR t.cache_read_tokens != 0 OR t.cache_write_tokens != 0)
          GROUP BY 1, 2, 3, m, 5, sp ORDER BY 1, 2, 3, 4, 5, 6"
    };
    let connection = store.lock();
    let mut statement = connection.prepare(sql).unwrap();
    statement
        .query_map([], |row| {
            let mut sums = [0; 13];
            for (index, sum) in sums.iter_mut().enumerate() {
                *sum = row.get(6 + index)?;
            }
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                sums,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn assert_minutes_match_turn_rows(store: &Store) -> usize {
    let scanned = minute_rows(store, false);
    assert_eq!(minute_rows(store, true), scanned);
    scanned.len()
}

const HOUR_MS: i64 = 3_600_000;

fn usage_row(
    source_key: &str,
    turn_index: u64,
    ts_ms: Option<i64>,
    model: Option<&str>,
    speed: Option<&str>,
    cost: Option<u64>,
) -> TurnRow {
    TurnRow {
        model: model.map(str::to_owned),
        speed: speed.map(str::to_owned),
        cache_read_tokens: 3 + turn_index,
        cache_write_tokens: 2,
        cache_write_1h_tokens: 1,
        recorded_cost_micros: cost,
        ..timed_row(source_key, turn_index, ts_ms)
    }
}

#[test]
fn minute_usage_follows_publish_republish_resume_and_deletion() {
    let store = store();
    let opus = Some("claude-opus-4-6");
    let first = [
        usage_row("s1", 0, Some(HOUR_MS + 5), opus, None, Some(7)),
        usage_row("s1", 1, Some(HOUR_MS + 50_000), opus, None, None),
        usage_row("s1", 2, Some(2 * HOUR_MS), opus, Some("fast"), Some(9)),
        usage_row("s1", 3, None, opus, None, Some(1)),
        usage_row("s1", 4, Some(0), Some("other"), None, None),
        usage_row("s1", 5, Some(3 * HOUR_MS), None, None, None),
        TurnRow {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            ..usage_row("s1", 6, Some(3 * HOUR_MS), opus, None, None)
        },
        TurnRow {
            role: "user",
            ..usage_row("s1", 7, Some(3 * HOUR_MS), opus, None, None)
        },
    ];
    let key = publish_pass(
        &store,
        "hourly-usage",
        "sv1:one",
        100,
        &first,
        &[outcome("s1", SourcePublishMode::Full)],
    );
    // Two dated minutes and two undated groups, one per model.
    assert_eq!(assert_minutes_match_turn_rows(&store), 4);
    let merged = minute_rows(&store, true);
    assert_eq!(merged[0].3, -1);
    assert_eq!(merged[1].3, -1);
    assert_eq!(merged[2].6[0], 2);
    assert_eq!(merged[2].6[11], 7);
    assert_eq!(merged[2].6[12], 1);

    publish_pass(
        &store,
        "hourly-usage",
        "sv1:two",
        200,
        &[
            usage_row("s1", 0, Some(20 * HOUR_MS), opus, None, Some(2)),
            usage_row("s1", 1, Some(21 * HOUR_MS), opus, None, Some(2)),
        ],
        &[outcome("s1", SourcePublishMode::Full)],
    );
    assert_eq!(assert_minutes_match_turn_rows(&store), 2);

    publish_pass(
        &store,
        "hourly-usage",
        "sv1:three",
        300,
        &[
            usage_row("s1", 2, Some(21 * HOUR_MS + 1), opus, None, Some(5)),
            usage_row("s2", 0, Some(31 * HOUR_MS), opus, None, None),
        ],
        &[
            outcome("s1", SourcePublishMode::Resumed),
            outcome("s2", SourcePublishMode::Full),
        ],
    );
    assert_eq!(assert_minutes_match_turn_rows(&store), 3);

    publish_pass(
        &store,
        "hourly-usage",
        "sv1:four",
        400,
        &[usage_row("s1", 0, Some(40 * HOUR_MS), opus, None, None)],
        &[outcome("s1", SourcePublishMode::Full)],
    );
    assert_eq!(assert_minutes_match_turn_rows(&store), 1);

    assert!(store.delete_session(&key).unwrap().is_some());
    assert_eq!(assert_minutes_match_turn_rows(&store), 0);
}

#[test]
fn minute_usage_follows_retention_pruning_and_the_full_reset() {
    let store = store();
    for session_id in ["hourly-pruned-one", "hourly-pruned-two"] {
        publish_pass(
            &store,
            session_id,
            &format!("sv1:{session_id}"),
            100,
            &[usage_row("s1", 0, Some(HOUR_MS), Some("m"), None, Some(1))],
            &[outcome("s1", SourcePublishMode::Full)],
        );
    }
    assert_eq!(assert_minutes_match_turn_rows(&store), 2);

    store
        .save_settings(&AppSettings {
            session_data_retention_days: SESSION_DATA_RETENTION_DAYS_30,
            ..AppSettings::default()
        })
        .unwrap();
    assert_eq!(store.apply_session_retention(2_000_000_000).unwrap().0, 2);
    assert_eq!(assert_minutes_match_turn_rows(&store), 0);

    publish_pass(
        &store,
        "hourly-reset",
        "sv1:reset",
        100,
        &[usage_row("s1", 0, Some(HOUR_MS), Some("m"), None, None)],
        &[outcome("s1", SourcePublishMode::Full)],
    );
    assert_eq!(assert_minutes_match_turn_rows(&store), 1);
    store.clear_local_session_data().unwrap();
    assert_eq!(assert_minutes_match_turn_rows(&store), 0);
}

#[test]
fn minute_usage_backfill_fills_existing_sessions_once() {
    let store = store();
    for index in 0..3 {
        publish_pass(
            &store,
            &format!("hourly-backfill-{index}"),
            &format!("sv1:hourly-backfill-{index}"),
            100 + index,
            &[
                usage_row("s1", 0, Some(HOUR_MS), Some("m"), None, Some(1)),
                usage_row("s1", 1, Some((2 + index) * HOUR_MS), Some("m"), None, None),
            ],
            &[outcome("s1", SourcePublishMode::Full)],
        );
    }
    let expected = minute_rows(&store, false);
    assert_eq!(expected.len(), 6);
    store
        .lock()
        .execute("DELETE FROM session_minute_usage", [])
        .unwrap();

    store.backfill_minute_usage().unwrap();
    assert_eq!(minute_rows(&store, true), expected);
    let marker: String = store
        .lock()
        .query_row(
            "SELECT value FROM setting WHERE key = 'internal:minuteUsageBackfillV2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(marker, "done");

    store
        .lock()
        .execute("DELETE FROM session_minute_usage", [])
        .unwrap();
    store.backfill_minute_usage().unwrap();
    assert!(minute_rows(&store, true).is_empty());
}

/// Every record's turn days as sorted debug strings, sorted by record.
fn sorted_turn_days(store: &Store, utc_offset_seconds: i32) -> Vec<Vec<String>> {
    let evidence = store
        .usage_evidence(
            0,
            utc_offset_seconds,
            crate::store::UsageOrigins::LocalAndRemote,
        )
        .unwrap();
    let mut records: Vec<Vec<String>> = evidence
        .iter()
        .map(|record| {
            let mut days: Vec<String> = record
                .turn_days
                .iter()
                .map(|day| format!("{day:?}"))
                .collect();
            days.sort();
            days
        })
        .collect();
    records.sort();
    records
}

#[test]
fn usage_evidence_reads_the_same_turn_days_from_minute_totals() {
    // 2027-01-11T00:00:00Z. The offsets put a local midnight in the middle of
    // the offsets' own hour, including the +05:30 and +05:45 half hours.
    let monday = 1_799_625_600_i64;
    let at = |seconds: i64, millis: i64| Some((monday + seconds) * 1_000 + millis);
    let opus = Some("claude-opus-4-6");
    let boundaries = [
        -3_600, 18_000, 66_600, 65_700, // local midnights for +60, -300, +330, +345
    ];
    let mut rows = Vec::new();
    for (index, boundary) in boundaries.into_iter().enumerate() {
        let index = index as u64 * 4;
        rows.push(usage_row(
            "s1",
            index,
            at(boundary - 60, 30_000),
            opus,
            None,
            Some(5),
        ));
        rows.push(usage_row(
            "s1",
            index + 1,
            at(boundary, 0),
            opus,
            None,
            Some(0),
        ));
        rows.push(usage_row(
            "s1",
            index + 2,
            at(boundary, 59_999),
            opus,
            Some("fast"),
            None,
        ));
        rows.push(usage_row(
            "s1",
            index + 3,
            at(boundary + 60, 1),
            Some("gpt-5"),
            None,
            None,
        ));
    }
    rows.push(usage_row("s1", 100, None, opus, None, Some(3)));
    rows.push(usage_row("s1", 101, Some(0), Some("gpt-5"), None, None));
    let store = store();
    publish_pass(
        &store,
        "minute-parity-local",
        "sv1:parity-local",
        100,
        &rows,
        &[outcome("s1", SourcePublishMode::Full)],
    );
    // A second session whose rows all carry a recorded cost.
    publish_pass(
        &store,
        "minute-parity-priced",
        "sv1:parity-priced",
        200,
        &[
            usage_row("s1", 0, at(10, 0), Some("local-model"), None, Some(250)),
            usage_row("s1", 1, at(20, 0), Some("local-model"), None, Some(0)),
        ],
        &[outcome("s1", SourcePublishMode::Full)],
    );
    // A synced remote session.
    let mut remote = session("minute-parity-remote", 1_000);
    remote.key = SessionKey::new("ssh:host", "claude-code", "minute-parity-remote");
    remote.source_fingerprint = Some("sv1:parity-remote".into());
    store
        .upsert_sessions(
            std::slice::from_ref(&remote),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let claim = store
        .claim_next_evidence(&["claude-code"], 300, 60)
        .unwrap()
        .expect("remote pass is pending");
    FencedTurnRowStore::new(store.clone(), remote.key.clone(), claim.claim_fence)
        .write_turn_rows(&[
            usage_row("s1", 0, at(66_600, 0), opus, None, Some(9)),
            usage_row("s1", 1, at(66_700, 0), opus, None, None),
        ])
        .unwrap();
    let completion = evidence_completion(
        &claim,
        PublishedEvidence::Ready,
        crate::store::test_support::evidence_json(&claim.key),
    );
    let analysis = projection_record(remote.key, "sv1:parity-remote", claim.source_generation);
    assert!(
        store
            .publish_projections(
                &analysis,
                None,
                &completion,
                &[],
                &[outcome("s1", SourcePublishMode::Full)],
            )
            .unwrap()
    );

    let offsets = [0, 3_600, -18_000, 19_800, 20_700, 7];
    // Before the fill pass completes, the query reads the turn rows.
    assert!(!crate::store::minute_usage::is_filled(&store.lock()).unwrap());
    let from_turn_rows: Vec<_> = offsets
        .iter()
        .map(|offset| sorted_turn_days(&store, *offset))
        .collect();
    assert!(from_turn_rows.iter().all(|records| records.len() == 3));

    store.backfill_minute_usage().unwrap();
    assert!(crate::store::minute_usage::is_filled(&store.lock()).unwrap());
    let from_minutes: Vec<_> = offsets
        .iter()
        .map(|offset| sorted_turn_days(&store, *offset))
        .collect();
    assert_eq!(from_minutes, from_turn_rows);
}
