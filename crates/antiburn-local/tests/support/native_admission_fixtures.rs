use serde_json::{Value, json};

pub fn records(agent: &str, kind: &str) -> [String; 4] {
    let (header, original, later) = match agent {
        "claude" => (
            json!({"type":"user","uuid":"00000000-0000-0000-0000-000000000001","parentUuid":null,"sessionId":"facts","version":"2.1.278","entrypoint":"cli","isSidechain":false,"message":{"role":"user","content":"Initial request."}}),
            json!({"type":if kind == "user" {"user"} else {"assistant"},"uuid":"00000000-0000-0000-0000-000000000002","parentUuid":"00000000-0000-0000-0000-000000000001","sessionId":"facts","version":"2.1.278","entrypoint":"cli","isSidechain":false,"message":{"role":if kind == "user" {"user"} else {"assistant"},"content":if kind == "user" {json!("Original request.")} else {json!([{"type":"tool_use","id":"call","name":"Bash","input":{"command":"original"}}])}}}),
            json!({"type":"assistant","uuid":"00000000-0000-0000-0000-000000000003","parentUuid":"00000000-0000-0000-0000-000000000002","sessionId":"facts","version":"2.1.278","entrypoint":"cli","isSidechain":false,"message":{"role":"assistant","content":[{"type":"tool_use","id":"later-call","name":"Bash","input":{"command":"later"}}]}}),
        ),
        "pi" => (
            json!({"type":"session","version":3,"id":"facts","timestamp":100}),
            json!({"type":"message","id":"original","parentId":null,"timestamp":200,"message":{"role":if kind == "user" {"user"} else {"assistant"},"content":if kind == "user" {json!("Original request.")} else {json!([{"type":"toolCall","id":"call","name":"bash","arguments":{"command":"original"}}])}}}),
            json!({"type":"message","id":"later","parentId":"original","timestamp":400,"message":{"role":"assistant","content":[{"type":"toolCall","id":"later-call","name":"bash","arguments":{"command":"later"}}]}}),
        ),
        _ => panic!("unsupported admission fixture agent"),
    };
    let mut conflict: Value = original.clone();
    if kind == "user" {
        conflict["message"]["content"] = json!("Conflicting request.");
    } else if agent == "claude" {
        conflict["message"]["content"][0]["input"]["command"] = json!("conflicting");
    } else {
        conflict["message"]["content"][0]["arguments"]["command"] = json!("conflicting");
    }
    [header, original, conflict, later].map(|value| format!("{value}\n"))
}

pub fn saturated_pi_call_owner_conflict() -> [String; 4] {
    let mut prefix = format!(
        "{}\n",
        json!({"type":"session","version":3,"id":"facts","timestamp":100})
    );
    let mut parent: Option<String> = None;
    for index in 0..4096 {
        let id = format!("model-{index:04}");
        let record = json!({"type":"model_change","id":id,"parentId":parent,"timestamp":200 + index,"provider":"openai","modelId":"synthetic-model"});
        prefix.push_str(&format!("{record}\n"));
        parent = Some(id);
    }
    let call = json!({"type":"message","id":"owner-a","parentId":parent,"timestamp":5000,"message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":"unresolved-call","name":"bash","arguments":{"command":"original"}}]}});
    let between = json!({"type":"model_change","id":"between-b","parentId":"owner-a","timestamp":6000,"provider":"openai","modelId":"synthetic-model"});
    let mut conflict = call.clone();
    conflict["message"]["content"][0]["arguments"]["command"] = json!("conflicting");
    conflict["timestamp"] = json!(7000);
    [
        prefix,
        format!("{call}\n"),
        format!("{between}\n"),
        format!("{conflict}\n"),
    ]
}
