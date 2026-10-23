use super::*;

fn protected_state(store: &Store) -> Vec<String> {
    let connection = store.lock();
    [
        "burn_check_activation",
        "burn_check_observation",
        "burn_check_session_attachment",
        "burn_check_current_status",
        "burn_check_acceptance_watermark",
        "burn_check_cooldown",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                Ok((0..columns)
                    .map(|index| format!("{:?}", row.get_ref(index).unwrap()))
                    .collect::<Vec<_>>())
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        format!("{table}: {rows:?}")
    })
    .collect()
}

#[test]
fn connection_changes_cancel_all_unfinished_states_and_preserve_source_and_status() {
    for state in ["pending", "claimed", "dispatching", "answered"] {
        let directory = tempfile::tempdir().unwrap();
        let (store, adapter, active) =
            fixture_with_store(Arc::new(Store::open(directory.path()).unwrap()));
        let accepted = apply(
            &adapter,
            &active,
            &event(&store, 1, "accepted", "branch-a", 1),
        );
        let assessment = answer(&adapter, &active, claim(&adapter, &accepted), 0.99);
        assert_eq!(
            adapter.accept(&assessment).unwrap(),
            AcceptanceOutcome::Accepted
        );
        let pending = apply(
            &adapter,
            &active,
            &event(&store, 2, "pending", "branch-a", 1),
        );
        let late = match state {
            "pending" => None,
            "claimed" => {
                claim(&adapter, &pending);
                None
            }
            "dispatching" => {
                let claimed = claim(&adapter, &pending);
                adapter
                    .record_dispatch_intent(&claimed, 100_000, Duration::ZERO, 1)
                    .unwrap();
                Some(AssessmentAcceptance {
                    claimed,
                    ..assessment.clone()
                })
            }
            "answered" => Some(answer(&adapter, &active, claim(&adapter, &pending), 0.99)),
            _ => unreachable!(),
        };
        let protected = protected_state(&store);
        let source = adapter.load_source_progress(&active).unwrap();
        store
            .cancel_unfinished_burn_check_assessments_for_connection_change()
            .unwrap();
        if let Some(late) = &late {
            if state == "dispatching" {
                adapter
                    .record_answer(&late.claimed, &late.evaluated)
                    .unwrap();
            }
            assert_eq!(adapter.accept(late).unwrap(), AcceptanceOutcome::Stale);
        }
        // A second transition represents B -> A. It cannot revive cancelled A work.
        store
            .cancel_unfinished_burn_check_assessments_for_connection_change()
            .unwrap();
        assert_eq!(protected_state(&store), protected);
        assert_eq!(adapter.load_source_progress(&active).unwrap(), source);
        assert!(matches!(
            adapter.claim_due(&pending, 100_000, true).unwrap(),
            CandidateClaim::Stale
        ));
        drop(adapter);
        drop(store);

        let store = Arc::new(Store::open(directory.path()).unwrap());
        let adapter =
            StoreCheckStore::new(store.clone(), "restarted".into(), active.id.scope.clone())
                .unwrap();
        adapter.register_activation(&active).unwrap();
        assert!(adapter.load_recovery(None, 10).unwrap().is_empty());
        assert_eq!(protected_state(&store), protected);
        let state: String = store
            .lock()
            .query_row(
                "SELECT state FROM burn_check_candidate WHERE candidate_id = ?1",
                [hex(&pending.fingerprint)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "cancelled");
    }
}

#[test]
fn connection_change_cancels_deferred_acceptance_without_publishing_late_status() {
    let (store, adapter, active) = fixture();
    let candidate = apply(&adapter, &active, &event(&store, 1, "first", "branch-a", 1));
    let assessment = answer(&adapter, &active, claim(&adapter, &candidate), 0.99);
    let _later_page = event(&store, 2, "later", "branch-a", 1);
    assert_eq!(
        adapter.accept(&assessment).unwrap(),
        AcceptanceOutcome::Deferred
    );
    store
        .cancel_unfinished_burn_check_assessments_for_connection_change()
        .unwrap();
    assert_eq!(
        adapter.accept(&assessment).unwrap(),
        AcceptanceOutcome::Stale
    );
    let statuses: u64 = store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM burn_check_current_status",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(statuses, 0);
}

#[test]
fn connection_change_rolls_back_when_cancellation_cannot_be_saved() {
    let (store, adapter, active) = fixture();
    let candidate = apply(&adapter, &active, &event(&store, 1, "first", "branch-a", 1));
    store.lock().execute_batch("CREATE TRIGGER reject_cancel BEFORE UPDATE ON burn_check_candidate WHEN NEW.state = 'cancelled' BEGIN SELECT RAISE(ABORT, 'injected cancellation failure'); END").unwrap();
    assert!(
        store
            .cancel_unfinished_burn_check_assessments_for_connection_change()
            .is_err()
    );
    let history: u64 = store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM burn_check_assessment_history",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history, 0);
    assert!(matches!(
        adapter.claim_due(&candidate, 100_000, true).unwrap(),
        CandidateClaim::Claimed(_)
    ));
}

#[test]
fn terminal_candidate_lookup_is_exact_scoped_bounded_and_propagates_read_errors() {
    let (store, adapter, active) = fixture();
    let mut requested = Vec::new();
    let mut expected = Vec::new();
    let states = [
        "pending",
        "claimed",
        "dispatching",
        "answered",
        "accepted",
        "coalesced",
        "not_assessed",
        "execution_failed",
        "cancelled",
        "stale",
    ];
    for index in 0..states.len() {
        let candidate = apply(
            &adapter,
            &active,
            &event(
                &store,
                index as u64 + 1,
                &format!("item-{index}"),
                &format!("branch-{index}"),
                1,
            ),
        );
        let id = CandidateId::from(&candidate);
        if index >= 4 {
            expected.push(id.clone());
        }
        requested.push(id);
    }
    for (id, state) in requested.iter().zip(states) {
        store
            .lock()
            .execute(
                "UPDATE burn_check_candidate SET state = ?1 WHERE candidate_id = ?2",
                params![state, hex(&id.0)],
            )
            .unwrap();
    }
    store.lock().execute("UPDATE burn_check_candidate SET activation_scope_key = 'other-scope' WHERE candidate_id = ?1", [hex(&expected.pop().unwrap().0)]).unwrap();
    requested.push(CandidateId([255; 32]));
    let terminal = adapter.load_terminal_candidate_ids(&requested).unwrap();
    assert_eq!(
        terminal
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        expected.into_iter().collect()
    );
    assert!(
        adapter
            .load_terminal_candidate_ids(&vec![requested[0].clone(); 129])
            .is_err()
    );
    store
        .lock()
        .execute_batch("ALTER TABLE burn_check_candidate RENAME TO unavailable_candidate")
        .unwrap();
    assert!(adapter.load_terminal_candidate_ids(&requested).is_err());
}
