use std::collections::VecDeque;

use antiburn_checks::actions::ChoiceResult;
use antiburn_checks::check_store::{CandidateClaim, EvaluatedCandidate, RecoveryWork};
use antiburn_checks::definition_lifecycle::{InactiveKind, LifecycleState, LifecycleTransition};
use antiburn_checks::domain::{FrozenCandidate, Generation};
use antiburn_checks::engine::RunOutcome;
use antiburn_local::analysis::{
    CompositeSink, EvidenceSource, RawSource, SessionEvidenceAccumulator, SessionInput,
    SessionMetricsAccumulator, SourceCapabilities, SourceFormat, SourceKind, reader_for,
};
use serde_json::{Value, json};

use super::*;
use crate::insights_report::installed_checks::read_installed_checks;
use crate::store::{
    AnalysisRecord, EvidenceCompletion, PublishedEvidence, RepositoryRecord, SessionKey,
    SessionRecord, ToolResultBatch, ToolResultPublishOptions,
};

const CHECK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../docs/burn-check/examples/failed-retry-loop.yml"
));

#[test]
fn disabling_yaml_discovery_deactivates_and_fences_active_generations() {
    let fixture = Fixture::new();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    let scope = host.projects.keys().next().unwrap().clone();
    let active = host.projects[&scope].active[0].clone();

    host.deactivate_active().unwrap();

    assert!(host.projects[&scope].active.is_empty());
    assert!(matches!(
        host.projects[&scope]
            .store
            .load_lifecycle("failed-retry-loop", None)
            .unwrap(),
        LifecycleState::Inactive {
            generation: Generation(2),
            inactive_kind: InactiveKind::Paused,
            ..
        }
    ));
    assert_eq!(active.id.generation, Generation(1));
}

#[derive(Default)]
struct ScriptedEvaluator {
    answers: Mutex<VecDeque<Result<&'static str, AttemptFailure>>>,
    inputs: Mutex<Vec<Value>>,
    started: Notify,
    release: Notify,
    wait: AtomicBool,
}

impl ScriptedEvaluator {
    fn answer(&self, label: &'static str) {
        self.answers.lock().unwrap().push_back(Ok(label));
    }
}

