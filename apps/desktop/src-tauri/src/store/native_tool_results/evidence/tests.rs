use super::*;
use crate::published_evidence_reader::StorePublishedEvidenceReader;
use crate::store::SessionRecord;
use antiburn_checks::domain::{ActivationScope, Generation, RunScope};
use antiburn_checks::source::{PublicationRead, PublishedEvidenceReader};
use antiburn_local::analysis::*;

mod session_selection;

fn fixture() -> (Store, SessionKey, ActivationId) {
    let store = Store::open_in_memory(std::path::Path::new("/synthetic/native-evidence")).unwrap();
    fixture_with_store(store)
}

fn fixture_with_store(store: Store) -> (Store, SessionKey, ActivationId) {
    let key = SessionKey::new("native", "claude-code", "session");
    store
        .upsert_sessions(
            &[SessionRecord {
                key: key.clone(),
                source_kind: "file".into(),
                source_label: "/synthetic/session.jsonl".into(),
                wsl_distro: None,
                title: None,
                title_source: None,
                cwd: Some("/synthetic/repo".into()),
                surface: "cli".into(),
                updated_at_epoch: Some(1000),
                activity_cursor: String::new(),
                activity_source: "native".into(),
                subagent_count: 0,
                fork_parent_session_id: None,
                source_fingerprint: Some("synthetic".into()),
            }],
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let activation = ActivationId {
        definition_id: "one".into(),
        generation: Generation(1),
        scope: ActivationScope {
            environment: "native".into(),
            session_cwd: "/synthetic/repo".into(),
            run: RunScope::Live,
        },
    };
    attach(&store, &key, &activation, 1000);
    (store, key, activation)
}

fn attach(store: &Store, key: &SessionKey, activation: &ActivationId, now_ms: i64) {
    let mut connection = store.lock();
    let tx = connection.transaction().unwrap();
    let scope = crate::check_store::StoreCheckStore::activation_scope_key(&activation.scope);
    tx.execute("INSERT INTO burn_check_activation(check_id, activation_scope_key, definition_revision, enabled,
        generation, bootstrap_sequence, applied_sequence, updated_at_epoch) VALUES (?1, ?2, 'synthetic', 1, 1, 0, 0, 0)
        ON CONFLICT DO NOTHING", params![activation.definition_id, scope]).unwrap();
    let incarnation: i64 = tx
        .query_row("SELECT incarnation FROM session", [], |r| r.get(0))
        .unwrap();
    tx.execute("INSERT INTO burn_check_session_attachment(check_id, activation_scope_key, generation, branch_key, incarnation)
        VALUES (?1, ?2, 1, ?3, ?4) ON CONFLICT DO NOTHING", params![activation.definition_id, scope, key.to_identity_json(), incarnation]).unwrap();
    super::super::observers::attach_in(
        &tx,
        activation,
        key,
        incarnation,
        "validated-receipt",
        now_ms,
    )
    .unwrap();
    tx.commit().unwrap();
}

fn user(revision: u64, text: &str) -> NativeEvidenceEvent {
    NativeEvidenceEvent {
        live_eligible: true,
        fact: NativeEvidenceFact::Message(NativeMessageObservation::User(UserMessageObservation {
            message: UserMessage {
                source_format: SourceFormat::ClaudeJsonl,
                session_id: "session".into(),
                message_id: "user-one".into(),
                position: UserMessagePosition::Jsonl { record_index: 1 },
                origin: UserMessageOrigin::ClaudeUserContent,
                scope: UserMessageScope::MainSession,
                change: UserMessageChange::Present,
                completeness: UserMessageCompleteness::Complete,
                parts: vec![UserMessagePart {
                    item_index: 0,
                    native_part_id: None,
                    text: text.into(),
                }],
                proof_revision: 1,
            },
            revision,
            context: NativeEventContext {
                first_eligible_position: ToolResultPosition::Jsonl {
                    record_index: 1,
                    item_index: 0,
                },
                main_session: true,
                complete_through_event: true,
                user_messages: Vec::new(),
            },
        })),
    }
}

fn publish(store: &Store, session: &SessionKey, events: Vec<NativeEvidenceEvent>, now_ms: i64) {
    let mut connection = store.lock();
    let tx = connection.transaction().unwrap();
    assert!(
        reconcile_evidence_in(
            &tx,
            session,
            1,
            1,
            &NativeEvidenceBatch {
                source_format: SourceFormat::ClaudeJsonl,
                events,
                history_complete: true
            },
            PublicationPolicy {
                suppress_coverage: true,
                ..Default::default()
            },
            now_ms
        )
        .unwrap()
    );
    tx.commit().unwrap();
}

#[test]
fn host_target_filter_precedes_limit_and_empty_page_keeps_request_cursor() {
    let (store, key, activation) = fixture();
    let mut other = activation.clone();
    other.definition_id = "two".into();
    attach(&store, &key, &other, 1000);
    publish(&store, &key, vec![user(1, "Public user input.")], 2000);
    let reader = StorePublishedEvidenceReader::for_project(&store, "native", "/synthetic/repo")
        .with_activation(&activation);
    let first = reader
        .read_batch(PublicationRead {
            after: antiburn_checks::source::PublicationCursor(0),
            limit: 1,
        })
        .unwrap();
    assert_eq!(first.events.len(), 1);
    let second = reader
        .read_batch(PublicationRead {
            after: first.next,
            limit: 1,
        })
        .unwrap();
    assert_eq!(second.events.len(), 1);
    assert!(second.next.0 > first.next.0 + 1);
    let empty = reader
        .read_batch(PublicationRead {
            after: second.next,
            limit: 1,
        })
        .unwrap();
    assert!(empty.events.is_empty());
    assert_eq!(empty.next, second.next);
    assert!(!empty.has_more);
    assert!(
        store
            .native_tool_result_publications_after(0, 1)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn public_correction_keeps_first_eligibility_and_exact_replay_is_empty() {
    let (store, key, _) = fixture();
    publish(&store, &key, vec![user(1, "First public input.")], 2000);
    publish(&store, &key, vec![user(2, "Corrected public input.")], 3000);
    publish(&store, &key, vec![user(2, "Corrected public input.")], 4000);
    let events = store.native_evidence_publications_after(0, 20).unwrap();
    let messages = events
        .iter()
        .filter(|e| e.event_kind == EvidenceEventKind::UserMessage)
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].native_identity, messages[1].native_identity);
    assert_eq!(
        messages[0].first_eligible_publication,
        messages[1].first_eligible_publication
    );
    assert_eq!(
        messages[0].first_eligible_publication,
        Some(messages[0].publication_sequence)
    );
    let source = antiburn_checks::source::PublishedEvidenceSource::new(
        StorePublishedEvidenceReader::new(&store),
        20,
    )
    .unwrap();
    let page = source
        .read_batch(antiburn_checks::source::PublicationCursor(0))
        .unwrap();
    assert!(page.events.iter().any(
        |e| matches!(e, antiburn_checks::source::SourceEvent::Observation { observation, .. }
        if matches!(observation.kind, antiburn_checks::domain::ObservationKind::UserMessage(_)))
    ));
}

#[test]
fn publication_version_uses_its_exact_stored_bytes_not_current_or_reserialized_payload() {
    let (store, key, _) = fixture();
    publish(&store, &key, vec![user(1, "First public input.")], 2000);
    store.lock().execute("INSERT INTO native_tool_result_publication(outcome, environment_key, agent, session_id,
        source_format, native_identity, native_position_json, native_revision, payload_json, complete,
        source_generation, claim_fence, live_eligible, project_cwd, session_incarnation, event_kind, first_eligible_publication)
        SELECT outcome, environment_key, agent, session_id, source_format, native_identity, native_position_json,
            native_revision, ' ' || payload_json || ' ', complete, source_generation, claim_fence, live_eligible,
            project_cwd, session_incarnation, event_kind, first_eligible_publication
        FROM native_tool_result_publication WHERE event_kind = 'user_message'", []).unwrap();
    let sequence = store.lock().last_insert_rowid();
    publish(&store, &key, vec![user(2, "Corrected public input.")], 3000);
    let publication = store
        .native_evidence_publications_after(sequence - 1, 1)
        .unwrap()
        .pop()
        .unwrap();
    let raw: String = store.lock().query_row("SELECT payload_json FROM native_tool_result_publication WHERE publication_sequence = ?1",
        [sequence], |row| row.get(0)).unwrap();
    let Some(EvidencePublicationPayload::Native(payload)) = publication.payload.as_ref() else {
        panic!("native payload expected")
    };
    assert_ne!(raw, payload.json().unwrap());
    let binding = NativeCurrentBinding {
        identity: publication.native_identity.clone().unwrap(),
        event_kind: publication.event_kind,
        source_format: publication.source_format.clone().unwrap(),
        revision: publication.native_revision.unwrap(),
        publication_sequence: sequence,
        payload_sha256: crate::store::burn_checks::payload_sha256(&raw),
    };
    assert_eq!(publication.source_evidence_version, Some(binding.version()));
    assert_ne!(
        current_binding_in(&store.lock(), &key, &binding.identity)
            .unwrap()
            .unwrap()
            .version(),
        binding.version()
    );
    let converted = StorePublishedEvidenceReader::new(&store)
        .read_batch(PublicationRead {
            after: antiburn_checks::source::PublicationCursor((sequence - 1) as u64),
            limit: 1,
        })
        .unwrap()
        .events
        .pop()
        .unwrap();
    assert_eq!(converted.source_evidence_version, Some(binding.version()));
    let normalized = antiburn_checks::source::PublishedEvidenceSource::new(
        StorePublishedEvidenceReader::new(&store),
        1,
    )
    .unwrap()
    .read_batch(antiburn_checks::source::PublicationCursor(
        (sequence - 1) as u64,
    ))
    .unwrap();
    let antiburn_checks::source::SourceEvent::Observation {
        source_evidence_version,
        observation,
        ..
    } = &normalized.events[0]
    else {
        panic!("user observation expected")
    };
    assert_eq!(*source_evidence_version, Some(binding.version()));
    assert_ne!(
        source_evidence_version.as_ref().unwrap(),
        &observation.version
    );
}

#[test]
fn attachment_start_and_idle_are_durable_once_and_activity_replay_does_not_reset() {
    let (store, key, activation) = fixture();
    attach(&store, &key, &activation, 10_000);
    {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        super::super::observers::frontier_in(&tx, &key, "validated-receipt", true, 120_999)
            .unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(
        store
            .native_evidence_publications_for_project_after(
                "native",
                "/synthetic/repo",
                Some(&activation),
                0,
                20
            )
            .unwrap()
            .len(),
        1
    );
    for _ in 0..2 {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        super::super::observers::frontier_in(&tx, &key, "validated-receipt", true, 121_000)
            .unwrap();
        tx.commit().unwrap();
    }
    let events = store
        .native_evidence_publications_for_project_after(
            "native",
            "/synthetic/repo",
            Some(&activation),
            0,
            20,
        )
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_kind, EvidenceEventKind::SessionStart);
    assert_eq!(events[1].event_kind, EvidenceEventKind::SessionIdle);
    assert!(events.iter().all(|e| e.native_identity.is_none()
        && e.native_revision.is_none()
        && e.native_position.is_none()));
    let activity = NativeEvidenceEvent {
        live_eligible: true,
        fact: NativeEvidenceFact::Activity(NativeActivityObservation {
            source_format: SourceFormat::ClaudeJsonl,
            session_id: "session".into(),
            position: ToolResultPosition::Jsonl {
                record_index: 10,
                item_index: 0,
            },
            token: "activity-ten".into(),
            native_at_ms: Some(10),
        }),
    };
    publish(&store, &key, vec![activity.clone()], 122_000);
    publish(&store, &key, vec![activity], 200_000);
    let state: (i64, i64) = store
        .lock()
        .query_row(
            "SELECT activity_revision, idle_deadline_ms FROM burn_check_session_attachment",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (1, 242_000));
}

#[test]
fn disk_reopen_keeps_attachment_deadline_and_emitted_idle_revision() {
    let directory = tempfile::tempdir().unwrap();
    let (store, key, activation) = fixture_with_store(Store::open(directory.path()).unwrap());
    let epoch: String = store
        .lock()
        .query_row(
            "SELECT observer_epoch FROM burn_check_session_attachment",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(store.next_session_idle_deadline().unwrap(), Some(121_000));
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    attach(&store, &key, &activation, 121_000);
    assert_eq!(store.next_session_idle_deadline().unwrap(), Some(121_000));
    assert_eq!(
        store
            .native_evidence_publications_for_project_after(
                "native",
                "/synthetic/repo",
                Some(&activation),
                0,
                20,
            )
            .unwrap()
            .len(),
        1
    );
    {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        super::super::observers::frontier_in(&tx, &key, "revalidated-receipt", true, 121_000)
            .unwrap();
        tx.commit().unwrap();
    }
    drop(store);

    let store = Store::open(directory.path()).unwrap();
    attach(&store, &key, &activation, 200_000);
    {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        super::super::observers::frontier_in(&tx, &key, "revalidated-receipt", true, 200_000)
            .unwrap();
        tx.commit().unwrap();
    }
    let state: (String, i64) = store
        .lock()
        .query_row(
            "SELECT observer_epoch, idle_emitted_revision FROM burn_check_session_attachment",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (epoch, 0));
    assert_eq!(store.next_session_idle_deadline().unwrap(), None);
    let events = store
        .native_evidence_publications_for_project_after(
            "native",
            "/synthetic/repo",
            Some(&activation),
            0,
            20,
        )
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_kind, EvidenceEventKind::SessionStart);
    assert_eq!(events[1].event_kind, EvidenceEventKind::SessionIdle);
}

#[test]
fn idle_frontier_rejects_changed_source_binding_even_with_the_old_receipt() {
    for mutation in [
        "UPDATE session SET source_label = '/synthetic/replaced.jsonl'",
        "UPDATE session SET cwd = '/other'",
        "UPDATE session SET incarnation = incarnation + 1",
    ] {
        let (store, key, _) = fixture();
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        tx.execute(mutation, []).unwrap();
        super::super::observers::frontier_in(&tx, &key, "validated-receipt", true, 121_000)
            .unwrap();
        assert_eq!(tx.query_row(
            "SELECT COUNT(*) FROM native_tool_result_publication WHERE event_kind = 'session_idle'",
            [], |row| row.get::<_, i64>(0),
        ).unwrap(), 0, "{mutation}");
        tx.commit().unwrap();
    }
}

#[test]
fn activity_dependency_fails_immediately_without_selector_consumption() {
    let (store, key, activation) = fixture();
    let mut connection = store.lock();
    let tx = connection.transaction().unwrap();
    let (epoch, incarnation): (String, i64) = tx
        .query_row(
            "SELECT observer_epoch, incarnation FROM burn_check_session_attachment",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let scope = crate::check_store::StoreCheckStore::activation_scope_key(&activation.scope);
    tx.execute("INSERT INTO burn_check_candidate(candidate_id, check_id, activation_scope_key, generation, branch_key,
        session_key_json, trigger_position, selected_publication_head, dependencies_json, frozen_input_json, input_fingerprint,
        state, created_at_epoch, updated_at_epoch) VALUES ('candidate', ?1, ?2, 1, ?3, ?3, 1, 1, '{}', '{}', 'frozen', 'pending', 1, 1)",
        params![activation.definition_id, scope, key.to_identity_json()]).unwrap();
    let binding = super::super::observers::DependencyBinding::SessionActivity {
        observer: SessionObserverKey {
            activation,
            branch: BranchId(key.to_identity_json()),
            incarnation,
            attachment_epoch: epoch,
        },
        activity_revision: 0,
    };
    tx.execute("INSERT INTO burn_check_candidate_dependency(candidate_id, observation_id, session_key_json, source_format,
        native_revision, evidence_version, removed, payload_sha256, event_kind, dependency_binding_json)
        VALUES ('candidate', 'activity', ?1, 'attachment', NULL, '0', 0, '', 'session_idle', ?2)",
        params![key.to_identity_json(), serde_json::to_string(&binding).unwrap()]).unwrap();
    assert!(super::super::observers::typed_dependencies_current_in(&tx, "candidate").unwrap());
    super::super::observers::activity_in(
        &tx,
        &key,
        &NativeActivityObservation {
            source_format: SourceFormat::ClaudeJsonl,
            session_id: "session".into(),
            position: ToolResultPosition::Jsonl {
                record_index: 1,
                item_index: 0,
            },
            token: "new-work".into(),
            native_at_ms: None,
        },
        2000,
    )
    .unwrap();
    assert!(!super::super::observers::typed_dependencies_current_in(&tx, "candidate").unwrap());
    assert_eq!(
        tx.query_row("SELECT state FROM burn_check_candidate", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "stale"
    );
    tx.commit().unwrap();
}

#[test]
fn v88_and_v89_preserve_result_bytes_sequences_high_water_and_foreign_keys() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    for migration in &crate::store::schema::MIGRATIONS[..87] {
        connection.execute_batch(migration).unwrap();
    }
    connection.execute_batch("INSERT INTO native_tool_result_publication(publication_sequence, outcome, environment_key,
        agent, session_id, source_format, native_identity, native_position_json, native_revision, payload_json,
        complete, source_generation, claim_fence, gap_kind) VALUES (7, 'update', 'native', 'fixture', 'session', 'fixture',
        'exact-identity', '{}', 1, '{ \"record_id\" : \"owner\", \"value\" : 1 }', 1, 1, 1, 'partial');
        INSERT INTO native_tool_result_current SELECT 'native', 'fixture', 'session', 'fixture', 'exact-identity',
            '{}', 1, '{ \"record_id\" : \"owner\", \"value\" : 1 }', 1, 0, 7, 1, 1;
        UPDATE sqlite_sequence SET seq = 42 WHERE name = 'native_tool_result_publication';").unwrap();
    connection
        .execute_batch(crate::store::schema::MIGRATIONS[87])
        .unwrap();
    connection
        .execute_batch(crate::store::schema::MIGRATIONS[88])
        .unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT gap_kind FROM native_tool_result_publication",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "partial"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT session_selection_json FROM native_tool_result_publication",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap(),
        None
    );
    let row: (i64, String, String, String) = connection
        .query_row(
            "SELECT publication_sequence, native_identity, payload_json,
        event_kind FROM native_tool_result_publication",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            7,
            "exact-identity".into(),
            "{ \"record_id\" : \"owner\", \"value\" : 1 }".into(),
            "tool_result".into()
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = 'native_tool_result_publication'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        42
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert!(
        connection
            .execute("UPDATE native_tool_result_publication SET complete = 0", [])
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO native_tool_result_publication (outcome, environment_key,
        agent, session_id, source_format, complete, source_generation, claim_fence)
        VALUES ('coverage_complete', 'native', 'fixture', 'session', 'fixture', 1, 1, 1)",
            [],
        )
        .unwrap();
    assert_eq!(connection.last_insert_rowid(), 43);
}

#[test]
fn v88_and_v89_preserve_an_empty_publication_high_water() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    for migration in &crate::store::schema::MIGRATIONS[..87] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute_batch(
            "INSERT INTO native_tool_result_publication (outcome, environment_key,
        agent, session_id, source_format, complete, source_generation, claim_fence)
        VALUES ('coverage_complete', 'native', 'fixture', 'session', 'fixture', 1, 1, 1);
        UPDATE sqlite_sequence SET seq = 42 WHERE name = 'native_tool_result_publication';
        DELETE FROM native_tool_result_publication;",
        )
        .unwrap();
    for migration in &crate::store::schema::MIGRATIONS[87..89] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute(
            "INSERT INTO native_tool_result_publication (outcome, environment_key,
        agent, session_id, source_format, complete, source_generation, claim_fence)
        VALUES ('coverage_complete', 'native', 'fixture', 'session', 'fixture', 1, 1, 1)",
            [],
        )
        .unwrap();
    assert_eq!(connection.last_insert_rowid(), 43);
}

