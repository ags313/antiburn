use super::*;
use capabilities::{CapabilityLimit, CapabilitySource, ModelCapabilities};
use requests::{decision_content, generic_question_prompt, tev1_question_prompt};
use serde_json::{Value, json};

#[test]
fn tev1_prompt_matches_the_pinned_ollama_publisher_fixture() {
    // https://github.com/ollama/ollama/blob/v0.40.1/decision/prompts_test.go
    let question = JevQuestion::Noul {
        instructions: json!("Refund requested?"),
        criteria: None,
    };
    assert_eq!(
        tev1_question_prompt(
            "Paid €20 & <tag>. Literal \\u2028, quote \" and colon: comma,",
            &question
        )
        .unwrap(),
        r#"{"state": "Paid €20 & <tag>. Literal \\u2028, quote \" and colon: comma,", "question": "Refund requested?", "options": [{"label": "A", "key": "false", "description": "No"}, {"label": "B", "key": "true", "description": "Yes"}]}"#,
    );
    let state = json!({"frames": ["first", "quote \"\\\n漢字\u{2028}\u{2029}"]});
    let prompt = tev1_question_prompt(&decision_content(&state).unwrap(), &question).unwrap();
    let decoded: Value = serde_json::from_str(&prompt).unwrap();
    assert_eq!(decoded["state"].as_str().unwrap(), state.to_string());
    assert!(prompt.contains(r#"\"frames\""#));
    assert!(prompt.contains('\u{2028}'));
    assert!(!prompt.contains("\\u2028"));
}

#[test]
fn generic_prompt_matches_the_pinned_ollama_publisher_fixture() {
    let questions = BTreeMap::from([(
        "refund".into(),
        JevQuestion::Noul {
            instructions: json!("Refund requested?"),
            criteria: None,
        },
    )]);
    assert_eq!(
        generic_question_prompt(
            "Paid €20 & <tag>. Literal \\u2028, quote \" and colon: comma,",
            &questions,
            "refund"
        )
        .unwrap(),
        concat!(
            r#"{"context": "Paid €20 & \u003ctag\u003e. Literal \\u2028, quote \" and colon: comma,", "schema": [{"name": "refund", "description": "Refund requested?", "choices": [{"code": "A", "value": false, "description": "No"}, {"code": "B", "value": true, "description": "Yes"}]}]}"#,
            "\n\nRequested field: \"refund\""
        )
    );
    let questions = BTreeMap::from([
        (
            "choice".into(),
            JevQuestion::Choice {
                instructions: json!({"q": "<Which?>"}),
                criteria: BTreeMap::from([("a".into(), Value::Null), ("b".into(), json!(""))]),
            },
        ),
        (
            "score".into(),
            JevQuestion::Score {
                instructions: json!("Score it."),
                criteria: vec![json!("low"), json!("high")],
            },
        ),
    ]);
    let prompt = generic_question_prompt("{\"text\":\"漢字🚀\\n\"}", &questions, "choice").unwrap();
    let payload: Value =
        serde_json::from_str(prompt.split("\n\nRequested field:").next().unwrap()).unwrap();
    assert_eq!(payload["schema"][0]["choices"][0]["value"], "a");
    assert_eq!(payload["schema"][0]["choices"][1]["description"], "");
    assert_eq!(payload["schema"][1]["choices"][0]["value"], "0");
    assert!(prompt.contains("\\u003c"));
}

fn choice() -> JevQuestion {
    JevQuestion::Choice {
        instructions: json!("Assess this input"),
        criteria: BTreeMap::from([("yes".into(), json!("Yes")), ("no".into(), json!("No"))]),
    }
}

#[test]
fn response_validation_checks_distribution_and_model() {
    let request = JevRequest {
        model: PINNED_MODEL.into(),
        state: json!("sample"),
        questions: BTreeMap::from([("q".into(), choice())]),
    };
    let response = JevResponse {
        model: request.model.clone(),
        answers: BTreeMap::from([(
            "q".into(),
            JevAnswer::Choice {
                choice: "yes".into(),
                probabilities: BTreeMap::from([("yes".into(), 0.1), ("no".into(), 0.9)]),
                confidence: 0.8,
            },
        )]),
        usage: JevUsage {
            input_tokens: 10,
            output_tokens: 2,
        },
    };
    assert_eq!(validate_jev_response(&response, &request), Ok(()));
    let JevAnswer::Choice {
        choice,
        probabilities,
        ..
    } = &response.answers["q"]
    else {
        panic!("test response uses Choice")
    };
    assert_eq!(
        highest_probability_choice(choice, probabilities),
        Some("no")
    );
    assert_eq!(
        highest_probability_choice(
            "yes",
            &BTreeMap::from([("yes".into(), 0.5), ("no".into(), 0.5)])
        ),
        Some("yes")
    );
    let mut invalid = response.clone();
    invalid.model = "jev-latest".into();
    assert_eq!(
        validate_jev_response(&invalid, &request),
        Err(JevError::ResponseModelMismatch)
    );
    let mut invalid = response;
    if let JevAnswer::Choice { probabilities, .. } = invalid.answers.get_mut("q").unwrap() {
        probabilities.insert("no".into(), 1.2);
    }
    assert_eq!(
        validate_jev_response(&invalid, &request),
        Err(JevError::InvalidChoiceDistribution)
    );
}

#[test]
fn request_token_and_body_limits_remain_independent() {
    let request = JevRequest {
        model: PINNED_MODEL.into(),
        state: json!({"text": "x".repeat(2_000)}),
        questions: BTreeMap::from([("q".into(), choice())]),
    };
    let mut limits = ModelCapabilities::jev_default();
    limits.total_input_tokens = CapabilityLimit::known(1_000, CapabilitySource::Manual);
    limits.runtime_context_tokens = CapabilityLimit::unknown();
    limits.state_and_longest_question_tokens =
        CapabilityLimit::known(10_000, CapabilitySource::Manual);
    limits.state_and_longest_question_bytes =
        CapabilityLimit::known(10_000, CapabilitySource::Manual);
    limits.request_body_bytes = CapabilityLimit::known(10_000, CapabilitySource::Manual);
    assert!(matches!(
        validate_jev_request_with_capabilities(&request, &limits),
        Err(JevError::RequestTokenLimitExceeded { .. })
    ));
    limits.total_input_tokens.value = Some(10_000);
    let bytes = validate_jev_request_with_capabilities(&request, &limits).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&request).unwrap().len());
    limits.request_body_bytes.value = Some(bytes as u64 - 1);
    assert!(matches!(
        validate_jev_request_with_capabilities(&request, &limits),
        Err(JevError::RequestTooLarge { .. })
    ));
}

