#[path = "../src/analysis/opencode_v2_events.rs"]
mod opencode_v2_events;

use opencode_v2_events::{Gap, OpenCodeV2EventJoiner, TerminalStatus};
use serde_json::{Value, json};

fn event(seq: u64, id: &str, event_type: &str, data: Value, created: f64) -> Value {
    json!({
        "id": id,
        "type": event_type,
        "created": created,
        "durable": {"aggregateID": "sess-1", "seq": seq, "version": 2},
        "data": data,
    })
}

fn data(extra: Value) -> Value {
    let mut value = json!({"sessionID": "sess-1"});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    value
}

fn started(seq: u64, call: &str, name: &str) -> Value {
    event(
        seq,
        &format!("start-{call}"),
        "session.tool.input.started",
        data(json!({
            "assistantMessageID": "assistant-1", "id": call, "name": name,
        })),
        10.0,
    )
}

fn terminal(seq: u64, id: &str, call: &str, status: &str, content: &str, created: f64) -> Value {
    event(
        seq,
        id,
        status,
        data(json!({
            "assistantMessageID": "assistant-1", "id": call, "executed": true,
            "content": [{"type": "text", "text": content}],
        })),
        created,
    )
}

#[test]
fn increasing_sequence_jumps_are_safe_even_when_timestamps_run_backwards() {
    let mut joiner = OpenCodeV2EventJoiner::new();
    assert!(joiner.observe(&started(5, "call-a", "read")).is_none());
    assert!(joiner.observe(&started(6, "call-b", "write")).is_none());

    let later_timestamp_first = terminal(9, "term-a", "call-a", "session.tool.success", "A", 30.0);
    let earlier_timestamp_second =
        terminal(10, "term-b", "call-b", "session.tool.success", "B", 20.0);
    let first = joiner.observe(&later_timestamp_first).unwrap();
    let second = joiner.observe(&earlier_timestamp_second).unwrap();
    assert_eq!(first.sequence, 9);
    assert_eq!(second.sequence, 10);
    assert_eq!(first.created, Some(30.0));
    assert_eq!(second.created, Some(20.0));
    assert!(first.gaps.is_empty());
    assert!(second.gaps.is_empty());
}

#[test]
fn equal_or_regressed_sequence_marks_later_results_unsafe() {
    for (second_sequence, expected_sequence) in [(5, 5), (4, 4)] {
        let mut joiner = OpenCodeV2EventJoiner::new();
        joiner.observe(&started(5, "call-a", "read"));
        joiner.observe(&started(second_sequence, "call-b", "write"));
        let result = joiner
            .observe(&terminal(
                expected_sequence + 1,
                "terminal-b",
                "call-b",
                "session.tool.success",
                "done",
                6.0,
            ))
            .unwrap();
        assert!(result.gaps.contains(&Gap::SequenceDiscontinuity));
        assert!(joiner.gaps().contains(&Gap::SequenceDiscontinuity));
    }
}

#[test]
fn caller_can_mark_transport_gap_for_future_results() {
    let mut joiner = OpenCodeV2EventJoiner::new();
    joiner.mark_gap();
    joiner.observe(&started(5, "call-a", "read"));
    let result = joiner
        .observe(&terminal(
            9,
            "terminal-a",
            "call-a",
            "session.tool.success",
            "done",
            6.0,
        ))
        .unwrap();
    assert!(result.gaps.contains(&Gap::ContinuityLost));
    assert!(joiner.gaps().contains(&Gap::ContinuityLost));
}

#[test]
fn missing_call_join_is_explicit_and_not_emitted() {
    let mut joiner = OpenCodeV2EventJoiner::new();
    assert!(
        joiner
            .observe(&terminal(
                1,
                "term",
                "unknown",
                "session.tool.success",
                "x",
                1.0
            ))
            .is_none()
    );
    assert!(joiner.gaps().contains(&Gap::MissingJoin));
}

#[test]
fn exact_duplicate_is_ignored_and_terminal_correction_is_unsafe() {
    let mut joiner = OpenCodeV2EventJoiner::new();
    joiner.observe(&started(1, "call-a", "read"));
    let first = terminal(2, "term-a", "call-a", "session.tool.success", "old", 2.0);
    assert!(joiner.observe(&first).is_some());
    assert!(joiner.observe(&first).is_none());
    let correction = terminal(
        3,
        "term-b",
        "call-a",
        "session.tool.success",
        "corrected",
        3.0,
    );
    assert!(joiner.observe(&correction).is_none());
    assert!(joiner.gaps().contains(&Gap::ConflictingDuplicate));
}

#[test]
fn only_delivered_user_message_provides_request_context() {
    let mut joiner = OpenCodeV2EventJoiner::new();
    // A queued request has no delivered message event and is not supplied as context.
    joiner.observe(&event(
        1,
        "user",
        "message.updated",
        data(json!({
            "info": {"id": "user-1", "sessionID": "sess-1", "role": "user"}
        })),
        1.0,
    ));
    joiner.observe(&event(2, "part", "message.part.updated", data(json!({
        "part": {"id": "part-1", "sessionID": "sess-1", "messageID": "user-1", "type": "text", "text": "delivered request"}
    })), 2.0));
    joiner.observe(&started(3, "call-a", "read"));
    joiner.observe(&event(4, "called", "session.tool.called", data(json!({
        "assistantMessageID": "assistant-1", "id": "call-a", "input": {"path": "a"}, "executed": true
    })), 4.0));
    let result = joiner
        .observe(&terminal(
            5,
            "term",
            "call-a",
            "session.tool.success",
            "ok",
            5.0,
        ))
        .unwrap();
    assert_eq!(result.request_message_id.as_deref(), Some("user-1"));
    assert_eq!(result.request_text.as_deref(), Some("delivered request"));

    // Text passed as tool input is not promoted to a user request when no
    // delivered user message precedes the call.
    let mut queued_only = OpenCodeV2EventJoiner::new();
    queued_only.observe(&started(1, "call-q", "execute"));
    queued_only.observe(&event(
        2,
        "called-q",
        "session.tool.called",
        data(json!({
            "assistantMessageID": "assistant-1", "id": "call-q",
            "input": {"prompt": "queued text"}, "executed": true
        })),
        2.0,
    ));
    let queued_result = queued_only
        .observe(&terminal(
            3,
            "term-q",
            "call-q",
            "session.tool.success",
            "done",
            3.0,
        ))
        .unwrap();
    assert_eq!(queued_result.request_message_id, None);
    assert_eq!(queued_result.request_text, None);
}

#[test]
fn execute_outer_result_is_not_split_into_nested_tool_results() {
    let mut joiner = OpenCodeV2EventJoiner::new();
    joiner.observe(&started(1, "execute-call", "execute"));
    joiner.observe(&event(2, "called", "session.tool.called", data(json!({
        "assistantMessageID": "assistant-1", "id": "execute-call", "input": {"code": "tools.search(...)"}, "executed": true
    })), 2.0));
    let result = joiner
        .observe(&terminal(
            3,
            "execute-terminal",
            "execute-call",
            "session.tool.success",
            "nested result text",
            3.0,
        ))
        .unwrap();
    assert_eq!(result.tool_name, "execute");
    assert_eq!(result.status, TerminalStatus::Success);
    assert_eq!(result.content.as_deref(), Some("nested result text"));
    assert_eq!(result.call_id, "execute-call");
}