#[test]
fn whole_message_removal_uses_durable_owners_and_replays_bounded_continuation() {
    let (store, key, _) = fixture();
    for chunk in 0..3 {
        let events = (chunk * 100..((chunk + 1) * 100).min(257))
            .map(|index| NativeEvidenceEvent {
                live_eligible: true,
                fact: NativeEvidenceFact::Message(NativeMessageObservation::Public(
                    PublicMessageObservation {
                        source_format: SourceFormat::ClaudeJsonl,
                        session_id: key.session_id.clone(),
                        record_id: "owner".into(),
                        part_id: Some(format!("part-{index}")),
                        call_id: Some(format!("call-{index}")),
                        position: ToolResultPosition::Jsonl {
                            record_index: index + 1,
                            item_index: 0,
                        },
                        revision: index + 1,
                        change: ToolResultChange::Update,
                        eligible: true,
                        completeness: ToolResultCompleteness::Complete,
                        context: Some(NativeEventContext {
                            first_eligible_position: ToolResultPosition::Jsonl {
                                record_index: index + 1,
                                item_index: 0,
                            },
                            main_session: true,
                            complete_through_event: true,
                            user_messages: Vec::new(),
                        }),
                        payload: PublicMessagePayload::ToolCall {
                            tool_name: "Read".into(),
                            input: Some(NativeToolInput::Json(
                                serde_json::json!({"path": "src/lib.rs"}),
                            )),
                            facts: None,
                            provider_executed: Some(false),
                        },
                    },
                )),
            })
            .collect();
        publish(&store, &key, events, 2000);
    }
    let removal = NativeEvidenceBatch {
        source_format: SourceFormat::ClaudeJsonl,
        history_complete: true,
        events: vec![NativeEvidenceEvent {
            live_eligible: true,
            fact: NativeEvidenceFact::MessageRemoval(NativeMessageRemoval {
                source_format: SourceFormat::ClaudeJsonl,
                session_id: key.session_id.clone(),
                message_id: "owner".into(),
                position: ToolResultPosition::Jsonl {
                    record_index: 500,
                    item_index: 0,
                },
                revision: 500,
            }),
        }],
    };
    for (expected_consumed, expected_removed) in [(false, 256), (true, 257), (true, 257)] {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        assert_eq!(
            reconcile_evidence_in(
                &tx,
                &key,
                1,
                1,
                &removal,
                PublicationPolicy {
                    suppress_coverage: true,
                    ..Default::default()
                },
                3000
            )
            .unwrap(),
            expected_consumed
        );
        assert_eq!(
            tx.query_row(
                "SELECT COUNT(*) FROM native_tool_result_current WHERE is_removed = 1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            expected_removed
        );
        tx.commit().unwrap();
    }
    assert!(
        store
            .native_tool_result_publications_after(0, 1000)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn continuity_recovery_starts_a_fresh_period_and_rollback_does_not_reset_activity() {
    let (store, key, _) = fixture();
    {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        super::super::observers::break_continuity_in(&tx, &key).unwrap();
        super::super::observers::frontier_in(&tx, &key, "recovery", false, 300_000).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(store.next_session_idle_deadline().unwrap(), None);
    {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        super::super::observers::frontier_in(&tx, &key, "recovery", true, 300_000).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(store.next_session_idle_deadline().unwrap(), Some(420_000));
    {
        let mut connection = store.lock();
        let tx = connection.transaction().unwrap();
        super::super::observers::activity_in(
            &tx,
            &key,
            &NativeActivityObservation {
                source_format: SourceFormat::ClaudeJsonl,
                session_id: key.session_id.clone(),
                position: ToolResultPosition::Jsonl {
                    record_index: 1,
                    item_index: 0,
                },
                token: "rolled-back".into(),
                native_at_ms: None,
            },
            310_000,
        )
        .unwrap();
    }
    assert_eq!(store.next_session_idle_deadline().unwrap(), Some(420_000));
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT last_activity_token FROM burn_check_session_attachment",
                [],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
}
