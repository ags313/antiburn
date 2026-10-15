use super::*;
use crate::insights_worker::journal::tests::{SyntheticJournal, run_page};

fn catch_up(host: &mut InstalledCheckHost<ScriptedEvaluator>) {
    for _ in 0..20 {
        if !host.catch_up().unwrap().1 {
            return;
        }
    }
    panic!("synthetic host catch-up exceeded its page budget");
}

fn setup(store: &Store, global: &Path, project: &Path) {
    std::fs::create_dir_all(global).unwrap();
    std::fs::create_dir_all(project.join(".antiburn/checks")).unwrap();
    std::fs::write(global.join("failed-retry-loop.yml"), CHECK).unwrap();
    store.replace_repositories(&[repository(project)]).unwrap();
}

#[tokio::test]
async fn native_pages_over_four_mib_restart_and_active_appends_reach_the_production_host() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let global = root.join("global/checks");
    let project = root.join("project");
    let state = root.join("state");
    let store = Arc::new(Store::open(&state).unwrap());
    setup(&store, &global, &project);
    let mut source = SyntheticJournal::new(40 * 1024);
    source.results(0..300);
    source.index(&store, project.to_str().unwrap());
    let evaluator = Arc::new(ScriptedEvaluator::default());
    let mut host =
        InstalledCheckHost::new(store.clone(), evaluator.clone(), Some(global.clone())).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    let activation = host.projects.values().next().unwrap().active[0].clone();
    assert_eq!(activation.bootstrap_publication.0, 0);
    assert!(run_page(&store, source.source(), 100).await);
    assert_eq!(store.next_opencode_journal_attempt(100).unwrap(), Some(101));
    catch_up(&mut host);
    assert_eq!(
        store
            .lock()
            .query_row("SELECT COUNT(*) FROM burn_check_candidate", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert!(evaluator.inputs.lock().unwrap().is_empty());
    drop(host);
    drop(store);

    let store = Arc::new(Store::open(&state).unwrap());
    let mut host =
        InstalledCheckHost::new(store.clone(), evaluator.clone(), Some(global.clone())).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.projects.values().next().unwrap().active[0], activation);
    source.results(300..303);
    source.index(&store, project.to_str().unwrap());
    let mut pages = 1;
    for now in 101..110 {
        if !run_page(&store, source.source(), now).await {
            break;
        }
        pages += 1;
        catch_up(&mut host);
    }
    assert!(pages >= 3);
    assert!(store.next_opencode_journal_attempt(110).unwrap().is_none());
    let counts: (i64, i64, i64) = store
        .lock()
        .query_row(
            "SELECT COUNT(*), SUM(live_eligible = 0), SUM(live_eligible = 1)
           FROM native_tool_result_publication WHERE outcome = 'update'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(counts, (303, 300, 3));
    assert!(
        store
            .lock()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_tool_result_publication
        WHERE outcome = 'update' AND source_generation > 1 AND live_eligible = 0)",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM native_tool_result_current WHERE is_removed = 1",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    evaluator.answer("stuck");
    let outcome = host
        .evaluation_jobs(1)
        .pop()
        .unwrap()
        .run()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.outcome, RunOutcome::Accepted);
    let input = evaluator.inputs.lock().unwrap()[0].clone();
    assert_eq!(input["activity"].as_array().unwrap().len(), 3);
    let work = input["activity"].to_string();
    for index in 300..303 {
        assert!(work.contains(&format!("Parser failure {index}")));
    }
    assert!(!work.contains("Parser failure 299"));
    assert_eq!(input["source_coverage"], "complete");
    let status = status_writes(&store, &host.definitions[0]);
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].0, "failure");
    store.lock().execute("UPDATE session_evidence SET status = 'failed', last_error = 'source-unreadable:no-events'", []).unwrap();
    let report = read_installed_checks(&store, &host.definitions).unwrap();
    assert_eq!(
        (
            report[0].finding,
            report[0].clean,
            report[0].waiting_for_data_count
        ),
        (1, 0, 0)
    );
    assert_eq!(report[0].targets.len(), 1);
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let evidence = crate::insights_report::installed_checks::read_evidence(
        &store,
        &host.definitions,
        &crate::insights_report::ReportRequest {
            environment_key: "native".into(),
            window: antiburn_local::insights::ReportWindow {
                start_epoch: now - 30 * 86_400,
                end_epoch: now + 1,
            },
            computed_at_epoch: now,
        },
        &report[0].targets[0].id,
    )
    .unwrap();
    assert_eq!(
        evidence.status,
        crate::remediation::BurnCheckEvidenceStatus::Available
    );
    drop(host);
    drop(store);

    let store = Arc::new(Store::open(&state).unwrap());
    let mut host = InstalledCheckHost::new(store.clone(), evaluator.clone(), Some(global)).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    catch_up(&mut host);
    assert!(!run_page(&store, source.source(), 200).await);
    assert_eq!(evaluator.inputs.lock().unwrap().len(), 1);
    assert_eq!(status_writes(&store, &host.definitions[0]), status);
    source.result(303);
    source.index(&store, project.to_str().unwrap());
    assert!(run_page(&store, source.source(), 201).await);
    catch_up(&mut host);
    evaluator.answer("clear");
    let now_ms =
        u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000).unwrap();
    assert_eq!(
        run_due_at(&mut host, now_ms + 61_000).await.outcome,
        RunOutcome::Accepted
    );
    assert_eq!(evaluator.inputs.lock().unwrap().len(), 2);
    assert_eq!(status_writes(&store, &host.definitions[0])[0].0, "success");
    let report = read_installed_checks(&store, &host.definitions).unwrap();
    assert_eq!(
        (
            report[0].finding,
            report[0].clean,
            report[0].waiting_for_data_count
        ),
        (0, 1, 0)
    );
}

