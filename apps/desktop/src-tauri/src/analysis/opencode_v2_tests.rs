use super::*;
use crate::store::{
    EvidenceCompletion, FencedTurnRowStore, PublishedEvidence, RelationRecord, SessionRecord,
};
use rusqlite::{Connection, params};

fn database() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode-v2.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE session_v2 (
                 id TEXT PRIMARY KEY, parent_id TEXT, fork_session_id TEXT,
                 directory TEXT NOT NULL DEFAULT '/repo', title TEXT,
                 time_created INTEGER NOT NULL DEFAULT 1000,
                 time_updated INTEGER NOT NULL DEFAULT 4000
             );
             CREATE TABLE session_message (
                 id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
                 seq INTEGER NOT NULL, time_created INTEGER NOT NULL DEFAULT 1000,
                 time_updated INTEGER NOT NULL DEFAULT 4000, data TEXT NOT NULL
             );
             CREATE UNIQUE INDEX native_sequence ON session_message(session_id, seq);
             INSERT INTO session_v2 (id,parent_id) VALUES
                 ('ses_parent',NULL),('ses_child','ses_parent'),('ses_grandchild','ses_child');",
        )
        .unwrap();
    for (session_id, model, input, output, created) in [
        ("ses_parent", "claude-opus-4-6", 10, 2, 1000),
        ("ses_child", "claude-sonnet-4-6", 30, 5, 500),
        ("ses_grandchild", "claude-haiku-4-5", 40, 7, 3000),
    ] {
        let mut message = serde_json::json!({
            "agent":"build", "model":{"id":model,"providerID":"anthropic"},
            "time":{"created":created,"completed":created+500},
            "tokens":{"input":input,"output":output,"reasoning":0,"cache":{"read":0,"write":0}},
            "content":[{"type":"text","text":format!("Output from {session_id}")}]
        });
        if session_id == "ses_parent" {
            for (id, continued) in [("call-new", false), ("call-continued", true)] {
                let mut input = serde_json::json!({
                    "agent":"review","description":"Inspect code","prompt":"Review it","background":true
                });
                if continued {
                    input["sessionID"] = serde_json::json!("ses_child");
                }
                message["content"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({
                        "type":"tool","name":"subagent","id":id,
                        "time":{"created":1000,"ran":1100,"completed":1400},
                        "state":{
                            "status":"completed","input":input,
                            "metadata":{"sessionID":"ses_child","status":"running"},
                            "content":[{"type":"text","text":"Subagent is running in the background."}]
                        }
                    }));
            }
        }
        connection
            .execute(
                "INSERT INTO session_message (id,session_id,type,seq,data)
                 VALUES (?1,?2,'assistant',0,?3)",
                params![format!("msg_{session_id}"), session_id, message.to_string()],
            )
            .unwrap();
    }
    (directory, path)
}