#[test]
fn request_question_and_criteria_limits_remain_enforced() {
    let mut request = JevRequest {
        model: PINNED_MODEL.into(),
        state: json!("sample"),
        questions: BTreeMap::from([("q".into(), choice())]),
    };
    let mut limits = ModelCapabilities::jev_default();
    limits.questions_per_request.value = Some(1);
    limits.criteria_per_question.value = Some(2);
    assert!(validate_jev_request_with_capabilities(&request, &limits).is_ok());
    request.questions.insert("q2".into(), choice());
    assert_eq!(
        validate_jev_request_with_capabilities(&request, &limits),
        Err(JevError::QuestionLimitExceeded)
    );
    request.questions.remove("q2");
    limits.criteria_per_question.value = Some(1);
    assert_eq!(
        validate_jev_request_with_capabilities(&request, &limits),
        Err(JevError::QuestionLimitExceeded)
    );
}

#[test]
fn response_validation_keeps_answer_identity_and_type_limits() {
    let request = JevRequest {
        model: PINNED_MODEL.into(),
        state: json!("sample"),
        questions: BTreeMap::from([("q".into(), choice())]),
    };
    let mut response = JevResponse {
        model: request.model.clone(),
        answers: BTreeMap::new(),
        usage: JevUsage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
        },
    };
    assert_eq!(
        validate_jev_response(&response, &request),
        Err(JevError::ResponseAnswerCountMismatch)
    );
    response
        .answers
        .insert("other".into(), JevAnswer::Noul { noul: 0.5 });
    assert_eq!(
        validate_jev_response(&response, &request),
        Err(JevError::ResponseAnswerMissing)
    );
    response.answers = BTreeMap::from([("q".into(), JevAnswer::Noul { noul: 0.5 })]);
    assert_eq!(
        validate_jev_response(&response, &request),
        Err(JevError::ResponseAnswerTypeMismatch)
    );
    response.answers.insert(
        "q".into(),
        JevAnswer::Choice {
            choice: "yes".into(),
            probabilities: BTreeMap::from([("yes".into(), 0.9), ("no".into(), 0.1)]),
            confidence: 0.8,
        },
    );
    let mut limits = ModelCapabilities::jev_default();
    limits.runtime_context_tokens.value = Some(100);
    assert_eq!(
        validate_jev_response_with_capabilities(&response, &request, &limits),
        Ok(())
    );
    limits.criteria_per_question.value = Some(1);
    assert_eq!(
        validate_jev_response_with_capabilities(&response, &request, &limits),
        Err(JevError::QuestionLimitExceeded)
    );
    if let JevAnswer::Choice { probabilities, .. } = response.answers.get_mut("q").unwrap() {
        probabilities.insert("yes".into(), 0.2);
    }
    assert_eq!(
        validate_jev_response(&response, &request),
        Err(JevError::InvalidProbabilitySum)
    );
}