impl DecisionEvaluator for ScriptedEvaluator {
    fn evaluate<'a>(
        &'a self,
        activation: &'a EffectiveActivation,
        _: &'a FrozenCandidate,
        input: &'a Value,
        attempt_number: u8,
    ) -> DecisionFuture<'a> {
        Box::pin(async move {
            self.inputs.lock().unwrap().push(input.clone());
            self.started.notify_one();
            if self.wait.load(Ordering::Acquire) {
                self.release.notified().await;
            }
            let label = self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted attempt");
            match label {
                Ok(label) => {
                    let decision = &activation.compiled.decision;
                    let labels = decision
                        .criteria
                        .iter()
                        .map(|(label, _)| label.clone())
                        .collect::<Vec<_>>();
                    Ok(EvaluatedCandidate {
                        choices: vec![
                            ChoiceResult::validate(
                                &decision.name,
                                &decision.question_id,
                                &labels,
                                label,
                                labels
                                    .iter()
                                    .map(|candidate| {
                                        (
                                            candidate.clone(),
                                            if candidate == label { 0.99 } else { 0.005 },
                                        )
                                    })
                                    .collect(),
                            )
                            .unwrap(),
                        ],
                        attempts: attempt_number,
                    })
                }
                Err(kind) => Err(EvaluationFailure {
                    kind,
                    detail: "scripted provider failure".into(),
                    attempts: attempt_number,
                }),
            }
        })
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    store: Arc<Store>,
    evaluator: Arc<ScriptedEvaluator>,
    global: PathBuf,
    project: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let global = root.join("global/checks");
        let project = root.join("project");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::create_dir_all(project.join(".antiburn/checks")).unwrap();
        std::fs::write(global.join("failed-retry-loop.yml"), CHECK).unwrap();
        let store = Arc::new(Store::open_in_memory(directory.path()).unwrap());
        let fixture = Self {
            directory,
            store,
            evaluator: Arc::new(ScriptedEvaluator::default()),
            global,
            project,
        };
        fixture
            .store
            .replace_repositories(&[repository(&fixture.project)])
            .unwrap();
        fixture.index("native", "main", 0);
        fixture.publish("main", 0);
        fixture
    }

    fn index(&self, environment: &str, session_id: &str, count: usize) {
        self.store
            .upsert_sessions(
                &[SessionRecord {
                    key: SessionKey::new(environment, "pi", session_id),
                    source_kind: "file".into(),
                    source_label: self
                        .directory
                        .path()
                        .join(format!("{session_id}.jsonl"))
                        .to_string_lossy()
                        .into_owned(),
                    wsl_distro: None,
                    title: Some("Parser tests".into()),
                    title_source: Some("explicit".into()),
                    cwd: Some(self.project.to_string_lossy().into_owned()),
                    surface: "cli".into(),
                    updated_at_epoch: Some(time::OffsetDateTime::now_utc().unix_timestamp()),
                    activity_cursor: String::new(),
                    activity_source: "event".into(),
                    subagent_count: 0,
                    fork_parent_session_id: None,
                    source_fingerprint: Some(format!("{session_id}-{count}")),
                }],
                &crate::agents::evidence_cohort(),
            )
            .unwrap();
    }

    fn host(&self) -> InstalledCheckHost<ScriptedEvaluator> {
        InstalledCheckHost::new(
            Arc::clone(&self.store),
            Arc::clone(&self.evaluator),
            Some(self.global.clone()),
        )
        .unwrap()
    }

    fn publish(&self, session_id: &str, count: usize) {
        self.publish_in("native", session_id, count);
    }

    fn publish_in(&self, environment: &str, session_id: &str, count: usize) {
        self.index(environment, session_id, count);
        let key = SessionKey::new(environment, "pi", session_id);
        let input = SessionInput {
            agent: "pi".into(),
            session_id: session_id.into(),
            source: RawSource::Jsonl(transcript(session_id, count)),
            source_format: SourceFormat::PiV3Jsonl,
            fork_parent_session_id: None,
        };
        let mut sink = CompositeSink::new(
            SessionMetricsAccumulator::new("pi", session_id),
            SessionEvidenceAccumulator::new(EvidenceSource {
                agent: "pi".into(),
                session_id: session_id.into(),
                kind: SourceKind::Jsonl,
                capabilities: SourceCapabilities::pi(),
            }),
        );
        reader_for("pi").visit(&input, &mut sink).unwrap();
        let results = sink.tool_result_observations();
        assert_eq!(results.len(), count);
        assert!(
            results
                .iter()
                .all(|result| result.context.as_ref().unwrap().main_session)
        );
        let claim = self
            .store
            .claim_next_evidence_by_recency(&["pi"], 10_000, 60)
            .unwrap()
            .unwrap();
        assert_eq!(claim.key, key);
        let revisions = crate::analysis::projection_revisions();
        let fingerprint = format!("{session_id}-{count}");
        assert!(
            self.store
                .publish_projections_with_native_tool_results(
                    &AnalysisRecord {
                        key: key.clone(),
                        model_breakdown_json: "{}".into(),
                        pricing_breakdown_json: "{}".into(),
                        inclusive_models_json: "[]".into(),
                        initial_context_json: None,
                        source_summaries_json: None,
                        provider_hints_json: None,
                        source_fingerprint: fingerprint.clone(),
                        pricing_generation: 1,
                        analyzed_generation: claim.source_generation,
                        parser_revision: revisions.parser_revision,
                        analyzer_revision: revisions.analyzer_revision,
                        metrics_schema_revision: revisions.metrics_schema_revision,
                    },
                    None,
                    &EvidenceCompletion {
                        claim_fence: claim.claim_fence,
                        status: PublishedEvidence::Ready,
                        evidence_schema_revision:
                            antiburn_local::analysis::EVIDENCE_SCHEMA_REVISION,
                        evidence_json: crate::store::test_support::evidence_json(&key),
                    },
                    &[],
                    &[],
                    ToolResultPublishOptions {
                        source_fingerprint: Some(&fingerprint),
                        batch: Some(&ToolResultBatch {
                            source_format: SourceFormat::PiV3Jsonl,
                            results: results.to_vec(),
                            complete: true
                        }),
                    },
                )
                .unwrap()
        );
    }
}

fn repository(root: &Path) -> RepositoryRecord {
    RepositoryRecord {
        key: root.to_string_lossy().into_owned(),
        repo_name: "synthetic".into(),
        full_name: "test/synthetic".into(),
        status: "accessible".into(),
        repo_root: Some(root.to_string_lossy().into_owned()),
        suspected_path: None,
        worktree_count: 1,
        session_count: 0,
        wsl_distro: None,
        enabled: true,
    }
}

