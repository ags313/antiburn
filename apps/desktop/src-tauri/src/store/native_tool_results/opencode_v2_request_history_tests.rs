use super::*;
use crate::published_evidence_reader::StorePublishedEvidenceReader;
use antiburn_checks::definition_lifecycle::EffectiveActivation;
use antiburn_checks::domain::{
    ActivationId, Generation, Observation, ObservationKind, ObservationPosition,
};
use antiburn_checks::preparation::ConditionOutcome;
use antiburn_checks::selection::{SelectionOutcome, SelectionState};
use antiburn_checks::source::{PublicationCursor, PublishedEvidenceSource, SourceEvent};
use antiburn_local::analysis::NativeEvidenceFact;
use antiburn_local::analysis::opencode_v2_events::OpenCodeV2EventJoiner;
use serde_json::Value;
use std::sync::Arc;

fn execution(index: u64, sequence: &mut u64, text: &str) -> Vec<Value> {
    let fixture: Vec<Value> = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/antiburn-local/tests/fixtures/opencode_public_events/v2.jsonl"
    ))
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect();
    fixture[..14]
        .iter()
        .chain(fixture.last())
        .cloned()
        .map(|mut event| {
            *sequence += 1;
            event["id"] = json!(format!("{index}-{}", event["id"].as_str().unwrap()));
            event["durable"]["aggregateID"] = json!("native-session");
            event["durable"]["seq"] = json!(*sequence);
            event["data"]["sessionID"] = json!("native-session");
            for field in ["inboxID", "assistantMessageID", "id"] {
                if let Some(id) = event["data"][field].as_str() {
                    event["data"][field] = json!(format!("{index}-{id}"));
                }
            }
            if event["type"] == "session.inbox.enqueued" {
                event["data"]["item"]["payload"]["text"] = json!(text);
            }
            event
        })
        .collect()
}

fn publish(store: &Store, claim: &OpenCodeV2CaptureClaim, events: &[Value]) -> Vec<bool> {
    let mut joiner = OpenCodeV2EventJoiner::new();
    joiner.set_live_baseline(claim.baseline_sequence.unwrap());
    let mut cycle_completeness = Vec::new();
    for event in events {
        let batch = joiner.observe_evidence(event);
        assert!(joiner.gaps().is_empty());
        cycle_completeness.extend(batch.events.iter().filter_map(|event| match &event.fact {
            NativeEvidenceFact::ResponseCycle(cycle) => Some(cycle.context.complete_through_event),
            _ => None,
        }));
        assert!(
            store
                .advance_opencode_v2_evidence(
                    claim,
                    event["durable"]["seq"].as_u64().unwrap(),
                    &batch,
                    101,
                    60,
                )
                .unwrap()
        );
    }
    cycle_completeness
}

fn turns(store: &Store) -> Vec<Observation> {
    let page = PublishedEvidenceSource::new(
        StorePublishedEvidenceReader::for_project(store, "native", "/synthetic"),
        100,
    )
    .unwrap()
    .read_batch(PublicationCursor(0))
    .unwrap();
    assert!(!page.has_more);
    page.events
        .into_iter()
        .filter_map(|event| match event {
            SourceEvent::Observation {
                cursor,
                observation,
                ..
            } if matches!(observation.kind, ObservationKind::TurnEnd { .. }) => {
                Some(observation.commit(ObservationPosition(cursor.0)))
            }
            _ => None,
        })
        .collect()
}

fn activation(condition: &str) -> EffectiveActivation {
    let source = format!(
        "name: Request history\non:\n  turn_end:\nif: {condition}\nactions:\n  - type: status\n    input: {{result: failure}}\n"
    );
    EffectiveActivation {
        id: ActivationId {
            scope: Default::default(),
            definition_id: "request-history".into(),
            generation: Generation(1),
        },
        revision: [1; 32],
        definition_scope: antiburn_checks::discovery::DefinitionScope::Global,
        source_paths: Vec::new(),
        project_root: Some("/synthetic".into()),
        compiled: Arc::new(antiburn_checks::compile(&source).unwrap()),
        bootstrap_observation_position: ObservationPosition(0),
        bootstrap_publication: PublicationCursor(0),
    }
}

