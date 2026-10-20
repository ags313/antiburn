use antiburn_local::analysis::{
    SourceFormat, ToolResultChange, ToolResultCompleteness, ToolResultObservation,
    ToolResultPosition, ToolResultStatus,
};
use serde_json::json;

use super::*;
use crate::insights_worker::journal::tests::SyntheticJournal;

fn setup() -> (
    tempfile::TempDir,
    Store,
    SyntheticJournal,
    OpenCodeV2CaptureStart,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(directory.path()).unwrap();
    let journal = SyntheticJournal::new(16);
    let mut record = journal.record("/synthetic");
    record.source_label = format!("opencode:{}:open_code_v2_sqlite", record.key.session_id);
    store
        .upsert_sessions(&[record], &crate::agents::evidence_cohort())
        .unwrap();
    SyntheticJournal::enable_live(&store, "/synthetic");
    let (incarnation, source_generation): (i64, i64) = store.lock().query_row(
        "SELECT incarnation, source_generation FROM session WHERE session_id = 'native-session'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let start = OpenCodeV2CaptureStart {
        key: SessionKey::new("native", "opencode", "native-session"),
        incarnation: Incarnation(incarnation as u64),
        source_generation,
        source_label: "opencode:native-session:open_code_v2_sqlite".into(),
        cwd: "/synthetic".into(),
        baseline_sequence: Some(4),
    };
    (directory, store, journal, start)
}

fn started(store: &Store, start: &OpenCodeV2CaptureStart) -> OpenCodeV2CaptureClaim {
    store
        .start_opencode_v2_capture_epoch(std::slice::from_ref(start), "epoch-a", 100, 60)
        .unwrap()
        .unwrap()
        .remove(0)
}

fn result(sequence: u64, complete: bool) -> ToolResultObservation {
    ToolResultObservation {
        source_format: SourceFormat::OpenCodeSqliteV2,
        session_id: "native-session".into(),
        record_id: Some(format!("event-{sequence}")),
        call_id: Some(format!("call-{sequence}")),
        result_id: None,
        position: ToolResultPosition::OpenCodeV2Event {
            sequence,
            item_index: 0,
        },
        revision: 1,
        change: ToolResultChange::Update,
        tool_name: Some("bash".into()),
        tool_category: None,
        arguments: Some(json!({"command":"true"})),
        content: Some("ok".into()),
        status: ToolResultStatus::Success,
        finality: true,
        completeness: ToolResultCompleteness::Complete,
        context: Some(antiburn_local::analysis::ToolResultContext {
            result_position: ToolResultPosition::OpenCodeV2Event {
                sequence,
                item_index: 0,
            },
            main_session: true,
            complete_through_event: complete,
            user_messages: Vec::new(),
        }),
    }
}