fn status_writes(store: &Store, definition: &InstalledCheckDefinition) -> Vec<(String, String)> {
    store.lock().prepare(
        "SELECT status, assessment_id FROM burn_check_current_status WHERE check_id = ?1 AND project_key = ?2 ORDER BY branch_key",
    ).unwrap().query_map(rusqlite::params![definition.check_id, definition.project_key], |row| {
        Ok((row.get(0)?, row.get(1)?))
    }).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn transcript(session_id: &str, count: usize) -> String {
    let mut records = vec![
        json!({"type":"session","version":3,"id":session_id,"timestamp":"2026-01-01T00:00:00Z","cwd":"/synthetic"}),
        json!({"type":"message","id":"task","parentId":null,"timestamp":"2026-01-01T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"Complete the parser tests. Report any remaining failure."}]}}),
    ];
    let mut parent = "task".to_owned();
    for index in 0..count {
        let call = format!("call-{index}");
        let result = format!("result-{index}");
        records.push(json!({"type":"message","id":call,"parentId":parent,"timestamp":"2026-01-01T00:00:02Z","message":{"role":"assistant","content":[{"type":"toolCall","id":call,"name":"bash","arguments":{"command":"cargo test parser"}}]}}));
        records.push(json!({"type":"message","id":result,"parentId":call,"timestamp":"2026-01-01T00:00:03Z","message":{"role":"toolResult","toolCallId":call,"toolName":"bash","content":[{"type":"text","text":"Parser test failed: expected two records, received one."}],"isError":true}}));
        parent = result;
    }
    records
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[tokio::test]
async fn actual_yaml_parser_publication_engine_and_report_survive_restart_without_backfill() {
    let fixture = Fixture::new();
    fixture.publish("main", 3);
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    let scope = host.projects.keys().next().unwrap().clone();
    let active = host.projects[&scope].active[0].clone();
    assert_eq!(active.id.generation, Generation(1));
    assert_eq!(active.bootstrap_publication.0, 4);
    assert_eq!(host.catch_up().unwrap(), (false, false));
    assert!(
        host.evaluation_jobs(1)
            .pop()
            .unwrap()
            .run()
            .await
            .unwrap()
            .is_none()
    );
    fixture.evaluator.answer("stuck");
    fixture.publish("main", 6);
    assert_eq!(host.catch_up().unwrap(), (true, false));
    let result = host
        .evaluation_jobs(1)
        .pop()
        .unwrap()
        .run()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.outcome, RunOutcome::Accepted);
    let input = fixture.evaluator.inputs.lock().unwrap()[0].clone();
    assert_eq!(input["activity"].as_array().unwrap().len(), 3);
    assert_eq!(input["requests"][0]["kind"], "user_message");
    assert_eq!(
        input["requests"][0]["text"],
        "Complete the parser tests. Report any remaining failure."
    );
    assert_eq!(input["request_history_complete"], true);
    assert_eq!(input["source_coverage"], "complete");
    assert_eq!(input["activity"][0]["tool"]["result"]["status"], "error");
    let reports = read_installed_checks(&fixture.store, &host.definitions).unwrap();
    assert_eq!(reports.len(), 1);
    let writes = status_writes(&fixture.store, &host.definitions[0]);
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, "failure");
    drop(host);
    let mut restarted = fixture.host();
    restarted.reconcile(&Admission::Ready).unwrap();
    assert_eq!(restarted.projects[&scope].active[0], active);
    assert_eq!(restarted.catch_up().unwrap(), (false, false));
    assert!(
        restarted
            .evaluation_jobs(1)
            .pop()
            .unwrap()
            .run()
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.evaluator.inputs.lock().unwrap().len(), 1);
    assert_eq!(
        status_writes(&fixture.store, &restarted.definitions[0]),
        writes
    );
}

#[tokio::test]
async fn active_sessions_run_without_waiting_for_historical_backlog() {
    let fixture = Fixture::new();
    fixture.index("native", "historical", 3);
    fixture
        .store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = 1000 WHERE session_id = 'historical'",
            [],
        )
        .unwrap();
    let mut host = fixture.host();
    host.live_since_epoch = time::OffsetDateTime::now_utc().unix_timestamp() - 600;
    host.reconcile(&Admission::Ready).unwrap();
    assert!(matches!(
        host.definitions[0].availability,
        InstalledCheckAvailability::Available
    ));
    fixture.evaluator.answer("clear");
    fixture.publish("main", 3);
    fixture
        .store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = ?1 WHERE session_id = 'main'",
            [time::OffsetDateTime::now_utc().unix_timestamp() - 300],
        )
        .unwrap();
    host.catch_up().unwrap();
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
    fixture.publish("historical", 3);
    fixture
        .store
        .lock()
        .execute(
            "UPDATE session SET updated_at_epoch = 1000 WHERE session_id = 'historical'",
            [],
        )
        .unwrap();
    host.catch_up().unwrap();
    assert!(
        host.evaluation_jobs(1)
            .pop()
            .unwrap()
            .run()
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.evaluator.inputs.lock().unwrap().len(), 1);
    assert_eq!(
        status_writes(&fixture.store, &host.definitions[0])[0].0,
        "success"
    );
}

