use super::*;
use crate::jev::config::{
    ContextLimitOverride, CredentialReference, SystemOneConnection, SystemOneEndpoint,
    SystemOneProvider, SystemOneResponseMode,
};
use antiburn_checks::domain::{
    ActivationId, ActivationScope, BranchId, FrozenContext, Generation, ObservationPosition,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

fn activation() -> EffectiveActivation {
    activation_with_output_id("verdict")
}

fn activation_with_output_id(output_id: &str) -> EffectiveActivation {
    EffectiveActivation {
        id: ActivationId {
            scope: ActivationScope::default(),
            definition_id: "selected-provider".into(),
            generation: Generation(1),
        },
        revision: [1; 32],
        definition_scope: antiburn_checks::discovery::DefinitionScope::Global,
        source_paths: Vec::new(),
        project_root: None,
        compiled: Arc::new(
            antiburn_checks::compiler::compile(
                &r#"
name: Selected provider
on:
  tool_result:
  window: 2
decisions:
  review:
    input:
      rows: ${{ window.messages }}
    fit:
      rows: recent
    output:
      verdict:
        type: choice
        question: Does the evidence show a concern?
        criteria:
          concern: The evidence shows a concern.
          clear: The evidence does not show a concern.
actions:
  - type: status
    if: decisions.review.details.verdict.probabilities.concern > 0.9
    input:
      result: failure
"#
                .replace("verdict", output_id),
            )
            .unwrap(),
        ),
        bootstrap_observation_position: ObservationPosition(0),
        bootstrap_publication: antiburn_checks::source::PublicationCursor(0),
    }
}

fn candidate(activation: &EffectiveActivation) -> FrozenCandidate {
    FrozenCandidate {
        activation: activation.id.clone(),
        branch: BranchId("main".into()),
        trigger_position: ObservationPosition(3),
        messages: Vec::new(),
        context: FrozenContext {
            value: json!({}),
            dependencies: Vec::new(),
        },
        dependencies: Vec::new(),
        fingerprint: [7; 32],
    }
}

fn connection(endpoint: &str) -> SystemOneConnection {
    SystemOneConnection {
        provider: SystemOneProvider::Custom,
        endpoint: SystemOneEndpoint::ExactUrl(endpoint.into()),
        model: "synthetic-model".into(),
        model_revision: Some("synthetic-digest".into()),
        response_mode: SystemOneResponseMode::Direct,
        credential: Some(CredentialReference::Connection(
            "synthetic-reference".into(),
        )),
        revision: 7,
        context_override: Some(ContextLimitOverride {
            total_input_tokens: Some(16_384),
            ..Default::default()
        }),
    }
}

fn capabilities(connection: &SystemOneConnection) -> ModelCapabilities {
    ModelCapabilities {
        model: connection.model.clone(),
        model_revision: connection.model_revision.clone(),
        ..ModelCapabilities::jev_default()
    }
}

fn evaluator(connection: SystemOneConnection) -> TypeSafeDecisionEvaluator {
    let limits = capabilities(&connection);
    TypeSafeDecisionEvaluator::from_selected_snapshot(
        connection,
        Some("synthetic-secret".into()),
        limits,
    )
    .unwrap()
}

fn cloudflare_evaluator(provider: SystemOneProvider) -> TypeSafeDecisionEvaluator {
    let mut selected = connection("https://gateway.invalid/envelope");
    selected.provider = provider;
    selected.response_mode = SystemOneResponseMode::CloudflareEnvelope;
    if provider == SystemOneProvider::Cloudflare {
        selected.endpoint = SystemOneEndpoint::CloudflareAccount("synthetic-account".into());
        selected.model = "clef".into();
        selected.model_revision = None;
    }
    let limits = if provider == SystemOneProvider::Cloudflare {
        selected.capabilities().unwrap()
    } else {
        capabilities(&selected)
    };
    TypeSafeDecisionEvaluator::from_selected_snapshot(
        selected,
        Some("synthetic-secret".into()),
        limits,
    )
    .unwrap()
    .with_usage(Arc::new(|_, _, _| {
        panic!("preflight must not record usage")
    }))
    .with_diagnostics(Arc::new(|_| {
        panic!("preflight must not start a provider attempt")
    }))
}

#[test]
fn selected_adapter_preflight_enforces_cloudflare_id_boundary_without_changing_direct_modes() {
    for length in [100, 101] {
        let output_id = "a".repeat(length);
        let activation = activation_with_output_id(&output_id);
        for provider in [SystemOneProvider::Cloudflare, SystemOneProvider::Custom] {
            let evaluator = cloudflare_evaluator(provider);
            let result = evaluator.prepare(&activation, json!({"rows": ["fixed"]}));
            if length == 100 {
                let prepared = result.unwrap();
                evaluator.validate_prepared(&activation, &prepared).unwrap();
            } else {
                let failure = result.unwrap_err();
                assert_eq!(failure.code, PreparationFailureCode::InvalidRequest);
                assert_eq!(
                    failure.context.field,
                    Some(format!("decisions.review.output.{output_id}"))
                );
                assert!(failure.report.unwrap().omissions.is_empty());
            }
        }
        for evaluator in [
            evaluator(connection("https://gateway.invalid/direct")),
            TypeSafeDecisionEvaluator::new(
                crate::jev::client::TypeSafeClient::new("synthetic-secret".into()).unwrap(),
            ),
        ] {
            let prepared = evaluator
                .prepare(&activation, json!({"rows": ["fixed"]}))
                .unwrap();
            evaluator.validate_prepared(&activation, &prepared).unwrap();
        }
    }
}

#[tokio::test]
async fn cloudflare_adapter_errors_reject_saved_requests_before_any_attempt_or_usage() {
    let activation = activation_with_output_id(&"a".repeat(101));
    for provider in [SystemOneProvider::Cloudflare, SystemOneProvider::Custom] {
        let evaluator = cloudflare_evaluator(provider);
        let input = json!({"rows": ["fixed"]});
        let report = FitReport::default();
        let request = build_request(
            &activation.compiled.decision,
            input.clone(),
            &report,
            &evaluator.capabilities,
        );
        validate_jev_request_with_capabilities(&request, &evaluator.capabilities).unwrap();
        let prepared = PreparedDecision {
            profile: preparation_profile(
                &activation.compiled.decision,
                &evaluator.capabilities,
                &evaluator.descriptor,
            ),
            input: input.clone(),
            request: Some(serde_json::to_value(request).unwrap()),
            report,
        };
        assert_eq!(
            evaluator
                .validate_prepared(&activation, &prepared)
                .unwrap_err()
                .code,
            PreparationFailureCode::IncompatibleProfile
        );
        let failure = evaluator
            .evaluate_prepared(&activation, &candidate(&activation), &prepared, 1)
            .await
            .unwrap_err();
        assert_eq!(failure.code, EvaluationFailureCode::InvalidRequest);
        assert_eq!(failure.attempts, 0);
        let failure = evaluator
            .evaluate(&activation, &candidate(&activation), &input, 1)
            .await
            .unwrap_err();
        assert_eq!(failure.code, EvaluationFailureCode::InvalidRequest);
        assert_eq!(failure.attempts, 0);
    }
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                return socket;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "the mock request did not arrive");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("the mock listener failed: {error}"),
        }
    }
}