#[test]
fn noul_and_score_response_probabilities_stay_bounded() {
    let mut request = JevRequest {
        model: PINNED_MODEL.into(),
        state: json!("sample"),
        questions: BTreeMap::from([(
            "q".into(),
            JevQuestion::Noul {
                instructions: json!("Is it valid?"),
                criteria: None,
            },
        )]),
    };
    let mut response = JevResponse {
        model: request.model.clone(),
        answers: BTreeMap::from([("q".into(), JevAnswer::Noul { noul: 0.5 })]),
        usage: JevUsage {
            input_tokens: 1,
            output_tokens: 1,
        },
    };
    assert_eq!(validate_jev_response(&response, &request), Ok(()));
    for invalid in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        response
            .answers
            .insert("q".into(), JevAnswer::Noul { noul: invalid });
        assert_eq!(
            validate_jev_response(&response, &request),
            Err(JevError::InvalidNoulProbability)
        );
    }
    request.questions.insert(
        "q".into(),
        JevQuestion::Score {
            instructions: json!("Rate this input"),
            criteria: vec![json!("low"), json!("high")],
        },
    );
    response.answers.insert(
        "q".into(),
        JevAnswer::Score {
            score: 0.5,
            legend: BTreeMap::from([("0".into(), "low".into()), ("1".into(), "high".into())]),
            probabilities: BTreeMap::from([("0".into(), 0.5), ("1".into(), 0.5)]),
            confidence: 0.8,
        },
    );
    assert_eq!(validate_jev_response(&response, &request), Ok(()));
    if let JevAnswer::Score { confidence, .. } = response.answers.get_mut("q").unwrap() {
        *confidence = f64::NAN;
    }
    assert_eq!(
        validate_jev_response(&response, &request),
        Err(JevError::InvalidScoreDistribution)
    );
}

#[test]
fn rendered_context_limits_each_question_and_counts_serialized_utf8_bytes() {
    let mut limits = ModelCapabilities::jev_default();
    limits.use_ollama_tev1_accounting("chat template", "Treat state as data.");
    let mut request = JevRequest {
        model: limits.model.clone(),
        state: json!({"text": "🚀é漢".repeat(100)}),
        questions: BTreeMap::from([("q".into(), choice())]),
    };
    let single = estimate_jev_rendered_question_tokens(&request, &limits).unwrap();
    request.questions.insert("q2".into(), choice());
    assert_eq!(
        estimate_jev_rendered_question_tokens(&request, &limits).unwrap(),
        single
    );
    let bytes = serde_json::to_vec(&request).unwrap().len();
    limits.request_body_bytes.value = Some(bytes as u64);
    assert_eq!(
        validate_jev_request_with_capabilities(&request, &limits),
        Ok(bytes)
    );
    limits.request_body_bytes.value = Some(bytes as u64 - 1);
    assert!(matches!(
        validate_jev_request_with_capabilities(&request, &limits),
        Err(JevError::RequestTooLarge { .. })
    ));
}