#[test]
fn start_requires_exact_live_v2_scope_and_rejects_active_lease_atomically() {
    let (_dir, store, _journal, start) = setup();
    let claim = started(&store, &start);
    assert_eq!(claim.baseline_sequence, Some(4));
    assert_eq!(claim.last_sequence, Some(4));
    assert!(claim.history_gap);
    assert!(
        store
            .start_opencode_v2_capture_epoch(std::slice::from_ref(&start), "epoch-b", 101, 60)
            .unwrap()
            .is_none()
    );

    let (_dir, atomic, _journal, start) = setup();
    let mut absent = start.clone();
    absent.key.session_id = "missing-session".into();
    absent.source_label = "opencode:missing-session:open_code_v2_sqlite".into();
    assert!(
        atomic
            .start_opencode_v2_capture_epoch(&[start, absent], "epoch", 100, 60)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        atomic
            .lock()
            .query_row("SELECT COUNT(*) FROM opencode_v2_capture", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let (_dir, mismatch, _journal, start) = setup();
    mismatch
        .lock()
        .execute(
            "UPDATE session SET source_label = 'opencode:native-session'",
            [],
        )
        .unwrap();
    assert!(
        mismatch
            .start_opencode_v2_capture_epoch(&[start], "epoch", 100, 60)
            .unwrap()
            .is_none()
    );

    let (_dir, inactive, _journal, start) = setup();
    inactive
        .lock()
        .execute("UPDATE burn_check_activation SET enabled = 0", [])
        .unwrap();
    assert!(
        inactive
            .start_opencode_v2_capture_epoch(&[start], "epoch", 100, 60)
            .unwrap()
            .is_none()
    );

    let (_dir, wrong_cwd, _journal, start) = setup();
    wrong_cwd
        .lock()
        .execute(
            "UPDATE burn_check_activation SET activation_scope_key =
                json_set(activation_scope_key, '$.session_cwd', '/other')",
            [],
        )
        .unwrap();
    assert!(
        wrong_cwd
            .start_opencode_v2_capture_epoch(&[start], "epoch", 100, 60)
            .unwrap()
            .is_none()
    );
}

#[test]
fn progress_fences_sequence_and_publication_position_are_enforced() {
    let (_dir, store, _journal, start) = setup();
    let claim = started(&store, &start);
    assert!(
        !store
            .advance_opencode_v2_capture(&claim, 4, Some(&result(4, false)), 101, 60)
            .unwrap()
    );
    assert!(
        !store
            .advance_opencode_v2_capture(&claim, 5, Some(&result(6, false)), 101, 60)
            .unwrap()
    );
    assert!(
        store
            .advance_opencode_v2_capture(&claim, 5, Some(&result(5, false)), 101, 60)
            .unwrap()
    );
    let publication = store
        .native_tool_result_publications_after(0, 20)
        .unwrap()
        .into_iter()
        .find(|event| event.payload.is_some())
        .unwrap();
    assert_eq!(
        publication.source_format.as_deref(),
        Some("\"open_code_v2_sqlite\"")
    );
    assert_eq!(
        publication.native_position,
        Some(ToolResultPosition::OpenCodeV2Event {
            sequence: 5,
            item_index: 0
        })
    );
    assert!(
        store
            .renew_opencode_v2_capture_epoch(std::slice::from_ref(&claim), 102, 60)
            .unwrap()
    );
    let mut stale = claim;
    stale.claim_fence -= 1;
    assert!(
        !store
            .advance_opencode_v2_capture(&stale, 6, None, 102, 60)
            .unwrap()
    );
}

#[test]
fn gap_marking_releases_claim_and_complete_result_restores_coverage() {
    let (_dir, store, _journal, start) = setup();
    let claim = started(&store, &start);
    assert_eq!(
        store
            .mark_opencode_v2_capture_gap(std::slice::from_ref(&claim), "capture-lost", 101)
            .unwrap(),
        1
    );
    assert!(
        !store
            .renew_opencode_v2_capture_epoch(std::slice::from_ref(&claim), 102, 60)
            .unwrap()
    );

    let next = store
        .start_opencode_v2_capture_epoch(std::slice::from_ref(&start), "epoch-b", 102, 60)
        .unwrap()
        .unwrap()
        .remove(0);
    assert!(
        store
            .advance_opencode_v2_capture(&next, 5, Some(&result(5, true)), 103, 60)
            .unwrap()
    );
    assert!(
        store
            .native_tool_result_publications_after(0, 20)
            .unwrap()
            .iter()
            .any(|event| {
                event.outcome
                == crate::store::native_tool_results::ToolResultPublicationOutcome::CoverageRestored
            })
    );
    let gap = store
        .lock()
        .query_row("SELECT history_gap FROM opencode_v2_capture", [], |row| {
            row.get::<_, bool>(0)
        })
        .unwrap();
    assert!(!gap);
}

#[test]
fn startup_recovery_gaps_and_releases_leftover_leases_without_removing_results() {
    let (_dir, store, _journal, start) = setup();
    let claim = started(&store, &start);
    assert!(
        store
            .advance_opencode_v2_capture(&claim, 5, Some(&result(5, false)), 101, 60)
            .unwrap()
    );
    assert_eq!(store.recover_opencode_v2_capture_claims(102).unwrap(), 1);
    assert_eq!(store.recover_opencode_v2_capture_claims(103).unwrap(), 0);
    let state: (bool, Option<i64>) = store
        .lock()
        .query_row(
            "SELECT history_gap, lease_expires_at_epoch FROM opencode_v2_capture",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (true, None));
    assert_eq!(
        store.native_tool_results_current(&claim.key).unwrap().len(),
        1
    );
    assert!(
        store
            .native_tool_result_publications_after(0, 20)
            .unwrap()
            .iter()
            .any(|event| event.outcome
                == crate::store::native_tool_results::ToolResultPublicationOutcome::CoverageGap
                && event.gap_reason.as_deref() == Some("capture-recovered"))
    );
}

#[test]
fn one_session_gap_does_not_release_another_v2_capture_lease() {
    let (_dir, store, journal, start) = setup();
    let mut other_record = journal.record("/synthetic");
    other_record.key.session_id = "other-session".into();
    other_record.source_label = "opencode:other-session:open_code_v2_sqlite".into();
    store
        .upsert_sessions(
            std::slice::from_ref(&other_record),
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let (presence, _) = store
        .session_presence_for_keys(std::slice::from_ref(&other_record.key))
        .unwrap();
    let generation = store
        .source_generation_batch(std::slice::from_ref(&other_record.key))
        .unwrap()[0]
        .unwrap();
    let other_start = OpenCodeV2CaptureStart {
        key: other_record.key.clone(),
        incarnation: presence[0].incarnation,
        source_generation: generation,
        source_label: other_record.source_label,
        cwd: "/synthetic".into(),
        baseline_sequence: Some(4),
    };
    let claims = store
        .start_opencode_v2_capture_epoch(&[start, other_start], "two-session-epoch", 100, 60)
        .unwrap()
        .unwrap();
    let first = claims
        .iter()
        .find(|claim| claim.key.session_id == "native-session")
        .unwrap();
    let second = claims
        .iter()
        .find(|claim| claim.key.session_id == "other-session")
        .unwrap();

    assert_eq!(
        store
            .mark_opencode_v2_capture_gap(std::slice::from_ref(first), "event-join-gap", 101)
            .unwrap(),
        1
    );
    assert!(
        store
            .renew_opencode_v2_capture_epoch(std::slice::from_ref(second), 102, 60)
            .unwrap()
    );
    let mut other_result = result(5, false);
    other_result.session_id = "other-session".into();
    assert!(
        store
            .advance_opencode_v2_capture(second, 5, Some(&other_result), 103, 60)
            .unwrap()
    );
    assert!(
        !store
            .advance_opencode_v2_capture(first, 5, Some(&result(5, false)), 103, 60)
            .unwrap()
    );

    store
        .lock()
        .execute(
            "UPDATE opencode_v2_capture SET claim_fence = claim_fence + 1
          WHERE session_id = 'native-session'",
            [],
        )
        .unwrap();
    assert_eq!(
        store
            .mark_opencode_v2_capture_gap(&claims, "capture-lease-lost", 104)
            .unwrap(),
        1
    );
    assert!(
        !store
            .renew_opencode_v2_capture_epoch(std::slice::from_ref(second), 105, 60)
            .unwrap()
    );
}

#[test]
fn ordinary_content_generations_do_not_end_a_bound_capture_segment() {
    let (_dir, store, _journal, start) = setup();
    let claim = started(&store, &start);
    store
        .lock()
        .execute(
            "UPDATE session SET source_generation = source_generation + 1
          WHERE session_id = 'native-session'",
            [],
        )
        .unwrap();
    assert!(
        store
            .renew_opencode_v2_capture_epoch(std::slice::from_ref(&claim), 101, 60)
            .unwrap()
    );
    assert!(
        store
            .advance_opencode_v2_capture(&claim, 5, Some(&result(5, false)), 102, 60)
            .unwrap()
    );
    let events = store.native_tool_result_publications_after(0, 20).unwrap();
    let update = events.iter().find(|event| event.payload.is_some()).unwrap();
    assert_eq!(update.source_generation, claim.source_generation + 1);

    let mut next_start = start.clone();
    next_start.source_generation += 1;
    assert!(
        store
            .start_opencode_v2_capture_epoch(&[next_start], "competing-worker", 103, 60,)
            .unwrap()
            .is_none(),
        "content changes must not permit a second live lease"
    );

    store
        .lock()
        .execute(
            "UPDATE session SET cwd = '/moved' WHERE session_id = 'native-session'",
            [],
        )
        .unwrap();
    assert!(
        !store
            .advance_opencode_v2_capture(&claim, 6, Some(&result(6, false)), 103, 60)
            .unwrap()
    );
}

#[test]
fn publication_failure_rolls_back_capture_start_and_gap_event() {
    let (_dir, store, _journal, start) = setup();
    store
        .lock()
        .execute_batch(
            "CREATE TEMP TRIGGER reject_v2_gap BEFORE INSERT ON native_tool_result_publication
         WHEN NEW.source_format = '\"open_code_v2_sqlite\"'
           AND NEW.outcome = 'coverage_gap'
         BEGIN SELECT RAISE(ABORT, 'test publication failure'); END;",
        )
        .unwrap();
    assert!(
        store
            .start_opencode_v2_capture_epoch(std::slice::from_ref(&start), "epoch", 100, 60)
            .is_err()
    );
    assert_eq!(
        store
            .lock()
            .query_row("SELECT COUNT(*) FROM opencode_v2_capture", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    store
        .lock()
        .execute_batch("DROP TRIGGER reject_v2_gap;")
        .unwrap();
    let claim = started(&store, &start);
    store
        .lock()
        .execute_batch(
            "CREATE TEMP TRIGGER reject_v2_result BEFORE INSERT ON native_tool_result_publication
             WHEN NEW.outcome = 'update'
             BEGIN SELECT RAISE(ABORT, 'test publication failure'); END;",
        )
        .unwrap();
    assert!(
        store
            .advance_opencode_v2_capture(&claim, 5, Some(&result(5, false)), 101, 60)
            .is_err()
    );
    assert_eq!(
        store
            .lock()
            .query_row("SELECT last_sequence FROM opencode_v2_capture", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .unwrap(),
        Some(4)
    );
}

#[test]
fn v2_capture_does_not_change_v1_cursor_rows() {
    let (_dir, store, _journal, start) = setup();
    store
        .lock()
        .execute(
            "INSERT INTO opencode_journal_delivery (
            environment_key, agent, session_id, incarnation, source_label, cwd,
            parser_revision, cursor_json, main_session, baseline_sequence,
            observed_generation, pending, blocked, history_gap)
         VALUES ('native', 'opencode', 'native-session', ?1, 'opencode:native-session',
            '/synthetic', 1, '{\"cursor\":1}', 1, 10, 1, 0, 0, 0)",
            [start.incarnation.0],
        )
        .unwrap();
    started(&store, &start);
    assert_eq!(store.lock().query_row(
        "SELECT cursor_json FROM opencode_journal_delivery WHERE session_id = 'native-session'",
        [], |row| row.get::<_, String>(0),
    ).unwrap(), "{\"cursor\":1}");
}

#[test]
fn candidates_require_v2_label_live_exact_cwd_main_session_and_recency() {
    let (_dir, store, journal, start) = setup();
    let now = journal.record("/synthetic").updated_at_epoch.unwrap();
    assert_eq!(store.opencode_v2_live_candidates(now, 10).unwrap().len(), 1);
    assert!(
        store
            .opencode_v2_live_candidates(now, 0)
            .unwrap()
            .is_empty()
    );
    assert!(store.opencode_v2_live_candidates(now, 1025).is_err());
    store
        .lock()
        .execute(
            "UPDATE session SET source_label = 'opencode:native-session'",
            [],
        )
        .unwrap();
    assert!(
        store
            .opencode_v2_live_candidates(now, 10)
            .unwrap()
            .is_empty()
    );
    store
        .lock()
        .execute(
            "UPDATE session SET source_label = ?1",
            [&start.source_label],
        )
        .unwrap();
    store
        .lock()
        .execute("UPDATE burn_check_activation SET enabled = 0", [])
        .unwrap();
    assert!(
        store
            .opencode_v2_live_candidates(now, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_admitted_v2_session_stays_active_after_the_recency_window() {
    let (_dir, store, journal, start) = setup();
    let now = journal.record("/synthetic").updated_at_epoch.unwrap();
    let claim = store
        .start_opencode_v2_capture_epoch(std::slice::from_ref(&start), "recency-test", now, 60)
        .unwrap()
        .unwrap()
        .remove(0);
    store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = ?1 WHERE session_id = 'native-session'",
            [now - antiburn_local::discovery::ACTIVE_SESSION_WINDOW_SECS - 1],
        )
        .unwrap();

    let candidates = store.opencode_v2_live_candidates(now, 10).unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].key.session_id, "native-session");

    store
        .mark_opencode_v2_capture_gap(&[claim], "test-release", now + 1)
        .unwrap();
    assert_eq!(
        store
            .opencode_v2_live_candidates(now + 2, 10)
            .unwrap()
            .len(),
        1
    );
    store
        .lock()
        .execute("UPDATE burn_check_activation SET enabled = 0", [])
        .unwrap();
    assert!(
        store
            .opencode_v2_live_candidates(now + 2, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn prior_admission_survives_restart_without_a_metadata_refresh() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    let journal = SyntheticJournal::new(16);
    let mut record = journal.record("/synthetic");
    record.source_label = "opencode:native-session:open_code_v2_sqlite".into();
    store
        .upsert_sessions(&[record], &crate::agents::evidence_cohort())
        .unwrap();
    SyntheticJournal::enable_live(&store, "/synthetic");
    let (incarnation, source_generation) = store
        .lock()
        .query_row(
            "SELECT incarnation, source_generation FROM session",
            [],
            |row| Ok((Incarnation(row.get(0)?), row.get(1)?)),
        )
        .unwrap();
    let start = OpenCodeV2CaptureStart {
        key: SessionKey::new("native", "opencode", "native-session"),
        incarnation,
        source_generation,
        source_label: "opencode:native-session:open_code_v2_sqlite".into(),
        cwd: "/synthetic".into(),
        baseline_sequence: Some(4),
    };
    let claim = started(&store, &start);
    store
        .lock()
        .execute("UPDATE session SET updated_at_epoch = 0", [])
        .unwrap();
    store
        .mark_opencode_v2_capture_gap(&[claim], "worker-stopped", 101)
        .unwrap();
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    assert_eq!(
        store.opencode_v2_live_candidates(1000, 10).unwrap().len(),
        1
    );
    assert!(
        store
            .start_opencode_v2_capture_epoch(&[start], "restart", 1000, 60)
            .unwrap()
            .is_some()
    );
    store
        .lock()
        .execute("UPDATE session SET cwd = '/other'", [])
        .unwrap();
    assert!(
        store
            .opencode_v2_live_candidates(1001, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn clear_local_data_removes_capture_status_selection_and_fences_old_results() {
    let (_directory, store, _journal, start) = setup();
    let claim = started(&store, &start);
    assert!(
        store
            .advance_opencode_v2_capture(&claim, 5, Some(&result(5, false)), 101, 60)
            .unwrap()
    );
    store.clear_local_session_data().unwrap();
    for table in [
        "opencode_v2_capture",
        "opencode_v2_source_status",
        "opencode_v2_selection",
        "native_tool_result_current",
        "native_tool_result_publication",
    ] {
        let count: usize = store
            .lock()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert!(
        !store
            .advance_opencode_v2_capture(&claim, 6, Some(&result(6, false)), 102, 60)
            .unwrap()
    );
    assert_eq!(
        store
            .mark_opencode_v2_capture_gap(&[claim], "worker-stopped", 103)
            .unwrap(),
        0
    );
}

#[test]
fn candidates_exclude_wrong_cwd_child_fork_and_stale_rows_and_apply_order_and_limit() {
    let (_dir, store, journal, _start) = setup();
    let now = journal.record("/synthetic").updated_at_epoch.unwrap();
    for id in ["alpha", "beta", "wrong-cwd", "child", "fork", "stale"] {
        let mut record = journal.record(if id == "wrong-cwd" {
            "/other"
        } else {
            "/synthetic"
        });
        record.key.session_id = id.into();
        record.source_label = format!("opencode:{id}:open_code_v2_sqlite");
        store
            .upsert_sessions(&[record], &crate::agents::evidence_cohort())
            .unwrap();
    }
    store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = ?1 WHERE session_id = 'alpha'",
            [now - 1],
        )
        .unwrap();
    store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = ?1 WHERE session_id = 'beta'",
            [now - 2],
        )
        .unwrap();
    store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = ?1 WHERE session_id = 'stale'",
            [now - antiburn_local::discovery::ACTIVE_SESSION_WINDOW_SECS - 1],
        )
        .unwrap();
    store
        .lock()
        .execute(
            "INSERT INTO session_relation (environment_key, agent, session_id, kind, related_id)
        VALUES ('native', 'opencode', 'parent', 'subagent', 'child'),
               ('native', 'opencode', 'fork', 'forkParent', 'parent')",
            [],
        )
        .unwrap();
    let candidates = store.opencode_v2_live_candidates(now, 10).unwrap();
    let ids = candidates
        .iter()
        .map(|record| record.key.session_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["native-session", "alpha", "beta"]);
    assert_eq!(store.opencode_v2_live_candidates(now, 2).unwrap().len(), 2);
}
