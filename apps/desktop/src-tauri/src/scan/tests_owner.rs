use super::super::prepare::{PreparationHooks, PreparationStage};
use super::*;
use std::sync::atomic::AtomicI64;
use tokio::sync::Semaphore;

fn fixture_log(home: &std::path::Path, id: &str) -> SessionLog {
    let project = home.join(".claude/projects/synthetic");
    std::fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{id}.jsonl"));
    std::fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::json!({
                "type": "user", "sessionId": id, "cwd": home,
                "timestamp": "2026-10-19T10:00:00Z",
                "message": {"role": "user", "content": "Repair the fixture"},
            })
        ),
    )
    .unwrap();
    SessionLog {
        agent_type: AgentKind::Claude,
        source: SessionSource::File(path),
        updated_at: Some(unix_now()),
        environment: DiscoveryEnvironment::Native,
    }
}

fn writer(store: &Store) -> IncrementalAdmission<'_> {
    IncrementalAdmission {
        io: AdmissionIo::Test {
            cancel: Arc::new(AtomicBool::new(false)),
            reports: Arc::new(Mutex::new(Vec::new())),
            now: Arc::new(AtomicI64::new(unix_now())),
            cancel_after_report: false,
        },
        store,
        previous_records: HashMap::new(),
        evidence_agents: agents::evidence_cohort(),
        write_fence: store.capture_destructive_write_epoch(),
        error: None,
        outcomes: HashMap::new(),
        list_changed: false,
    }
}

fn policy(home: &std::path::Path, hooks: Option<Arc<PreparationHooks>>) -> OwnerPolicy {
    OwnerPolicy {
        home: home.to_path_buf(),
        ignored: HashSet::new(),
        cutoff: Some(0),
        include_non_repo_folders: true,
        progress: false,
        refresh_repositories: false,
        hooks,
    }
}

fn batch(logs: Vec<SessionLog>) -> DiscoveryBatch {
    DiscoveryBatch {
        agent: AgentKind::Claude,
        candidates: logs,
        complete: false,
    }
}

async fn wait_for_row(store: &Store, id: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !store
            .session_records()
            .unwrap()
            .values()
            .any(|record| record.key.session_id == id)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the live row commits while background work stays blocked");
}

async fn barrier_live_commit(stage: PreparationStage) {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(home.path()).unwrap();
    let background = (0..16)
        .map(|index| fixture_log(home.path(), &format!("background-{index}")))
        .collect::<Vec<_>>();
    let live_log = fixture_log(home.path(), "live");
    let (entered, mut arrivals) = mpsc::unbounded_channel();
    let hooks = Arc::new(PreparationHooks {
        stage,
        blocked: background.iter().map(SessionLog::source_label).collect(),
        panic: HashSet::new(),
        panic_once: AtomicBool::new(false),
        entered,
        release: Semaphore::new(0),
    });
    let policy = policy(home.path(), Some(hooks.clone()));
    let mut writer = writer(&store);
    let (finite_sender, finite) = mpsc::channel(2);
    let (live_sender, live) = mpsc::channel(2);
    finite_sender.send(batch(background)).await.unwrap();
    let control = async {
        for _ in 0..15 {
            tokio::time::timeout(Duration::from_secs(10), arrivals.recv())
                .await
                .unwrap()
                .unwrap();
        }
        assert!(
            arrivals.try_recv().is_err(),
            "the sixteenth background slot stays reserved"
        );
        live_sender.send(batch(vec![live_log])).await.unwrap();
        wait_for_row(&store, "live").await;
        assert_eq!(store.session_records().unwrap().len(), 1);
        hooks.release.add_permits(16);
        drop(finite_sender);
    };
    let (described, ()) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(run(finite, live, None, &policy, &mut writer), control)
    })
    .await
    .unwrap();
    writer.finish().unwrap();
    assert_eq!(
        described.records.len(),
        16,
        "live work does not enter finite coverage"
    );
    assert_eq!(store.session_records().unwrap().len(), 17);
}

#[tokio::test]
async fn live_commits_with_all_background_identity_slots_blocked() {
    barrier_live_commit(PreparationStage::Identity).await;
}

#[tokio::test]
async fn live_commits_with_all_background_filters_blocked() {
    barrier_live_commit(PreparationStage::Filter).await;
}

#[tokio::test]
async fn live_commits_with_all_background_title_lookups_blocked() {
    barrier_live_commit(PreparationStage::Title).await;
}

#[tokio::test]
async fn live_commits_with_all_background_git_lookups_blocked() {
    barrier_live_commit(PreparationStage::Git).await;
}