#[tokio::test]
async fn first_session_snapshot_does_not_trigger_historical_results() {
    let fixture = Fixture::new();
    fixture.index("native", "unpublished", 3);
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    fixture.publish("unpublished", 3);
    host.catch_up().unwrap();
    assert!(
        host.evaluation_jobs(1)
            .pop()
            .unwrap()
            .run()
            .await
            .unwrap()
            .is_none()
    );
    fixture.evaluator.answer("clear");
    fixture.publish("unpublished", 6);
    host.catch_up().unwrap();
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
        fixture.evaluator.inputs.lock().unwrap()[0]["activity"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn saved_answer_recovers_after_source_catchup_without_resend() {
    let fixture = Fixture::new();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    fixture.publish("main", 3);
    host.catch_up().unwrap();
    fixture.evaluator.answer("stuck");
    fixture.evaluator.wait.store(true, Ordering::Release);
    let task = tokio::spawn(host.evaluation_jobs(1).pop().unwrap().run());
    fixture.evaluator.started.notified().await;
    fixture.publish("main", 4);
    // The answer reaches Store before source catchup and must remain durable.
    fixture.evaluator.release.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap().unwrap().outcome,
        RunOutcome::Deferred
    );
    drop(host);
    let mut restarted = fixture.host();
    restarted.reconcile(&Admission::Ready).unwrap();
    assert!(status_writes(&fixture.store, &restarted.definitions[0]).is_empty());
    restarted.catch_up().unwrap();
    assert_eq!(
        status_writes(&fixture.store, &restarted.definitions[0])[0].0,
        "failure"
    );
    assert_eq!(fixture.evaluator.inputs.lock().unwrap().len(), 1);

    fixture.evaluator.wait.store(false, Ordering::Release);
    // The newer pending candidate remains subject to the persisted live cooldown.
    assert!(
        restarted
            .evaluation_jobs(1)
            .pop()
            .unwrap()
            .run()
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn source_catchup_and_lifecycle_reconciliation_continue_during_evaluation() {
    let fixture = Fixture::new();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    fixture.publish("main", 3);
    host.catch_up().unwrap();
    fixture.evaluator.answer("stuck");
    fixture.evaluator.wait.store(true, Ordering::Release);
    let task = tokio::spawn(host.evaluation_jobs(1).pop().unwrap().run());
    fixture.evaluator.started.notified().await;
    fixture.publish("second", 3);
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.catch_up().unwrap(), (true, false));
    assert!(!task.is_finished());
    fixture.evaluator.release.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap().unwrap().outcome,
        RunOutcome::Accepted
    );
    assert_eq!(
        status_writes(&fixture.store, &host.definitions[0])[0].0,
        "failure"
    );
}

#[tokio::test]
async fn project_override_removal_and_recreation_reset_bootstrap() {
    let fixture = Fixture::new();
    let path = fixture
        .project
        .join(".antiburn/checks/failed-retry-loop.yml");
    std::fs::write(
        &path,
        CHECK.replace("name: Failed Retry Loop", "name: Project Retry Review"),
    )
    .unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.definitions[0].name, "Project Retry Review");
    assert_eq!(
        host.definitions[0].source.scope,
        InstalledCheckScope::Project
    );
    fixture.publish("main", 3);
    host.catch_up().unwrap();
    std::fs::remove_file(&path).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(
        host.definitions[0].source.scope,
        InstalledCheckScope::Global
    );
    assert_eq!(host.definitions[0].generation, Some(2));
    assert_eq!(host.catch_up().unwrap(), (false, false));
    std::fs::remove_file(fixture.global.join("failed-retry-loop.yml")).unwrap();
    drop(host);
    let mut restarted = fixture.host();
    restarted.reconcile(&Admission::Ready).unwrap();
    assert!(restarted.definitions.is_empty());
    assert!(matches!(
        restarted
            .projects
            .values()
            .next()
            .unwrap()
            .store
            .load_lifecycle("failed-retry-loop", None)
            .unwrap(),
        LifecycleState::Removed {
            generation: Generation(3)
        }
    ));
    std::fs::write(&path, CHECK).unwrap();
    restarted.reconcile(&Admission::Ready).unwrap();
    assert_eq!(restarted.definitions[0].generation, Some(4));
    assert_eq!(restarted.catch_up().unwrap(), (false, false));
}

#[tokio::test]
async fn abandoned_dispatch_stays_unknown_and_replay_never_enters_live_metadata() {
    let fixture = Fixture::new();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    fixture.publish("main", 3);
    host.catch_up().unwrap();
    let scope = host.projects.keys().next().unwrap().clone();
    let project = &host.projects[&scope];
    let RecoveryWork::Pending(candidate) = project
        .store
        .load_recovery(None, 1)
        .unwrap()
        .pop()
        .unwrap()
        .work
    else {
        panic!("pending candidate")
    };
    let CandidateClaim::Claimed(claim) =
        project.store.claim_due(&candidate, 100_000, false).unwrap()
    else {
        panic!("claim candidate")
    };
    project
        .store
        .record_dispatch_intent(&claim, 100_000, Duration::from_secs(60), 1)
        .unwrap();
    let replay_scope = ActivationScope {
        run: RunScope::Replay("local-diagnostic".into()),
        ..scope.clone()
    };
    let replay = StoreCheckStore::new(
        Arc::clone(&fixture.store),
        "replay-owner".into(),
        replay_scope.clone(),
    )
    .unwrap();
    let mut replay_activation = project.active[0].clone();
    replay_activation.id.scope = replay_scope;
    replay
        .activate(
            "failed-retry-loop",
            &LifecycleTransition::Activated(replay_activation.clone()),
            Some(&replay_activation),
        )
        .unwrap();
    drop(host);
    let mut restarted = fixture.host();
    restarted.reconcile(&Admission::Ready).unwrap();
    restarted.catch_up().unwrap();
    assert_eq!(restarted.definitions.len(), 1);
    assert!(
        restarted
            .projects
            .keys()
            .all(|scope| scope.run == RunScope::Live)
    );
    assert!(
        restarted
            .evaluation_jobs(1)
            .pop()
            .unwrap()
            .run()
            .await
            .unwrap()
            .is_none()
    );
    assert!(fixture.evaluator.inputs.lock().unwrap().is_empty());
    assert!(status_writes(&fixture.store, &restarted.definitions[0]).is_empty());
    let detail: String = fixture
        .store
        .lock()
        .query_row(
            "SELECT actions_json FROM burn_check_assessment_history WHERE project_key = ?1 LIMIT 1",
            [&restarted.definitions[0].project_key],
            |row| row.get(0),
        )
        .unwrap();
    assert!(detail.contains("unknown"));
}

