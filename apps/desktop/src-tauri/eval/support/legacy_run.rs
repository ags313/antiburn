use super::provider::EvalClient;
use antiburn_local::analysis::jev::{JevError, JevRequestBatch, JevResponse};
use std::sync::{Arc, Mutex};

#[path = "run.rs"]
mod request_run;
pub(crate) use request_run::{RunUsage, evaluate_request, measurements, stop_reason};

pub(crate) async fn evaluate_batch(
    client: &EvalClient,
    usage: &Arc<Mutex<RunUsage>>,
    case_id: &str,
    stage: &str,
    batch: &JevRequestBatch,
) -> Result<JevResponse, JevError> {
    request_run::evaluate(client, usage, case_id, stage, &batch.request, Some(batch)).await
}
