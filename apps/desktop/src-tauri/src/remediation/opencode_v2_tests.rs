use antiburn_local::analysis::{
    EvidenceSource, EvidenceValue, METRICS_SCHEMA_REVISION, ModelControlObservation,
    PARSER_REVISION, SessionEvidence, SessionEvidenceAccumulator, SourceCapabilities, SourceKind,
    TurnCounts, TurnFacts,
};
use antiburn_local::insights::ReportWindow;

use super::*;
use crate::store::{AnalysisRecord, EvidenceCompletion, PublishedEvidence, SessionRecord};

const V2_SOURCES: [SourceFormat; 2] = [
    SourceFormat::OpenCodeSqliteV2,
    SourceFormat::OpenCodeV2SessionExport,
];
const MODEL_USAGE_AT_MS: i64 = 2_000_000_000_000;
const SESSION_STARTED_AT_EPOCH: i64 = MODEL_USAGE_AT_MS / 1_000;

fn model_evidence(source: SourceFormat) -> SessionEvidence {
    let mut capabilities = SourceCapabilities::opencode();
    capabilities.source_format = source;
    let mut evidence = SessionEvidenceAccumulator::new(EvidenceSource {
        agent: "opencode".into(),
        session_id: "session".into(),
        kind: SourceKind::File,
        capabilities,
    })
    .evidence(&TurnFacts::default());
    let EvidenceValue::Complete(eligibility) = &mut evidence.eligibility else {
        panic!("the fixture needs complete eligibility");
    };
    eligibility.assistant_turns = 1;
    let EvidenceValue::Complete(models) = &mut evidence.models else {
        panic!("the fixture needs complete model evidence");
    };
    models.by_model.insert(
        "gpt-5.5".into(),
        antiburn_local::analysis::ModelTokens {
            input: 10,
            turns: 1,
            first_ts_ms: MODEL_USAGE_AT_MS,
            last_ts_ms: MODEL_USAGE_AT_MS,
            ..Default::default()
        },
    );
    models.control_observations = vec![ModelControlObservation {
        provider: Some("openai".into()),
        api: Some("responses".into()),
        model: "gpt-5.5".into(),
        effort: None,
        speed: None,
        last_ts_ms: MODEL_USAGE_AT_MS,
        turns: TurnCounts {
            main_loop: 1,
            delegated: 0,
        },
    }];
    evidence
}

fn publish_v1_model(store: &Store) -> SessionKey {
    let key = SessionKey::new("native", "opencode", "session");
    store
        .upsert_sessions(
            &[SessionRecord {
                key: key.clone(),
                source_kind: "file".into(),
                source_label: "synthetic".into(),
                wsl_distro: None,
                title: None,
                title_source: None,
                cwd: None,
                surface: "cli".into(),
                updated_at_epoch: Some(SESSION_STARTED_AT_EPOCH),
                activity_cursor: "cursor".into(),
                activity_source: "event".into(),
                subagent_count: 0,
                fork_parent_session_id: None,
                source_fingerprint: Some("fingerprint".into()),
            }],
            &["opencode"],
        )
        .unwrap();
    let claim = store
        .claim_next_evidence(&["opencode"], 10, 60)
        .unwrap()
        .unwrap();
    assert!(
        store
            .publish_projections(
                &AnalysisRecord {
                    key: key.clone(),
                    model_breakdown_json: "{}".into(),
                    pricing_breakdown_json: "{}".into(),
                    inclusive_models_json: "[]".into(),
                    initial_context_json: None,
                    source_summaries_json: None,
                    provider_hints_json: None,
                    source_fingerprint: "fingerprint".into(),
                    pricing_generation: 1,
                    analyzed_generation: claim.source_generation,
                    parser_revision: PARSER_REVISION,
                    analyzer_revision: ANALYZER_REVISION,
                    metrics_schema_revision: METRICS_SCHEMA_REVISION,
                },
                Some(SESSION_STARTED_AT_EPOCH),
                &EvidenceCompletion {
                    claim_fence: claim.claim_fence,
                    status: PublishedEvidence::Ready,
                    evidence_schema_revision: EVIDENCE_SCHEMA_REVISION,
                    evidence_json: serde_json::to_string(&model_evidence(
                        SourceFormat::OpenCodeSqliteV1
                    ))
                    .unwrap(),
                },
                &[],
                &[]
            )
            .unwrap()
    );
    key
}

