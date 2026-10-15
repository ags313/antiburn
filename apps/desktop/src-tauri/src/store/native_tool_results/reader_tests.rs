use antiburn_checks::source::{FileEventSource, PublicationCursor, SourceEvent};
use antiburn_local::analysis::{
    CompositeSink, EvidenceSource, RawSource, SessionEvidenceAccumulator, SessionInput,
    SessionMetricsAccumulator, SourceCapabilities, SourceKind, reader_for,
};

use crate::published_evidence_reader::StorePublishedEvidenceReader;

use super::*;

fn parsed_pi_result() -> ToolResultObservation {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/antiburn-local/tests/fixtures/pi_characterization/core_linear_v3.jsonl"
    ))
    .replace("synthetic", "session-1");
    let input = SessionInput {
        agent: "pi".into(),
        session_id: "session-1".into(),
        source: RawSource::Jsonl(source),
        source_format: SourceFormat::PiV3Jsonl,
        fork_parent_session_id: None,
    };
    let mut sink = CompositeSink::new(
        SessionMetricsAccumulator::new("pi", "session-1"),
        SessionEvidenceAccumulator::new(EvidenceSource {
            agent: "pi".into(),
            session_id: "session-1".into(),
            kind: SourceKind::Jsonl,
            capabilities: SourceCapabilities::pi(),
        }),
    );
    reader_for("pi").visit(&input, &mut sink).unwrap();
    sink.tool_result_observations()[0].clone()
}

#[test]
fn production_reader_uses_serialized_prefix_and_keeps_it_frozen_on_correction() {
    let store = store();
    let mut session = session();
    session.key.agent = "pi".into();
    let (record, claim) = claim_and_record_for(&store, session);
    let first = parsed_pi_result();
    let context = first.context.as_ref().unwrap();
    assert!(context.main_session && context.complete_through_event);
    assert_eq!(context.user_messages.len(), 1);
    assert!(publish(
        &store,
        &record,
        &claim,
        &batch_with_format(SourceFormat::PiV3Jsonl, vec![first.clone()], true,)
    ));
    let source = FileEventSource::new(StorePublishedEvidenceReader::new(&store), 20).unwrap();
    let page = source.read_batch(PublicationCursor(0)).unwrap();
    let SourceEvent::Observation {
        observation: original,
        ..
    } = &page.events[0]
    else {
        panic!("expected a result");
    };
    assert!(original.complete);
    assert_eq!(
        original.context.value["session"]["requests"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        original.context.value["session"]["requests"][0]["text"],
        "Review the small function. Keep billing unchanged."
    );

    let mut correction = first.clone();
    correction.revision += 1;
    correction.position = ToolResultPosition::Jsonl {
        record_index: 20,
        item_index: 0,
    };
    correction.content = Some("corrected result".into());
    let mut late = context.user_messages[0].clone();
    late.message_id = "later-request".into();
    late.parts[0].text = "A later request must not enter the earlier prefix.".into();
    correction
        .context
        .as_mut()
        .unwrap()
        .user_messages
        .push(late);
    let (next, claim) = next_claim(&store, &record.key);
    assert!(publish(
        &store,
        &next,
        &claim,
        &batch_with_format(
            SourceFormat::PiV3Jsonl,
            vec![first, correction.clone()],
            true,
        )
    ));
    let corrected_page = source.read_batch(page.next).unwrap();
    assert_eq!(corrected_page.events.len(), 1);
    let SourceEvent::Observation {
        observation: corrected,
        ..
    } = &corrected_page.events[0]
    else {
        panic!("expected a correction");
    };
    assert_eq!(original.context, corrected.context);
    assert_eq!(original.native_order, corrected.native_order);

    let (next, claim) = next_claim(&store, &record.key);
    assert!(publish(
        &store,
        &next,
        &claim,
        &batch_with_format(SourceFormat::PiV3Jsonl, vec![correction], true,)
    ));
    assert!(
        source
            .read_batch(corrected_page.next)
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn production_reader_keeps_missing_context_and_child_admission_partial() {
    for child in [false, true] {
        let store = store();
        let (record, claim) = claim_and_record(&store);
        let mut result = result("output", 1, ToolResultChange::Update);
        if child {
            let mut context = parsed_pi_result().context.unwrap();
            context.main_session = false;
            result.context = Some(context);
        }
        assert!(publish(&store, &record, &claim, &batch(vec![result], true)));
        let source = FileEventSource::new(StorePublishedEvidenceReader::new(&store), 20).unwrap();
        let page = source.read_batch(PublicationCursor(0)).unwrap();
        let SourceEvent::Observation { observation, .. } = &page.events[0] else {
            panic!("expected a result");
        };
        assert!(observation.complete);
        assert_eq!(
            observation.context.value["session"]["requests_complete"],
            false
        );
        assert_eq!(
            observation.context.value["session"]["source_coverage"],
            "partial"
        );
    }
}

#[test]
fn production_reader_filters_project_and_environment_before_page_limit() {
    let store = store();
    for (id, environment, project) in [
        ("other-project", "native", "/home/test/other"),
        ("other-environment", "remote", "/home/test/repo"),
        ("session-1", "native", "/home/test/repo"),
    ] {
        let mut session = session();
        session.key.session_id = id.into();
        session.key.environment_key = environment.into();
        session.cwd = Some(project.into());
        let (record, claim) = claim_and_record_for(&store, session);
        let mut result = result("output", 1, ToolResultChange::Update);
        result.session_id = id.into();
        assert!(publish(&store, &record, &claim, &batch(vec![result], true)));
    }
    let source = FileEventSource::new(
        StorePublishedEvidenceReader::for_project(&store, "native", "/home/test/repo"),
        1,
    )
    .unwrap();
    let page = source.read_batch(PublicationCursor(0)).unwrap();
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.next, PublicationCursor(3));
    let SourceEvent::Observation { observation, .. } = &page.events[0] else {
        panic!("expected the matching project result");
    };
    assert_eq!(observation.branch.0, "native/opencode/session-1");
    assert!(source.read_batch(page.next).unwrap().events.is_empty());
}