#[test]
fn invalid_unsupported_and_unbound_winners_remain_visible() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture
            .project
            .join(".antiburn/checks/failed-retry-loop.yml"),
        "name: Broken\non: {turn_end: null}\nactions: []\n",
    )
    .unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert!(matches!(
        host.definitions[0].availability,
        InstalledCheckAvailability::Unsupported { .. }
    ));
    assert!(host.evaluation_jobs(1).is_empty());
    assert!(host.definitions[0].diagnostic.is_some());
    std::fs::write(
        fixture
            .project
            .join(".antiburn/checks/failed-retry-loop.yml"),
        "name: [\n",
    )
    .unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    assert!(matches!(
        host.definitions[0].availability,
        InstalledCheckAvailability::Invalid { .. }
    ));
    fixture
        .store
        .lock()
        .execute("DELETE FROM session", [])
        .unwrap();
    // Persisted live activations still reconcile even when sessions disappear.
    host.reconcile(&Admission::Paused("Smart Checks paused".into()))
        .unwrap();
    assert_eq!(host.definitions.len(), 1);
}

#[tokio::test]
async fn activations_and_statuses_are_isolated_by_environment_and_project() {
    let fixture = Fixture::new();
    fixture.publish_in("ssh:test", "remote", 0);
    std::fs::write(
        fixture
            .project
            .join(".antiburn/checks/failed-retry-loop.yml"),
        CHECK.replace("name: Failed Retry Loop", "name: Project Retry Review"),
    )
    .unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.definitions.len(), 2);
    assert_eq!(host.definitions[0].name, "Project Retry Review");
    assert_eq!(host.definitions[1].name, "Failed Retry Loop");
    assert_ne!(
        host.definitions[0].effective_id,
        host.definitions[1].effective_id
    );
    fixture.publish("main", 3);
    host.catch_up().unwrap();
    fixture.evaluator.answer("stuck");
    for job in host.evaluation_jobs(2) {
        job.run().await.unwrap();
    }
    fixture.publish_in("ssh:test", "remote", 3);
    host.catch_up().unwrap();
    fixture.evaluator.answer("clear");
    for job in host.evaluation_jobs(2) {
        job.run().await.unwrap();
    }
    assert_eq!(
        status_writes(&fixture.store, &host.definitions[0])[0].0,
        "failure"
    );
    assert_eq!(
        status_writes(&fixture.store, &host.definitions[1])[0].0,
        "success"
    );
    assert_eq!(fixture.evaluator.inputs.lock().unwrap().len(), 2);
}

#[test]
fn global_checks_are_visible_before_a_project_is_indexed() {
    let fixture = Fixture::new();
    fixture
        .store
        .lock()
        .execute("DELETE FROM session", [])
        .unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.definitions.len(), 1);
    assert_eq!(host.definitions[0].name, "Failed Retry Loop");
    assert_eq!(host.definitions[0].generation, None);
    assert!(matches!(
        host.definitions[0].availability,
        InstalledCheckAvailability::Paused { .. }
    ));
    assert!(host.evaluation_jobs(1).is_empty());
    let report = read_installed_checks(&fixture.store, &host.definitions).unwrap();
    assert_eq!(report[0].finding, 0);
    assert_eq!(report[0].clean, 0);
    assert!(report[0].targets.is_empty());
}

#[tokio::test]
async fn unreadable_definition_revokes_an_in_flight_answer_and_other_checks_continue() {
    let fixture = Fixture::new();
    std::fs::write(fixture.global.join("healthy.yml"), CHECK).unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    fixture.publish("main", 3);
    host.catch_up().unwrap();
    fixture.evaluator.answer("stuck");
    fixture.evaluator.wait.store(true, Ordering::Release);
    let task = tokio::spawn(host.evaluation_jobs(1).pop().unwrap().run());
    fixture.evaluator.started.notified().await;

    std::fs::write(fixture.global.join("failed-retry-loop.yml"), [0xff, 0xfe]).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    let invalid = host
        .definitions
        .iter()
        .find(|definition| definition.check_id == "failed-retry-loop")
        .unwrap()
        .clone();
    assert!(matches!(
        invalid.availability,
        InstalledCheckAvailability::Invalid { .. }
    ));
    assert_eq!(invalid.generation, Some(2));
    let jobs = host.evaluation_jobs(1);
    assert_eq!(jobs[0].activation.id.definition_id, "healthy");
    fixture.evaluator.release.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap().unwrap().outcome,
        RunOutcome::Stale
    );
    fixture.evaluator.wait.store(false, Ordering::Release);
    fixture.evaluator.answer("stuck");
    assert_eq!(
        jobs.into_iter()
            .next()
            .unwrap()
            .run()
            .await
            .unwrap()
            .unwrap()
            .outcome,
        RunOutcome::Accepted
    );
    assert!(status_writes(&fixture.store, &invalid).is_empty());
    assert!(
        !status_writes(
            &fixture.store,
            host.definitions
                .iter()
                .find(|definition| definition.check_id == "healthy")
                .unwrap()
        )
        .is_empty()
    );
}