#[tokio::test]
async fn live_commits_before_discovery_closes_and_outside_the_finite_agent_scope() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(home.path()).unwrap();
    let mut writer = writer(&store);
    let policy = policy(home.path(), None);
    let (finite_sender, finite) = mpsc::channel(2);
    let (live_sender, live) = mpsc::channel(2);
    let control = async {
        live_sender
            .send(batch(vec![fixture_log(
                home.path(),
                "live-during-discovery",
            )]))
            .await
            .unwrap();
        wait_for_row(&store, "live-during-discovery").await;
        assert!(
            !finite_sender.is_closed(),
            "finite discovery is still blocked"
        );
        drop(finite_sender);
    };
    let (described, ()) = tokio::join!(run(finite, live, None, &policy, &mut writer), control);
    writer.finish().unwrap();
    assert!(described.records.is_empty());
    let AdmissionIo::Test { reports, .. } = &writer.io else {
        unreachable!()
    };
    assert!(reports.lock().unwrap().iter().any(|report| matches!(report,
        session_lifecycle::Observation::Indexed { sessions, .. } if sessions.iter().any(|session| session.key.session_id == "live-during-discovery")
    )));
}

#[tokio::test]
async fn continuous_equal_and_backward_live_hints_do_not_extend_finite_completion() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(home.path()).unwrap();
    let background = fixture_log(home.path(), "finite");
    let mut live_log = fixture_log(home.path(), "continuous-live");
    let (entered, mut arrivals) = mpsc::unbounded_channel();
    let hooks = Arc::new(PreparationHooks {
        stage: PreparationStage::Git,
        blocked: HashSet::from([background.source_label()]),
        panic: HashSet::new(),
        panic_once: AtomicBool::new(false),
        entered,
        release: Semaphore::new(0),
    });
    let policy = policy(home.path(), Some(hooks.clone()));
    let mut writer = writer(&store);
    let (finite_sender, finite) = mpsc::channel(2);
    let (live_sender, live) = mpsc::channel(2);
    finite_sender.send(batch(vec![background])).await.unwrap();
    let continuous = async {
        let mut turn = 0;
        loop {
            live_log.updated_at = Some(if turn % 2 == 0 { 100 } else { 99 });
            if live_sender
                .send(batch(vec![live_log.clone()]))
                .await
                .is_err()
            {
                break;
            }
            turn += 1;
            tokio::task::yield_now().await;
        }
        turn
    };
    let control = async {
        arrivals.recv().await.unwrap();
        wait_for_row(&store, "continuous-live").await;
        hooks.release.add_permits(1);
        drop(finite_sender);
    };
    let (described, (), turns) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            run(finite, live, None, &policy, &mut writer),
            control,
            continuous
        )
    })
    .await
    .unwrap();
    writer.finish().unwrap();
    assert_eq!(described.records.len(), 1);
    assert_eq!(described.records[0].key.session_id, "finite");
    assert!(turns > 1);
}

#[tokio::test]
async fn panic_releases_physical_identity_for_a_pending_dirty_refresh() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(home.path()).unwrap();
    let log = fixture_log(home.path(), "panic-then-refresh");
    let (entered, mut arrivals) = mpsc::unbounded_channel();
    let hooks = Arc::new(PreparationHooks {
        stage: PreparationStage::Identity,
        blocked: HashSet::from([log.source_label()]),
        panic: HashSet::from([log.source_label()]),
        panic_once: AtomicBool::new(true),
        entered,
        release: Semaphore::new(0),
    });
    let policy = policy(home.path(), Some(hooks.clone()));
    let mut writer = writer(&store);
    let (finite_sender, finite) = mpsc::channel(2);
    let (live_sender, live) = mpsc::channel(2);
    finite_sender.send(batch(vec![log.clone()])).await.unwrap();
    let control = async {
        arrivals.recv().await.unwrap();
        live_sender.send(batch(vec![log])).await.unwrap();
        hooks.release.add_permits(2);
        arrivals.recv().await.unwrap();
        wait_for_row(&store, "panic-then-refresh").await;
        drop(finite_sender);
    };
    tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(run(finite, live, None, &policy, &mut writer), control)
    })
    .await
    .unwrap();
    assert!(
        writer.finish().is_err(),
        "the panic remains an explicit failed pass"
    );
}

#[tokio::test]
async fn cancellation_aborts_blocked_preparation_and_fences_late_results() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory(home.path()).unwrap();
    let log = fixture_log(home.path(), "cancelled");
    let (entered, mut arrivals) = mpsc::unbounded_channel();
    let hooks = Arc::new(PreparationHooks {
        stage: PreparationStage::Git,
        blocked: HashSet::from([log.source_label()]),
        panic: HashSet::new(),
        panic_once: AtomicBool::new(false),
        entered,
        release: Semaphore::new(0),
    });
    let policy = policy(home.path(), Some(hooks.clone()));
    let mut writer = writer(&store);
    let AdmissionIo::Test { cancel, .. } = &writer.io else {
        unreachable!()
    };
    let cancel = cancel.clone();
    let (finite_sender, finite) = mpsc::channel(2);
    let (_live_sender, live) = mpsc::channel(2);
    finite_sender.send(batch(vec![log])).await.unwrap();
    let control = async {
        arrivals.recv().await.unwrap();
        cancel.store(true, Ordering::SeqCst);
    };
    let (described, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(run(finite, live, None, &policy, &mut writer), control)
    })
    .await
    .unwrap();
    hooks.release.add_permits(1);
    tokio::task::yield_now().await;
    assert!(described.records.is_empty());
    assert!(store.session_records().unwrap().is_empty());
}
