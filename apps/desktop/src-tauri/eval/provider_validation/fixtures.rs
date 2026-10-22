use std::collections::BTreeMap;

use antiburn_local::analysis::jev::{JevQuestion, JevRequest};
use serde_json::json;

pub const CASE_ID: &str = "completed-task";

pub fn request(model: &str) -> JevRequest {
    JevRequest {
        model: model.into(),
        state: json!({"text":"The task is to read one configuration file. The assistant read it and reported its contents."}),
        questions: BTreeMap::from([
            (
                "sufficient".into(),
                JevQuestion::Noul {
                    instructions: json!("Was the requested task completed?"),
                    criteria: Some(json!({"true":"Completed","false":"Not completed"})),
                },
            ),
            (
                "scope".into(),
                JevQuestion::Choice {
                    instructions: json!("Classify the work against the task."),
                    criteria: BTreeMap::from([
                        ("within".into(), json!("The work stays within the task.")),
                        ("outside".into(), json!("The work exceeds the task.")),
                        (
                            "unknown".into(),
                            json!("The evidence does not resolve scope."),
                        ),
                    ]),
                },
            ),
        ]),
    }
}
