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

fn step_started(seq: u64, assistant_message_id: &str) -> Value {
    event(
        seq,
        &format!("step-{assistant_message_id}"),
        "session.step.started",
        data(json!({ "assistantMessageID": assistant_message_id })),
        9.0,
    )
}

fn delivered_request(joiner: &mut OpenCodeV2EventJoiner, seq: u64, id: &str, text: &str) {
    joiner.observe(&event(
        seq,
        &format!("enqueued-{id}"),
        "session.inbox.enqueued",
        data(json!({
            "inboxID":"user-message",
            "item":{"type":"user","payload":{"text":text,"files":[],"agents":[],"skills":[]},"delivery":"queue"}
        })),
        seq as f64,
    ));
    joiner.observe(&event(
        seq + 1,
        &format!("delivered-{id}"),
        "session.inbox.delivered",
        data(json!({"inboxID":"user-message"})),
        (seq + 1) as f64,
    ));
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
    delivered_request(&mut joiner, 1, "a", "request");
    assert!(joiner.observe(&step_started(4, "assistant-1")).is_none());
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
        delivered_request(&mut joiner, 1, "a", "request");
        joiner.observe(&step_started(4, "assistant-1"));
        joiner.observe(&started(6, "call-a", "read"));
        joiner.observe(&started(second_sequence + 1, "call-b", "write"));
        let result = joiner
            .observe(&terminal(
                expected_sequence + 2,
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
    delivered_request(&mut joiner, 1, "a", "request");
    joiner.observe(&step_started(4, "assistant-1"));
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
    delivered_request(&mut joiner, 1, "a", "request");
    joiner.observe(&step_started(4, "assistant-1"));
    joiner.observe(&started(5, "call-a", "read"));
    let first = terminal(6, "term-a", "call-a", "session.tool.success", "old", 2.0);
    assert!(joiner.observe(&first).is_some());
    assert!(joiner.observe(&first).is_none());
    let correction = terminal(
        7,
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
    // The native V2 enqueue payload supplies the user text. Delivery admits it.
    delivered_request(&mut joiner, 1, "a", "delivered request");
    joiner.observe(&step_started(4, "assistant-1"));
    joiner.observe(&started(5, "call-a", "read"));
    joiner.observe(&event(6, "called", "session.tool.called", data(json!({
        "assistantMessageID": "assistant-1", "id": "call-a", "input": {"path": "a"}, "executed": true
    })), 4.0));
    let result = joiner
        .observe(&terminal(
            7,
            "term",
            "call-a",
            "session.tool.success",
            "ok",
            5.0,
        ))
        .unwrap();
    assert_eq!(result.request_message_id.as_deref(), Some("user-message"));
    assert_eq!(result.request_text.as_deref(), Some("delivered request"));

    // Text passed as tool input is not promoted to a user request when no
    // delivered user message precedes the call.
    let mut queued_only = OpenCodeV2EventJoiner::new();
    queued_only.observe(&event(
        1,
        "queued-inbox",
        "session.inbox.enqueued",
        data(json!({
            "inboxID":"queued-id",
            "item":{"type":"user","payload":{"text":"queued text"},"delivery":"queue"}
        })),
        1.0,
    ));
    queued_only.observe(&event(
        2,
        "cancel-queued",
        "session.inbox.cancelled",
        data(json!({"inboxID":"queued-id"})),
        2.0,
    ));
    queued_only.observe(&step_started(3, "assistant-1"));
    queued_only.observe(&started(4, "call-q", "execute"));
    queued_only.observe(&event(
        5,
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
            6,
            "term-q",
            "call-q",
            "session.tool.success",
            "done",
            3.0,
        ))
        .unwrap();
    assert_eq!(queued_result.request_message_id, None);
    assert_eq!(queued_result.request_text, None);
    assert!(queued_result.gaps.contains(&Gap::MissingJoin));
}

#[test]
fn a_steer_delivered_during_a_step_does_not_rebind_its_tool_context() {
    let mut joiner = OpenCodeV2EventJoiner::new();
    for (seq, id, message_id, text) in [
        (1, "user-1", "user-1", "first request"),
        (5, "user-2", "user-2", "steer during the step"),
    ] {
        joiner.observe(&event(
            seq,
            &format!("enqueued-{id}"),
            "session.inbox.enqueued",
            data(json!({
                "inboxID": message_id,
                "item":{"type":"user","payload":{"text":text},"delivery":"steer"}
            })),
            seq as f64,
        ));
        joiner.observe(&event(
            seq + 1,
            &format!("delivered-{id}"),
            "session.inbox.delivered",
            data(json!({"inboxID": message_id})),
            (seq + 1) as f64,
        ));
        if seq == 1 {
            joiner.observe(&step_started(4, "assistant-1"));
        }
    }
    joiner.observe(&started(8, "call-a", "read"));
    joiner.observe(&event(
        9,
        "called-a",
        "session.tool.called",
        data(json!({
            "assistantMessageID":"assistant-1", "id":"call-a", "input":{"path":"a"}
        })),
        7.0,
    ));
    let result = joiner
        .observe(&terminal(
            10,
            "terminal-a",
            "call-a",
            "session.tool.success",
            "done",
            8.0,
        ))
        .unwrap();

    assert_eq!(result.request_message_id.as_deref(), Some("user-1"));
    assert_eq!(result.request_text.as_deref(), Some("first request"));

    joiner.observe(&step_started(11, "assistant-2"));
    joiner.observe(&event(
        12,
        "start-call-b",
        "session.tool.input.started",
        data(json!({
            "assistantMessageID":"assistant-2", "id":"call-b", "name":"read"
        })),
        12.0,
    ));
    joiner.observe(&event(
        13,
        "called-b",
        "session.tool.called",
        data(json!({
            "assistantMessageID":"assistant-2", "id":"call-b", "input":{"path":"b"}
        })),
        13.0,
    ));
    let later = joiner
        .observe(&event(
            14,
            "terminal-b",
            "session.tool.success",
            data(json!({
                "assistantMessageID":"assistant-2", "id":"call-b", "executed":true,
                "content":[{"type":"text","text":"done"}]
            })),
            14.0,
        ))
        .unwrap();
    assert_eq!(later.request_message_id.as_deref(), Some("user-2"));
    assert_eq!(later.request_text.as_deref(), Some("steer during the step"));
}

#[test]
fn execute_outer_result_is_not_split_into_nested_tool_results() {
    let mut joiner = OpenCodeV2EventJoiner::new();
    delivered_request(&mut joiner, 1, "execute", "run this");
    joiner.observe(&step_started(4, "assistant-1"));
    joiner.observe(&started(5, "execute-call", "execute"));
    joiner.observe(&event(6, "called", "session.tool.called", data(json!({
        "assistantMessageID": "assistant-1", "id": "execute-call", "input": {"code": "tools.search(...)"}, "executed": true
    })), 2.0));
    let result = joiner
        .observe(&terminal(
            7,
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