#[test]
fn oversized_and_unavailable_project_definitions_do_not_fall_back_or_block_other_projects() {
    let fixture = Fixture::new();
    let other = fixture.directory.path().join("other");
    std::fs::create_dir(&other).unwrap();
    fixture.publish("other", 0);
    fixture
        .store
        .lock()
        .execute(
            "UPDATE session SET cwd = ?1 WHERE session_id = 'other'",
            [other.to_string_lossy().as_ref()],
        )
        .unwrap();
    fixture
        .store
        .replace_repositories(&[repository(&fixture.project), repository(&other)])
        .unwrap();
    let path = fixture
        .project
        .join(".antiburn/checks/failed-retry-loop.yml");
    std::fs::write(&path, CHECK).unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.definitions.len(), 2);
    std::fs::write(&path, vec![b'x'; MAX_FILE_BYTES as usize + 1]).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    assert!(host.definitions.iter().any(|definition| matches!(
        definition.availability,
        InstalledCheckAvailability::Invalid { .. }
    )
        && definition.diagnostic.as_ref().unwrap().contains("256 KiB")));
    assert!(
        host.definitions
            .iter()
            .any(|definition| definition.source.scope == InstalledCheckScope::Global)
    );
    std::fs::remove_dir_all(fixture.project.join(".antiburn/checks")).unwrap();
    std::fs::write(fixture.project.join(".antiburn/checks"), "not a directory").unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(
        host.evaluation_jobs(1)[0].activation.id.scope.project,
        other.to_string_lossy()
    );
    assert!(host.definitions.iter().any(|definition| matches!(
        definition.availability,
        InstalledCheckAvailability::Invalid { .. }
    )));
    assert!(
        host.definitions
            .iter()
            .any(|definition| definition.source.scope == InstalledCheckScope::Global)
    );
    let mut restarted = fixture.host();
    restarted.reconcile(&Admission::Ready).unwrap();
    assert_eq!(
        restarted.evaluation_jobs(1)[0].activation.id.scope.project,
        other.to_string_lossy()
    );
}

#[test]
fn repository_root_definitions_bind_to_exact_subdirectory_evidence_scopes() {
    let fixture = Fixture::new();
    let subdirectory = fixture.project.join("packages/api");
    fixture
        .store
        .lock()
        .execute(
            "UPDATE session SET cwd = ?1",
            [subdirectory.to_string_lossy().as_ref()],
        )
        .unwrap();
    std::fs::create_dir_all(subdirectory.join(".antiburn/checks")).unwrap();
    std::fs::write(subdirectory.join(".antiburn/checks/wrong.yml"), CHECK).unwrap();
    let definition = fixture
        .project
        .join(".antiburn/checks/failed-retry-loop.yml");
    std::fs::write(
        &definition,
        CHECK.replace("name: Failed Retry Loop", "name: Root Definition"),
    )
    .unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.definitions.len(), 1);
    assert_eq!(host.definitions[0].name, "Root Definition");
    assert_eq!(
        host.definitions[0].source.path.as_deref(),
        definition.to_str()
    );
    assert_eq!(
        host.projects.keys().next().unwrap().project,
        subdirectory.to_string_lossy()
    );
    assert_eq!(
        host.projects.values().next().unwrap().active[0]
            .project_root
            .as_deref(),
        Some(subdirectory.as_path())
    );
    assert!(
        host.watch_directories
            .contains(&fixture.project.join(".antiburn/checks"))
    );
    assert!(
        !host
            .watch_directories
            .contains(&subdirectory.join(".antiburn/checks"))
    );
}

#[test]
fn known_repository_without_sessions_is_visible_without_activation_or_directory_creation() {
    let fixture = Fixture::new();
    fixture
        .store
        .lock()
        .execute("DELETE FROM session", [])
        .unwrap();
    std::fs::remove_dir_all(fixture.project.join(".antiburn")).unwrap();
    std::fs::remove_dir_all(&fixture.global).unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert!(!fixture.project.join(".antiburn").exists());
    assert!(!fixture.global.exists());
    assert!(host.definitions.is_empty());
    assert!(host.projects.is_empty());
    std::fs::create_dir_all(fixture.project.join(".antiburn/checks")).unwrap();
    std::fs::write(fixture.project.join(".antiburn/checks/review.yml"), CHECK).unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.definitions.len(), 1);
    assert_eq!(host.definitions[0].generation, None);
    assert!(matches!(
        host.definitions[0].availability,
        InstalledCheckAvailability::Paused { .. }
    ));
    assert!(host.evaluation_jobs(1).is_empty());
    assert!(host.projects.is_empty());
    let report = read_installed_checks(&fixture.store, &host.definitions).unwrap();
    assert_eq!(report[0].finding, 0);
    assert_eq!(report[0].clean, 0);
    assert!(report[0].targets.is_empty());
}