#[tokio::test]
async fn journal_gap_diagnostics_and_partial_coverage_survive_host_and_store_restart() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let global = root.join("global/checks");
    let project = root.join("project");
    let state = root.join("state");
    let store = Arc::new(Store::open(&state).unwrap());
    setup(&store, &global, &project);
    let mut source = SyntheticJournal::new(16);
    source.result(0);
    source.index(&store, project.to_str().unwrap());
    let evaluator = Arc::new(ScriptedEvaluator::default());
    let mut host =
        InstalledCheckHost::new(store.clone(), evaluator.clone(), Some(global.clone())).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    assert!(run_page(&store, source.source(), 100).await);
    catch_up(&mut host);
    source.event(
        "message.part.removed.1",
        json!({"sessionID":"native-session","messageID":"wrong-message","partID":"part-0"}),
    );
    source.results(1..4);
    source.index(&store, project.to_str().unwrap());
    assert!(run_page(&store, source.source(), 101).await);
    catch_up(&mut host);
    evaluator.answer("stuck");
    assert_eq!(
        host.evaluation_jobs(1)
            .pop()
            .unwrap()
            .run()
            .await
            .unwrap()
            .unwrap()
            .outcome,
        RunOutcome::Accepted
    );
    assert_eq!(
        evaluator.inputs.lock().unwrap()[0]["source_coverage"],
        "partial"
    );
    assert_eq!(
        read_installed_checks(&store, &host.definitions).unwrap()[0].finding,
        1
    );
    drop(host);
    drop(store);

    let store = Arc::new(Store::open(&state).unwrap());
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT last_error FROM opencode_journal_delivery",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "journal-history-incomplete"
    );
    let mut host = InstalledCheckHost::new(store.clone(), evaluator.clone(), Some(global)).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    source.result(4);
    source.index(&store, project.to_str().unwrap());
    assert!(run_page(&store, source.source(), 102).await);
    catch_up(&mut host);
    evaluator.answer("uncertain");
    let now_ms =
        u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000).unwrap();
    run_due_at(&mut host, now_ms + 61_000).await;
    assert_eq!(
        evaluator.inputs.lock().unwrap()[1]["source_coverage"],
        "partial"
    );
    assert_eq!(
        store
            .lock()
            .query_row(
                "SELECT last_error FROM opencode_journal_delivery",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "journal-history-incomplete"
    );
    assert!(
        store
            .lock()
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM native_tool_result_publication
        WHERE outcome = 'coverage_complete')",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
}