fn complete_capture(store: &Store, claim: &OpenCodeV2CaptureClaim, sequence: &mut u64) {
    *sequence += 1;
    assert!(
        store
            .advance_opencode_v2_capture(claim, *sequence, Some(&result(*sequence, true)), 101, 60)
            .unwrap()
    );
}

#[test]
fn v2_second_turn_selects_the_retained_constraint_without_adding_old_members() {
    let (_directory, store, _journal, start) = setup();
    let claim = started(&store, &start);
    let mut sequence = claim.baseline_sequence.unwrap();
    complete_capture(&store, &claim, &mut sequence);
    let mut events = execution(1, &mut sequence, "Do not change generated files.");
    events.extend(execution(2, &mut sequence, "Read the synthetic file."));
    assert_eq!(publish(&store, &claim, &events), [true, true]);
    let turns = turns(&store);
    assert_eq!(turns.len(), 2);
    let active = activation(
        "session.requests.length == 2 && contains(session.requests.*.text, 'Do not change generated files.')",
    );
    let SelectionOutcome::Candidate(candidate) =
        SelectionState::default().apply(&active, &turns[1])
    else {
        panic!("the second turn must keep the earlier constraint");
    };
    let session = &candidate.context.value["session"];
    assert_eq!(session["requests_complete"], true);
    assert_eq!(
        session["requests"][0]["text"],
        "Do not change generated files."
    );
    assert_eq!(session["requests"][1]["text"], "Read the synthetic file.");
    let members = candidate.context.value["turn"]["messages"]
        .as_array()
        .unwrap();
    assert_eq!(members.len(), 4);
    assert!(members.iter().all(|message| {
        message["id"].as_str().unwrap().contains("2-")
            && !message["id"].as_str().unwrap().contains("1-assistant")
    }));
    assert!(
        !serde_json::to_string(members)
            .unwrap()
            .contains("Do not change generated files.")
    );
}

#[test]
fn v2_partial_turn_history_remains_usable_but_blocks_request_membership() {
    for limit in ["discarded-request", "capture-origin"] {
        let (_directory, store, _journal, start) = setup();
        let claim = started(&store, &start);
        let mut sequence = claim.baseline_sequence.unwrap();
        if limit == "discarded-request" {
            complete_capture(&store, &claim, &mut sequence);
        }
        let mut events = execution(1, &mut sequence, "DISCARDED_EARLIER_REQUEST");
        if limit == "discarded-request" {
            let mut partial = execution(9, &mut sequence, "DISCARDED_PARTIAL_REQUEST");
            partial[0]["data"]["item"]["payload"]["files"] = json!([{"path":"synthetic.png"}]);
            events.extend([partial[0].clone(), partial[2].clone()]);
        }
        events.extend(execution(2, &mut sequence, "Read the synthetic file."));
        events.retain(|event| !event["type"].as_str().unwrap().starts_with("session.tool."));
        assert_eq!(
            publish(&store, &claim, &events),
            if limit == "capture-origin" {
                [true, true]
            } else {
                [true, false]
            },
            "{limit}"
        );
        let turns = turns(&store);
        assert_eq!(turns.len(), 2, "{limit}");
        let usable = activation("true");
        let SelectionOutcome::Candidate(candidate) =
            SelectionState::default().apply(&usable, &turns[1])
        else {
            panic!("partial request history must not block unrelated turn checks: {limit}");
        };
        assert_eq!(
            candidate.context.value["session"]["requests_complete"], false,
            "{limit}"
        );
        let requests = &candidate.context.value["session"]["requests"];
        if limit == "discarded-request" {
            assert_eq!(requests.as_array().unwrap().len(), 1);
            assert_eq!(requests[0]["text"], "Read the synthetic file.");
            assert!(
                !serde_json::to_string(requests)
                    .unwrap()
                    .contains("DISCARDED_")
            );
        } else {
            assert_eq!(requests.as_array().unwrap().len(), 2);
        }
        assert_eq!(
            SelectionState::default().apply(&activation("session.requests.length == 2"), &turns[1]),
            SelectionOutcome::ConditionFiltered(ConditionOutcome::Unavailable),
            "{limit}"
        );
    }
}