#[cfg(unix)]
#[test]
fn permission_denied_file_is_invalid_while_its_neighbor_stays_available() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let path = fixture.global.join("unreadable.yml");
    std::fs::write(&path, CHECK).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0)).unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert!(matches!(
        host.definitions
            .iter()
            .find(|definition| definition.check_id == "unreadable")
            .unwrap()
            .availability,
        InstalledCheckAvailability::Invalid { .. }
    ));
    assert!(matches!(
        host.definitions
            .iter()
            .find(|definition| definition.check_id == "failed-retry-loop")
            .unwrap()
            .availability,
        InstalledCheckAvailability::Available
    ));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn watcher_plan_and_event_filter_cover_only_the_installation_chain_and_yaml_children() {
    use notify::{
        Event, EventKind,
        event::{AccessKind, ModifyKind, RenameMode},
    };
    let directory = tempfile::tempdir().unwrap();
    let checks = directory.path().join(".antiburn/checks");
    let directories = vec![checks.clone()];
    assert_eq!(
        watch::watch_paths(&directories),
        BTreeSet::from([directory.path().to_path_buf()])
    );
    assert!(!checks.parent().unwrap().exists());
    let event = |path: PathBuf| Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path);
    assert!(watch::is_relevant(
        &event(directory.path().join(".antiburn")),
        &directories
    ));
    assert!(watch::is_relevant(
        &event(checks.join("review.yml")),
        &directories
    ));
    assert!(!watch::is_relevant(
        &event(directory.path().join("source.rs")),
        &directories
    ));
    assert!(!watch::is_relevant(
        &event(checks.join(".review.tmp")),
        &directories
    ));
    assert!(!watch::is_relevant(
        &event(checks.join("nested/review.yml")),
        &directories
    ));
    assert!(!watch::is_relevant(
        &Event::new(EventKind::Access(AccessKind::Read)).add_path(checks.join("review.yml")),
        &directories
    ));
    let rename = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
        .add_path(checks.join(".review.tmp"))
        .add_path(checks.join("review.yaml"));
    assert!(watch::is_relevant(&rename, &directories));
}

#[tokio::test]
async fn native_watcher_reconciles_created_replaced_and_recreated_global_and_repo_directories() {
    let fixture = Fixture::new();
    let home = fixture.project.parent().unwrap().join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::remove_dir_all(fixture.project.join(".antiburn")).unwrap();
    let global = home.join(".antiburn/checks");
    let mut host = InstalledCheckHost::new(
        Arc::clone(&fixture.store),
        Arc::clone(&fixture.evaluator),
        Some(global.clone()),
    )
    .unwrap();
    host.reconcile(&Admission::Ready).unwrap();
    let changed = Arc::new(Notify::new());
    let signal = Arc::clone(&changed);
    let mut watcher = watch::DirectoryWatcher::new(move || signal.notify_one()).unwrap();
    watcher.refresh(&host.watch_directories);
    assert!(!global.parent().unwrap().exists());
    assert!(!fixture.project.join(".antiburn").exists());
    for directory in [global, fixture.project.join(".antiburn/checks")] {
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("review.yml");
        std::fs::write(&path, CHECK).unwrap();
        watch_reconcile(&changed, &mut watcher, &mut host).await;
        assert!(
            host.definitions
                .iter()
                .any(|definition| definition.source.path.as_deref() == path.to_str())
        );

        let temporary = directory.join(".save.tmp");
        std::fs::write(
            &temporary,
            CHECK.replace("name: Failed Retry Loop", "name: Atomic Save"),
        )
        .unwrap();
        std::fs::rename(&temporary, &path).unwrap();
        watch_reconcile(&changed, &mut watcher, &mut host).await;
        assert!(
            host.definitions
                .iter()
                .any(|definition| definition.name == "Atomic Save")
        );

        let replacement = directory.with_file_name("replacement");
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(replacement.join("review.yml"), CHECK).unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        std::fs::rename(&replacement, &directory).unwrap();
        watch_reconcile(&changed, &mut watcher, &mut host).await;
        assert!(
            host.definitions
                .iter()
                .any(
                    |definition| definition.source.path.as_deref() == path.to_str()
                        && definition.name == "Failed Retry Loop"
                )
        );

        std::fs::remove_dir_all(directory.parent().unwrap()).unwrap();
        watch_reconcile(&changed, &mut watcher, &mut host).await;
        assert!(
            !host
                .definitions
                .iter()
                .any(|definition| definition.source.path.as_deref() == path.to_str())
        );
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, CHECK).unwrap();
        watch_reconcile(&changed, &mut watcher, &mut host).await;
        assert!(
            host.definitions
                .iter()
                .any(|definition| definition.source.path.as_deref() == path.to_str())
        );
    }
}