#[tokio::test]
async fn opencode_v2_source_groups_publish_separate_costs_and_replay_child_drilldowns() {
    let (directory, path) = database();
    let (latest, rows) = Explorers::DISK
        .provider_db_fingerprint(&AgentKind::OpenCode, &path, "ses_parent")
        .await
        .unwrap();
    let fingerprint = format!("sv1:db:{latest}:{rows}");
    let store = Store::open_in_memory(directory.path()).unwrap();
    let key = SessionKey::new("native", "opencode", "ses_parent");
    let source = SessionSource::ProviderDb {
        agent: AgentKind::OpenCode,
        db_path: path,
        session_id: key.session_id.clone(),
        source_format: Some(SourceFormat::OpenCodeSqliteV2),
    };
    store
        .upsert_sessions(
            &[SessionRecord {
                key: key.clone(),
                source_kind: "providerDb".to_owned(),
                source_label: "opencode:ses_parent:open_code_v2_sqlite".to_owned(),
                wsl_distro: None,
                title: None,
                title_source: None,
                cwd: Some("/repo".to_owned()),
                surface: "cli".to_owned(),
                updated_at_epoch: Some(4),
                activity_cursor: String::new(),
                activity_source: "native".to_owned(),
                subagent_count: 0,
                fork_parent_session_id: None,
                source_fingerprint: Some(fingerprint.clone()),
            }],
            &crate::agents::evidence_cohort(),
        )
        .unwrap();
    let claim = store
        .claim_next_evidence(&["opencode"], 10, 60)
        .unwrap()
        .unwrap();
    let row_store = Arc::new(FencedTurnRowStore::new(
        store.clone(),
        key.clone(),
        claim.claim_fence,
    ));
    let pass = analyze_located_for_evidence(
        AgentKind::OpenCode,
        &key.session_id,
        ClaimedSource {
            fingerprint: Some(fingerprint),
            generation: claim.source_generation,
        },
        PassSignal::new(),
        Some(row_store),
        None,
        LocatedTranscripts {
            source,
            children: Vec::new(),
        },
    )
    .await;
    assert_eq!(pass.outcome, PassOutcome::Published);
    let live = &pass.analysis;
    assert_eq!(live.started_at_epoch, Some(1));
    assert_eq!(live.inclusive_tokens.as_ref().unwrap().input_tokens, 80);
    assert_eq!(live.subagents_tokens.as_ref().unwrap().input_tokens, 70);
    let parent_cost = live.top_level_cost.unwrap().total_usd;
    let child_cost = live.subagents_cost.unwrap().total_usd;
    assert!(parent_cost > 0.0);
    assert!(child_cost > 0.0);
    assert!((live.cost.unwrap().total_usd - parent_cost - child_cost).abs() < 1e-12);
    let members = &live.orchestration.as_ref().unwrap().members;
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].subagent_id, "ses_child");
    assert_eq!(members[0].tokens.as_ref().unwrap().input_tokens, 30);
    assert_eq!(members[1].tokens.as_ref().unwrap().input_tokens, 40);
    assert_eq!(live.source_summaries.as_ref().unwrap().len(), 3);
    let launches = |analysis: &SessionAnalysis| {
        analysis
            .metrics
            .as_ref()
            .unwrap()
            .buckets
            .iter()
            .map(|bucket| bucket.subagent_launches)
            .sum::<u32>()
    };
    assert_eq!(launches(live), 1);
    let relations = members
        .iter()
        .map(|member| RelationRecord {
            kind: RelationKind::Subagent,
            related_id: member.subagent_id.clone(),
            label: Some(member.label.clone()),
        })
        .collect::<Vec<_>>();
    let completion = EvidenceCompletion {
        claim_fence: claim.claim_fence,
        status: PublishedEvidence::Unsupported,
        evidence_schema_revision: EVIDENCE_SCHEMA_REVISION,
        evidence_json: serde_json::to_string(pass.evidence.as_ref().unwrap()).unwrap(),
    };
    assert!(
        store
            .publish_projections(
                &live.record(&key).unwrap(),
                live.started_at_epoch,
                &completion,
                &relations,
                &pass.source_outcomes,
            )
            .unwrap()
    );
    let rows = store.published_turn_rows(&key).unwrap().unwrap();
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert_eq!(row.source_key, row.thread_id);
        assert_eq!(row.turn_index, 0);
        assert_eq!(
            row.subagent_launches,
            u32::from(row.source_key == "ses_parent")
        );
        assert_eq!(
            row.scope,
            if row.source_key == "ses_parent" {
                TurnScope::Main
            } else {
                TurnScope::Delegated
            }
        );
    }
    let replay = analysis_from_rows(&store, &key, &key.session_id, AgentKind::OpenCode).unwrap();
    assert_eq!(replay.started_at_epoch, live.started_at_epoch);
    assert_eq!(replay.inclusive_tokens, live.inclusive_tokens);
    assert_eq!(replay.subagents_tokens, live.subagents_tokens);
    assert_eq!(replay.cost, live.cost);
    assert_eq!(replay.top_level_cost, live.top_level_cost);
    assert_eq!(replay.subagents_cost, live.subagents_cost);
    assert_eq!(replay.model_runs, live.model_runs);
    assert_eq!(launches(&replay), 1);
    for (id, input) in [("ses_child", 30), ("ses_grandchild", 40)] {
        let child =
            subagent_analysis_from_rows(&store, &key, &key.session_id, id, AgentKind::OpenCode)
                .unwrap();
        assert_eq!(child.inclusive_tokens.unwrap().input_tokens, input);
        assert_eq!(
            child.cost,
            members
                .iter()
                .find(|member| member.subagent_id == id)
                .unwrap()
                .cost
        );
    }
}
