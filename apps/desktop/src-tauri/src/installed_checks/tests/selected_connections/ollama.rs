use super::*;
use crate::jev_ollama::test_hooks;
use crate::store::native_tool_results::{evidence, publications::PublicationPolicy};
use antiburn_checks::engine::{EvaluationDiagnostic, EvaluationFailureCode};
use antiburn_local::analysis::{
    NativeEvidenceBatch, NativeEvidenceEvent, NativeEvidenceFact, NativeMessageObservation,
    ToolResultObservation,
};

fn harness(a: &Server) -> Harness {
    let fixture = Fixture::new();
    std::fs::write(fixture.global.join("failed-retry-loop.yml"), SOURCE).unwrap();
    let mut connection = a.connection("synthetic-local-model");
    connection.provider = SystemOneProvider::Ollama;
    connection.endpoint = SystemOneEndpoint::BaseUrl(format!("http://{}", a.address));
    headless::setup_keyless_profiles(
        &fixture.store,
        BTreeMap::from([("a".into(), connection)]),
        "a",
        true,
    )
    .unwrap();
    let worker = Arc::new(WorkerHandle::default());
    headless::restore_keyless_profile(&fixture.store, &worker).unwrap();
    Harness::with_worker(fixture, worker)
}

fn correct_native_source(store: &Store, candidate: &str, now_ms: u64) {
    let mut connection = store.lock();
    let tx = connection.transaction().unwrap();
    assert_eq!(
        tx.query_row(
            "SELECT state FROM burn_check_candidate WHERE candidate_id = ?1",
            [candidate],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "dispatching"
    );
    let payload: String = tx
        .query_row(
            "SELECT payload_json FROM native_tool_result_current
         WHERE environment_key = 'native' AND agent = 'pi' AND session_id = 'main'
           AND event_kind = 'tool_result' AND is_removed = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut corrected: ToolResultObservation = serde_json::from_str(&payload).unwrap();
    corrected.revision += 1;
    corrected.content = Some("Corrected synthetic result: parser tests passed.".into());
    let generation: i64 = tx
        .query_row(
            "SELECT source_generation FROM session
         WHERE environment_key = 'native' AND agent = 'pi' AND session_id = 'main'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        evidence::reconcile_evidence_in(
            &tx,
            &SessionKey::new("native", "pi", "main"),
            generation,
            0,
            &NativeEvidenceBatch {
                source_format: SourceFormat::PiV3Jsonl,
                events: vec![NativeEvidenceEvent {
                    live_eligible: true,
                    fact: NativeEvidenceFact::Message(NativeMessageObservation::ToolResult(
                        corrected
                    )),
                }],
                history_complete: true,
            },
            PublicationPolicy {
                suppress_coverage: true,
                ..Default::default()
            },
            i64::try_from(now_ms).unwrap(),
        )
        .unwrap()
    );
    assert_eq!(
        tx.query_row(
            "SELECT state FROM burn_check_candidate WHERE candidate_id = ?1",
            [candidate],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "stale"
    );
    tx.commit().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ollama_inner_slot_release_rechecks_the_exact_claim_before_http_send() {
    let a = Server::new();
    let mut harness = harness(&a);
    harness.publish(1);
    let candidate = pending_candidate(&harness);
    let generation = harness
        .runtime
        .worker
        .execution_snapshot()
        .unwrap()
        .generation;
    let held = Arc::new(Mutex::new(Some(test_hooks::hold_inference_slots().await)));
    let pending = Arc::new(AtomicUsize::new(0));
    let store = Arc::clone(&harness.fixture.store);
    let queued_candidate = candidate.clone();
    let clock = harness.runtime.clock.load(Ordering::Acquire);
    let released = Arc::clone(&held);
    let observed = Arc::clone(&pending);
    a.reply("clear", None);
    let job = harness.host.evaluation_jobs(1).pop().unwrap();
    let runtime: Arc<dyn SelectedJobRuntime> = harness.runtime.clone();
    let task = tokio::spawn(test_hooks::with_inference_slot_pending(
        move || {
            assert_eq!(observed.fetch_add(1, Ordering::AcqRel), 0);
            correct_native_source(&store, &queued_candidate, clock);
            drop(released.lock().unwrap().take().unwrap());
        },
        job.run_selected(runtime),
    ));
    let settlement = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(settlement.outcome, RunOutcome::Stale);
    assert_eq!((settlement.attempts, settlement.new_intents), (1, 1));
    assert_eq!(pending.load(Ordering::Acquire), 1);
    assert!(held.lock().unwrap().is_none());
    assert!(harness.runtime.worker.key_is_current(generation));
    assert_stale_attempt(&harness, &candidate, "permanent");
    let diagnostic: EvaluationDiagnostic = serde_json::from_str(
        &harness
            .fixture
            .store
            .lock()
            .query_row(
                "SELECT diagnostic_json FROM burn_check_candidate WHERE candidate_id = ?1",
                [&candidate],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(diagnostic.code, EvaluationFailureCode::Cancelled);
    harness.host.catch_up().unwrap();
    assert!(harness.job().await.unwrap().unwrap().is_none());
    assert_stale_attempt(&harness, &candidate, "permanent");
    let usage = harness.fixture.store.burn_check_usage_summary().unwrap();
    assert_eq!((usage.confirmed_calls, usage.unknown_outcomes), (0, 0));
    assert_eq!(a.count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ollama_sent_source_cancellation_keeps_unknown_delivery_without_resend() {
    let a = Server::new();
    let mut harness = harness(&a);
    harness.publish(1);
    let candidate = pending_candidate(&harness);
    let generation = harness
        .runtime
        .worker
        .execution_snapshot()
        .unwrap()
        .generation;
    let gate = Arc::new(Gate::default());
    let _release = ReleaseGate(Arc::clone(&gate));
    a.reply("clear", Some(Arc::clone(&gate)));
    let task = harness.job();
    gate.wait().await;
    assert_eq!(a.count(), 1);
    correct_native_source(
        &harness.fixture.store,
        &candidate,
        harness.runtime.clock.load(Ordering::Acquire),
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .unwrap()
            .outcome,
        RunOutcome::Stale
    );
    gate.open();
    assert!(harness.runtime.worker.key_is_current(generation));
    assert_stale_attempt(&harness, &candidate, "unknown");
    harness.host.catch_up().unwrap();
    assert!(harness.job().await.unwrap().unwrap().is_none());
    assert_stale_attempt(&harness, &candidate, "unknown");
    let usage = harness.fixture.store.burn_check_usage_summary().unwrap();
    assert_eq!((usage.confirmed_calls, usage.unknown_outcomes), (0, 1));
    assert_eq!(a.count(), 1);
}