fn read_request(socket: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    loop {
        let mut buffer = [0; 4096];
        let count = socket.read(&mut buffer).unwrap();
        assert!(count > 0);
        received.extend_from_slice(&buffer[..count]);
        assert!(received.len() < MAX_RESIDENT_REQUEST_BYTES);
        let Some(header_end) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&received[..header_end]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        if received.len() >= header_end + 4 + length {
            return received[header_end + 4..header_end + 4 + length].to_vec();
        }
    }
}

fn write_response(socket: &mut TcpStream, status: &str, body: &str) {
    write!(
        socket,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    socket.flush().unwrap();
}

fn valid_response() -> Value {
    json!({
        "model": "synthetic-model",
        "answers": {
            "verdict": {
                "type": "choice",
                "choice": "clear",
                "probabilities": {"clear": 0.8, "concern": 0.2},
                "confidence": 0.8,
            },
        },
        "usage": {"input_tokens": 12, "output_tokens": 3},
    })
}

#[test]
fn snapshot_is_immutable_and_saved_provenance_excludes_private_routing() {
    let mut selected = connection("https://gateway.invalid/private-path?token=private-query");
    let mut limits = capabilities(&selected);
    let evaluator = TypeSafeDecisionEvaluator::from_selected_snapshot(
        selected.clone(),
        Some("synthetic-secret".into()),
        limits.clone(),
    )
    .unwrap();
    selected.model = "changed-model".into();
    selected.revision += 1;
    limits.rendering_reserve_tokens += 1;
    let activation = activation();
    let prepared = evaluator
        .prepare(&activation, json!({"rows": ["fixed"]}))
        .unwrap();
    let encoded = serde_json::to_string(&prepared).unwrap();
    for private in [
        "gateway.invalid",
        "private-path",
        "private-query",
        "synthetic-secret",
        "synthetic-reference",
    ] {
        assert!(!encoded.contains(private));
    }
    assert_eq!(
        prepared.request.as_ref().unwrap()["model"],
        "synthetic-model"
    );
    let provenance = prepared_decision_provenance(&prepared).unwrap();
    assert_eq!(provenance.provider, SystemOneProvider::Custom);
    assert_eq!(provenance.model, "synthetic-model");
    assert_eq!(
        provenance.model_revision.as_deref(),
        Some("synthetic-digest")
    );
    assert_eq!(provenance.connection_revision, 7);
    evaluator.validate_prepared(&activation, &prepared).unwrap();
    let restored: PreparedDecision = serde_json::from_str(&encoded).unwrap();
    assert_eq!(prepared, restored);
}

#[test]
fn saved_profile_binds_every_selected_connection_field_and_all_capabilities() {
    let selected = connection("https://gateway.invalid/first");
    let original = evaluator(selected.clone());
    let activation = activation();
    let prepared = original
        .prepare(&activation, json!({"rows": [1, 2]}))
        .unwrap();
    let mut changes = Vec::new();
    let mut endpoint = selected.clone();
    endpoint.endpoint = SystemOneEndpoint::ExactUrl("https://gateway.invalid/second".into());
    changes.push(endpoint);
    let mut mode = selected.clone();
    mode.response_mode = SystemOneResponseMode::CloudflareEnvelope;
    changes.push(mode);
    let mut revision = selected.clone();
    revision.revision += 1;
    changes.push(revision);
    let mut model = selected.clone();
    model.model = "another-model".into();
    changes.push(model);
    let mut digest = selected.clone();
    digest.model_revision = Some("changed-digest".into());
    changes.push(digest);
    let mut provider = selected.clone();
    provider.provider = SystemOneProvider::Ollama;
    provider.endpoint = SystemOneEndpoint::BaseUrl("http://127.0.0.1:9".into());
    changes.push(provider);
    for changed in changes {
        assert_eq!(
            evaluator(changed)
                .validate_prepared(&activation, &prepared)
                .unwrap_err()
                .code,
            PreparationFailureCode::IncompatibleProfile
        );
    }
    for changed in [
        {
            let mut limits = capabilities(&selected);
            limits.rendering_reserve_tokens += 1;
            limits
        },
        {
            let mut limits = capabilities(&selected);
            limits.output.choice.as_mut().unwrap().confidence = false;
            limits
        },
        {
            let mut limits = capabilities(&selected);
            limits.request_body_bytes.source =
                antiburn_local::analysis::jev::capabilities::CapabilitySource::RuntimeMetadata;
            limits
        },
    ] {
        let evaluator =
            TypeSafeDecisionEvaluator::from_selected_snapshot(selected.clone(), None, changed)
                .unwrap();
        assert_eq!(
            evaluator
                .validate_prepared(&activation, &prepared)
                .unwrap_err()
                .code,
            PreparationFailureCode::IncompatibleProfile
        );
    }
}

#[test]
fn invalid_model_binding_and_unverified_choice_outputs_fail_locally() {
    let selected = connection("https://gateway.invalid/first");
    for changed in [
        ModelCapabilities {
            model: "another-model".into(),
            ..capabilities(&selected)
        },
        ModelCapabilities {
            model_revision: Some("another-digest".into()),
            ..capabilities(&selected)
        },
    ] {
        assert!(matches!(
            TypeSafeDecisionEvaluator::from_selected_snapshot(selected.clone(), None, changed),
            Err(JevError::UnsupportedModel)
        ));
    }
    let mut limits = capabilities(&selected);
    limits.output.choice = None;
    let evaluator =
        TypeSafeDecisionEvaluator::from_selected_snapshot(selected, None, limits).unwrap();
    assert_eq!(
        evaluator
            .prepare(&activation(), json!({"rows": [1]}))
            .unwrap_err()
            .code,
        PreparationFailureCode::ProviderUnavailable
    );
}

#[test]
fn output_capabilities_select_label_only_results_without_fabricated_details() {
    let mut activation = activation();
    Arc::make_mut(&mut activation.compiled).actions[0].condition =
        Some(antiburn_checks::compiler::Condition::Boolean(true));
    let selected = connection("https://gateway.invalid/first");
    let mut limits = capabilities(&selected);
    limits.output.choice.as_mut().unwrap().probabilities = false;
    let request =
        prepare_request(&activation.compiled.decision, json!({"rows": [1]}), &limits).unwrap();
    let response: JevResponse = serde_json::from_value(valid_response()).unwrap();
    let result =
        validate_response(&activation.compiled.decision, &request, response, &limits).unwrap();
    let encoded = serde_json::to_value(&result).unwrap();
    assert_eq!(encoded["output"], "clear");
    assert!(encoded.get("probabilities").is_none());
    assert!(encoded.get("confidence").is_none());
    let evaluated = EvaluatedCandidate {
        choices: vec![result],
        attempts: 1,
    };
    evaluated.validate(&activation.compiled).unwrap();
    assert!(evaluated.validate(&self::activation().compiled).is_err());
}

#[test]
fn a_probability_capable_route_never_falls_back_to_label_only_validation() {
    let activation = activation();
    let selected = connection("https://gateway.invalid/first");
    let limits = capabilities(&selected);
    let request =
        prepare_request(&activation.compiled.decision, json!({"rows": [1]}), &limits).unwrap();
    let mut invalid = valid_response();
    invalid["answers"]["verdict"]["probabilities"] = json!({"clear": 1.0});
    let response: JevResponse = serde_json::from_value(invalid).unwrap();
    assert_eq!(
        validate_response(&activation.compiled.decision, &request, response, &limits),
        Err(JevError::InvalidChoiceDistribution)
    );
}

#[tokio::test]
async fn incompatible_saved_requests_are_not_refitted_or_sent_on_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let original = evaluator(connection("https://gateway.invalid/original"));
    let activation = activation();
    let candidate = candidate(&activation);
    let prepared = original
        .prepare(&activation, json!({"rows": ["fixed"]}))
        .unwrap();
    let changed = evaluator(connection(&format!(
        "http://{}/changed",
        listener.local_addr().unwrap()
    )))
    .with_usage(Arc::new(|_, _, _| {
        panic!("incompatible preparation must not settle usage")
    }));
    let failure = changed
        .evaluate_prepared(&activation, &candidate, &prepared, 2)
        .await
        .unwrap_err();
    assert_eq!(failure.code, EvaluationFailureCode::InvalidRequest);
    assert_eq!(failure.attempts, 0);
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    for profile in ["legacy-profile".to_owned(), {
        let mut profile: Value = serde_json::from_str(&prepared.profile).unwrap();
        profile["construction_version"] = json!("changed-version");
        profile.to_string()
    }] {
        let mut changed = prepared.clone();
        changed.profile = profile;
        assert!(original.validate_prepared(&activation, &changed).is_err());
        assert!(prepared_decision_provenance(&changed).is_none());
    }
}