#[test]
fn v2_matching_model_use_does_not_prove_current_config_attribution() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let config = home.join(".config/opencode/opencode.json");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, r#"{"model":"openai/gpt-5.5"}"#).unwrap();
    let store = Store::open(&temporary.path().join("store")).unwrap();
    let key = publish_v1_model(&store);
    for source in [SourceFormat::OpenCodeSqliteV1, V2_SOURCES[0], V2_SOURCES[1]] {
        let attribution = publication_config_attribution_with_home(
            &store,
            &key,
            PublishedEvidence::Ready,
            &serde_json::to_string(&model_evidence(source)).unwrap(),
            &home,
        )
        .unwrap();
        assert_eq!(
            attribution.model.is_some(),
            source == SourceFormat::OpenCodeSqliteV1
        );
        assert_eq!(
            attribution.records.len(),
            usize::from(source == SourceFormat::OpenCodeSqliteV1)
        );
        assert!(attribution.reasoning.is_none());
    }
}

#[test]
fn retained_v2_findings_cannot_reuse_v1_physical_targets_or_watches() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("home");
    let config = home.join(".config/opencode/opencode.json");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, r#"{"model":"openai/gpt-5.5"}"#).unwrap();
    let store = Store::open(temporary.path()).unwrap();
    let key = publish_v1_model(&store);
    let attribution = publication_config_attribution_with_home(
        &store,
        &key,
        PublishedEvidence::Ready,
        &serde_json::to_string(&model_evidence(SourceFormat::OpenCodeSqliteV1)).unwrap(),
        &home,
    )
    .unwrap()
    .model
    .unwrap();
    store.lock().execute(
        "UPDATE session_evidence SET effective_model_target_hash = ?1, effective_model_scope = ?2, effective_model = ?3",
        rusqlite::params![attribution.0, attribution.1, attribution.2],
    ).unwrap();
    let current = insights_report::list_current_findings(
        temporary.path(),
        CurrentFindingsRequest {
            environment_key: "native".into(),
            window: ReportWindow {
                start_epoch: SESSION_STARTED_AT_EPOCH - 1,
                end_epoch: SESSION_STARTED_AT_EPOCH + 10,
            },
            detector: DetectorId::OldModelUsage,
        },
    )
    .unwrap()
    .findings
    .into_iter()
    .next()
    .expect("the V1 fixture must produce a finding");
    assert_eq!(
        current.finding.source_format,
        SourceFormat::OpenCodeSqliteV1
    );
    let secret = store.provider_account_secret().unwrap();
    let v1_identity = target_identity(&secret, &current, None);
    assert_eq!(
        v1_identity.physical_target_key.as_deref(),
        Some(attribution.0.as_str())
    );
    let controller = RemediationController::new(temporary.path().into());
    for source in V2_SOURCES {
        let mut retained = current.clone();
        retained.finding.source_format = source;
        assert!(remediation_prompt(&retained.finding).is_err());
        let identity = target_identity(&secret, &retained, None);
        assert!(identity.physical_target_key.is_none());
        assert_ne!(identity.target_key, v1_identity.target_key);
        let (_, resolved) = controller
            .resolve_target(&store, retained.clone(), AgentKind::OpenCode, Some(&home))
            .unwrap();
        assert!(resolved.config.is_none());
        let definition = watch_definition(&resolved);
        assert_eq!(definition.source_format.value(), source);
        assert!(!watch_verification_available(
            &definition,
            "global",
            "opencode",
            DetectorId::OldModelUsage
        ));
        assert!(
            passive_remediations(
                &store.lock(),
                &secret,
                vec![retained],
                MODEL_USAGE_AT_MS + 1_000
            )
            .unwrap()
            .is_empty()
        );
    }
}