async fn watch_reconcile(
    changed: &Notify,
    watcher: &mut watch::DirectoryWatcher,
    host: &mut InstalledCheckHost<ScriptedEvaluator>,
) {
    tokio::time::timeout(Duration::from_secs(10), changed.notified())
        .await
        .expect("native watch change");
    watcher.refresh(&host.watch_directories);
    host.reconcile(&Admission::Ready).unwrap();
    while tokio::time::timeout(Duration::from_millis(200), changed.notified())
        .await
        .is_ok()
    {
        watcher.refresh(&host.watch_directories);
        host.reconcile(&Admission::Ready).unwrap();
    }
}

#[derive(Clone, Default)]
struct LogCapture(Arc<Mutex<Vec<String>>>);

impl tracing::Subscriber for LogCapture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                write!(&mut self.0, " {}={value:?}", field.name()).unwrap();
            }
        }
        let mut fields = Fields(event.metadata().level().to_string());
        event.record(&mut fields);
        if fields.0.contains("installed_check_invalid") {
            self.0.lock().unwrap().push(fields.0);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn definition_diagnostics_are_error_level_located_safe_and_deduplicated_until_recovery() {
    let fixture = Fixture::new();
    for (name, content) in [
        ("syntax.yml", "name: [\n".into()),
        ("scalar.yml", "name: !!bool PAYLOAD_CANARY\n".into()),
        (
            "duplicate.yml",
            "PAYLOAD_CANARY: a\nPAYLOAD_CANARY: b\n".into(),
        ),
        ("schema.yml", format!("{CHECK}\nPAYLOAD_CANARY: true\n")),
        (
            "expression.yml",
            format!("{CHECK}\nif: message.PAYLOAD_CANARY\n"),
        ),
    ] {
        std::fs::write(fixture.global.join(name), content).unwrap();
    }
    let captured = LogCapture::default();
    let mut host = fixture.host();
    tracing::subscriber::with_default(captured.clone(), || {
        host.reconcile(&Admission::Ready).unwrap();
        let count = captured.0.lock().unwrap().len();
        assert!(count >= 5);
        host.reconcile(&Admission::Ready).unwrap();
        assert_eq!(captured.0.lock().unwrap().len(), count);
        std::fs::write(fixture.global.join("syntax.yml"), CHECK).unwrap();
        host.reconcile(&Admission::Ready).unwrap();
        std::fs::write(fixture.global.join("syntax.yml"), "name: [\n").unwrap();
        host.reconcile(&Admission::Ready).unwrap();
        assert_eq!(captured.0.lock().unwrap().len(), count + 1);
    });
    let logs = captured.0.lock().unwrap();
    assert!(logs.iter().all(|log| log.starts_with("ERROR")
        && log.contains("path=")
        && log.contains("code=")
        && log.contains("line=")
        && log.contains("column=")
        && log.contains("config_location=")));
    assert!(logs.iter().any(|log| log.contains("yaml-syntax")));
    assert!(logs.iter().any(|log| log.contains("config-unknown-field")));
    assert!(logs.iter().all(|log| !log.contains("PAYLOAD_CANARY")));
    assert!(
        host.definitions
            .iter()
            .filter_map(|definition| definition.diagnostic.as_deref())
            .all(|diagnostic| !diagnostic.contains("PAYLOAD_CANARY"))
    );
}

#[test]
fn disabled_admission_still_logs_invalid_yaml() {
    let fixture = Fixture::new();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    std::fs::write(fixture.global.join("failed-retry-loop.yml"), "name: [\n").unwrap();
    let captured = LogCapture::default();
    tracing::subscriber::with_default(captured.clone(), || {
        host.reconcile(&Admission::Paused("Smart Burn Checks are disabled.".into()))
            .unwrap()
    });
    assert!(host.evaluation_jobs(1).is_empty());
    assert!(
        host.definitions[0]
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("yaml-syntax")
    );
    assert!(
        captured
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|log| log.contains("yaml-syntax"))
    );
}

#[cfg(target_os = "macos")]
#[test]
fn protected_repository_without_grant_is_not_read_or_watched() {
    let fixture = Fixture::new();
    let root = antiburn_local::paths::home_dir()
        .unwrap()
        .join("Documents/antiburn-check-no-access");
    fixture
        .store
        .replace_repositories(&[repository(&root)])
        .unwrap();
    fixture
        .store
        .lock()
        .execute(
            "UPDATE session SET cwd = ?1",
            [root.to_string_lossy().as_ref()],
        )
        .unwrap();
    let mut host = fixture.host();
    host.reconcile(&Admission::Ready).unwrap();
    assert_eq!(host.watch_directories, vec![fixture.global.clone()]);
    assert!(host.evaluation_jobs(1).is_empty());
    assert!(
        host.definitions[0]
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("Folder access is not granted")
    );
}