#[tokio::test]
async fn safe_retry_reuses_exact_saved_request_and_records_only_reported_usage() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!(
        "http://{}/exact?route=selected",
        listener.local_addr().unwrap()
    );
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for status in ["429 Too Many Requests", "200 OK"] {
            let mut socket = accept(&listener);
            requests.push(read_request(&mut socket));
            write_response(&mut socket, status, &valid_response().to_string());
        }
        assert_eq!(requests[0], requests[1]);
        requests
    });
    let records = Arc::new(Mutex::new(Vec::new()));
    let captured = records.clone();
    let evaluator =
        evaluator(connection(&endpoint)).with_usage(Arc::new(move |id, attempt, usage| {
            captured
                .lock()
                .unwrap()
                .push((id.to_owned(), attempt, usage));
            Ok(())
        }));
    let activation = activation();
    let candidate = candidate(&activation);
    let prepared = evaluator
        .prepare(&activation, json!({"rows": ["fixed"]}))
        .unwrap();
    let failure = evaluator
        .evaluate_prepared(&activation, &candidate, &prepared, 1)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, AttemptFailure::SafeTransient);
    assert!(failure.detail.contains("Custom model synthetic-model"));
    assert!(!failure.detail.contains("TypeSafe"));
    assert_eq!(
        evaluator
            .evaluate_prepared(&activation, &candidate, &prepared, 2)
            .await
            .unwrap()
            .attempts,
        2
    );
    let requests = tokio::task::spawn_blocking(move || server.join().unwrap())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0]).unwrap(),
        *prepared.request.as_ref().unwrap()
    );
    let records = records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].1, 2);
    assert_eq!(
        records[0].2,
        Some(JevUsage {
            input_tokens: 12,
            output_tokens: 3
        })
    );
}