#[test]
fn v2_static_checks_do_not_turn_missing_support_into_clean_results() {
    for source in V2_SOURCES {
        let evidence = model_evidence(source);
        for detector in [
            DetectorId::SessionsOverDepth,
            DetectorId::ModelOverthinking,
            DetectorId::OverpoweredSubagents,
            DetectorId::UnusedMcpServers,
            DetectorId::UnusedBuiltInTools,
            DetectorId::UnusedSkills,
            DetectorId::OldModelUsage,
            DetectorId::OveruseOfFastMode,
            DetectorId::CacheChurn,
        ] {
            assert!(
                matches!(
                    antiburn_local::remediation::assess_detector(
                        detector,
                        &evidence,
                        &antiburn_local::insights::ReportCatalogs::default()
                    ),
                    FindingAssessment::NotApplicable | FindingAssessment::Unavailable(_)
                ),
                "{source:?} {detector:?}"
            );
        }
    }
}

#[test]
fn v2_actions_stay_unavailable_on_every_edit_platform_and_environment() {
    for source in V2_SOURCES {
        for setting in [
            ConfigSetting::Model,
            ConfigSetting::Reasoning,
            ConfigSetting::Compaction,
            ConfigSetting::SubagentModel,
            ConfigSetting::McpServer,
            ConfigSetting::BuiltInTool,
            ConfigSetting::Skill,
            ConfigSetting::FastMode,
        ] {
            for action in [
                RemediationAction::AutomaticEdit(setting),
                RemediationAction::RecoverUncertainWrite(setting),
                RemediationAction::PublicationAttribution(setting),
            ] {
                assert_eq!(
                    vendor_policy(AgentKind::OpenCode)
                        .unwrap()
                        .action_support(action, source),
                    ActionSupport::Unsupported
                );
            }
            for platform in ["macos", "linux", "windows"] {
                for environment in ["native", "wsl:ubuntu", "ssh:host"] {
                    assert!(!automatic_editor_supported(
                        AgentKind::OpenCode,
                        setting,
                        source,
                        "global",
                        environment,
                        platform
                    ));
                }
            }
        }
    }
}

#[test]
fn v2_uncertain_writes_stay_blocked_instead_of_starting_verification() {
    for source in V2_SOURCES {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::open(temporary.path()).unwrap();
        let definition = json!({
            "version": 1,
            "detector": "unused_mcp_servers",
            "canonicalIdentity": "docs",
            "sourceFormat": source,
            "workspaceKey": null,
            "provider": null,
            "api": null,
            "oldModel": null,
            "replacement": null,
            "resource": "docs",
            "physicalTargetKey": "physical",
            "configSetting": "mcpServer",
            "configExpectedValue": "docs=true",
            "configProposedValue": "docs=false",
            "verificationMethodRevision": VERIFICATION_METHOD_REVISION,
            "remediationPolicyRevision": REMEDIATION_POLICY_REVISION,
            "savingsMethodRevision": SAVINGS_METHOD_REVISION,
            "pricingRevision": null,
            "oldPricing": null,
            "replacementPricing": null
        })
        .to_string();
        let record = store.create_or_reuse_remediation(&Remediation {
            remediation_id: "recovery".into(), target_key: "target".into(),
            environment_key: "native".into(), agent: "opencode".into(),
            scope_kind: "global".into(), scope_key: "physical".into(),
            state: RemediationState::Reserved, origin: "action".into(), prompt_group_id: None,
            definition_json: definition.clone(),
            result_json: json!({"version": 1, "verification": {"status": "watching"}, "savings": {"status": "unavailable"}}).to_string(),
            created_at_epoch: 1, effective_boundary_ms: None,
        }, &[]).unwrap().unwrap();
        assert!(
            store
                .begin_remediation_write(&record.remediation_id, 2)
                .unwrap()
        );
        assert!(
            store
                .mark_remediation_recovery_needed(&record.remediation_id, "readbackFailed", 2)
                .unwrap()
        );
        let record = store.remediation(&record.remediation_id).unwrap().unwrap();
        assert!(recover_uncertain_write(&store, &record, 3).unwrap());
        let after = store.remediation(&record.remediation_id).unwrap().unwrap();
        assert_eq!(after.definition_json, definition);
        assert_eq!(after.state, RemediationState::RecoveryNeeded);
        assert!(after.effective_boundary_ms.is_none());
        let result: serde_json::Value = serde_json::from_str(&after.result_json).unwrap();
        assert_eq!(result["verification"]["reason"], "verificationUnavailable");
    }
}