#[tokio::test]
async fn usage_received_before_cancellation_survives_invalid_answers_in_both_custom_modes() {
    for mode in [
        SystemOneResponseMode::Direct,
        SystemOneResponseMode::CloudflareEnvelope,
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut socket = accept(&listener);
            read_request(&mut socket);
            let mut response = valid_response();
            response["answers"]["verdict"]["choice"] = json!("undeclared");
            let body = if mode == SystemOneResponseMode::CloudflareEnvelope {
                json!({"success": true, "result": response})
            } else {
                response
            };
            write_response(&mut socket, "200 OK", &body.to_string());
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = records.clone();
        let mut selected = connection(&endpoint);
        selected.response_mode = mode;
        let evaluator = evaluator(selected)
            .with_cancellation(move || cancel.load(Ordering::Acquire))
            .with_usage(Arc::new(move |_, _, usage| {
                captured.lock().unwrap().push(usage);
                cancelled.store(true, Ordering::Release);
                Ok(())
            }));
        let activation = activation();
        let prepared = evaluator
            .prepare(&activation, json!({"rows": ["fixed"]}))
            .unwrap();
        let failure = evaluator
            .evaluate_prepared(&activation, &candidate(&activation), &prepared, 1)
            .await
            .unwrap_err();
        assert_eq!(failure.code, EvaluationFailureCode::InvalidResponse);
        assert_eq!(
            *records.lock().unwrap(),
            [Some(JevUsage {
                input_tokens: 12,
                output_tokens: 3
            })]
        );
        tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn post_send_disconnect_records_unknown_once_without_an_automatic_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut socket = accept(&listener);
        read_request(&mut socket);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n")
            .unwrap();
        drop(socket);
        listener
    });
    let records = Arc::new(Mutex::new(Vec::new()));
    let captured = records.clone();
    let evaluator = evaluator(connection(&endpoint)).with_usage(Arc::new(move |_, _, usage| {
        captured.lock().unwrap().push(usage);
        Ok(())
    }));
    let activation = activation();
    let prepared = evaluator
        .prepare(&activation, json!({"rows": ["fixed"]}))
        .unwrap();
    let failure = evaluator
        .evaluate_prepared(&activation, &candidate(&activation), &prepared, 1)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, AttemptFailure::Unknown);
    assert_eq!(failure.attempts, 1);
    assert_eq!(*records.lock().unwrap(), [None]);
    let listener = tokio::task::spawn_blocking(move || server.join().unwrap())
        .await
        .unwrap();
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn selected_ollama_dispatch_uses_resolved_limits_without_discovery() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut socket = accept(&listener);
        let request: JevRequest = serde_json::from_slice(&read_request(&mut socket)).unwrap();
        assert_eq!(request.model, "synthetic-model");
        assert_eq!(request.state["evidence"]["rows"], json!(["fixed"]));
        write_response(&mut socket, "200 OK", &valid_response().to_string());
        listener
    });
    let mut selected = connection(&base);
    selected.provider = SystemOneProvider::Ollama;
    selected.endpoint = SystemOneEndpoint::BaseUrl(base);
    selected.credential = None;
    let records = Arc::new(Mutex::new(Vec::new()));
    let captured = records.clone();
    let limits = capabilities(&selected);
    let evaluator = TypeSafeDecisionEvaluator::from_selected_snapshot(selected, None, limits)
        .unwrap()
        .with_usage(Arc::new(move |_, _, usage| {
            captured.lock().unwrap().push(usage);
            Ok(())
        }));
    let activation = activation();
    let prepared = evaluator
        .prepare(&activation, json!({"rows": ["fixed"]}))
        .unwrap();
    let result = evaluator
        .evaluate_prepared(&activation, &candidate(&activation), &prepared, 1)
        .await
        .unwrap();
    assert_eq!(result.attempts, 1);
    result.validate(&activation.compiled).unwrap();
    assert_eq!(
        *records.lock().unwrap(),
        [Some(JevUsage {
            input_tokens: 12,
            output_tokens: 3
        })]
    );
    let listener = tokio::task::spawn_blocking(move || server.join().unwrap())
        .await
        .unwrap();
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn usage_storage_failure_is_not_hidden_by_cancellation_or_answer_validation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut socket = accept(&listener);
        read_request(&mut socket);
        write_response(&mut socket, "200 OK", &valid_response().to_string());
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel = cancelled.clone();
    let records = Arc::new(Mutex::new(Vec::new()));
    let captured = records.clone();
    let evaluator = evaluator(connection(&endpoint))
        .with_cancellation(move || cancel.load(Ordering::Acquire))
        .with_usage(Arc::new(move |_, _, usage| {
            assert_eq!(
                usage,
                Some(JevUsage {
                    input_tokens: 12,
                    output_tokens: 3
                })
            );
            cancelled.store(true, Ordering::Release);
            Err("synthetic storage failure".into())
        }))
        .with_diagnostics(Arc::new(move |record| {
            captured.lock().unwrap().push(record)
        }));
    let activation = activation();
    let prepared = evaluator
        .prepare(&activation, json!({"rows": ["fixed"]}))
        .unwrap();
    let failure = evaluator
        .evaluate_prepared(&activation, &candidate(&activation), &prepared, 1)
        .await
        .unwrap_err();
    assert_eq!(failure.code, EvaluationFailureCode::StorageFailure);
    assert_eq!(failure.kind, AttemptFailure::Permanent);
    assert!(matches!(
        records.lock().unwrap().as_slice(),
        [ProviderAttemptDiagnostic {
            outcome: AttemptOutcome::Failed {
                error: JevError::ProgressStorageFailure,
                usage: Some(_),
                ..
            },
            ..
        }]
    ));
    tokio::task::spawn_blocking(move || server.join().unwrap())
        .await
        .unwrap();
}
