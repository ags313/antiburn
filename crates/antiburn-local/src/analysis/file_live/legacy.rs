//! Convert only the named file cursor revision 4 without replaying its progress.

use super::*;

#[derive(Serialize, Deserialize)]
struct CursorV4 {
    revision: u32,
    agent: String,
    session_id: String,
    source_format: SourceFormat,
    anchor: NativeFileAnchor,
    baseline_offset: u64,
    start_offset: u64,
    next_record_index: u64,
    #[serde(with = "framer_codec")]
    framer: LiveJsonlFramer,
    adapter: AdapterV4,
    requests: VecDeque<NativeFileRequest>,
    request_ids: IdentityFilter,
    request_bytes: usize,
    history_complete: bool,
    pending_results: Option<PendingResults>,
}

#[derive(Serialize, Deserialize)]
enum AdapterV4 {
    Claude(Box<claude::live::StateV4>),
    Codex(Box<codex::live::StateV4>),
    Pi(Box<pi::live::StateV4>),
}

impl CursorV4 {
    fn convert(self) -> anyhow::Result<NativeFileCursor> {
        anyhow::ensure!(self.revision == 4, NativeFileReadError::InvalidCursor);
        let evidence_baseline_offset = self.anchor.boundary;
        let admission_failure = self.pending_results.as_ref().and_then(|pending| {
            pending
                .observations_json
                .iter()
                .filter_map(|json| serde_json::from_str::<ToolResultObservation>(json).ok())
                .filter(|result| {
                    // A legacy non-main result can reflect missing history, not known ownership loss.
                    result.source_format == self.source_format
                        && result.session_id == self.session_id
                        && (result.completeness
                            != super::super::ToolResultCompleteness::UnsafeOwnership
                            || pending.main_session)
                })
                .find_map(|result| {
                    super::super::public_events::unsafe_reason(result.completeness).then(|| {
                        NativeAdmissionFailure {
                            source_format: self.source_format,
                            session_id: self.session_id.clone(),
                            position: super::super::ToolResultPosition::FileBytes {
                                record_start: pending.position.record_start,
                                record_end: pending.position.record_end,
                                item_index: u32::MAX,
                            },
                            reason: result.completeness,
                        }
                    })
                })
        });
        let mut cursor = NativeFileCursor {
            revision: NATIVE_FILE_CURSOR_REVISION,
            agent: self.agent,
            session_id: self.session_id,
            source_format: self.source_format,
            anchor: self.anchor,
            baseline_offset: self.baseline_offset,
            start_offset: self.start_offset,
            next_record_index: self.next_record_index,
            framer: self.framer,
            adapter: match self.adapter {
                AdapterV4::Claude(state) => {
                    Adapter::Claude(Box::new(state.convert(!self.history_complete)))
                }
                AdapterV4::Codex(state) => Adapter::Codex(Box::new(state.convert())),
                AdapterV4::Pi(state) => {
                    Adapter::Pi(Box::new(state.convert(!self.history_complete)))
                }
            },
            requests: self.requests,
            request_ids: self.request_ids,
            request_bytes: self.request_bytes,
            history_complete: self.history_complete,
            pending_results: self.pending_results,
            evidence_baseline_offset,
            pending_evidence: JsonState::default(),
            admission_failure: JsonState(admission_failure.clone()),
        };
        if let Some(failure) = admission_failure {
            cursor.requests.clear();
            cursor.request_bytes = 0;
            cursor.pending_evidence.0.push_back(NativeEvidenceEvent {
                fact: NativeEvidenceFact::AdmissionFailure(failure),
                live_eligible: true,
            });
        }
        anyhow::ensure!(cursor.valid(), NativeFileReadError::InvalidCursor);
        Ok(cursor)
    }
}

impl NativeFileCursor {
    pub fn convert_v4(bytes: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            bytes.len() <= 32 * 1024 * 1024,
            NativeFileReadError::InvalidCursor
        );
        let (legacy, remaining): (CursorV4, _) =
            postcard::take_from_bytes(bytes).map_err(|_| NativeFileReadError::InvalidCursor)?;
        anyhow::ensure!(remaining.is_empty(), NativeFileReadError::InvalidCursor);
        legacy.convert()
    }

    pub fn convert_v4_json(bytes: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            NativeFileReadError::InvalidCursor
        );
        serde_json::from_slice::<CursorV4>(bytes)
            .map_err(|_| NativeFileReadError::InvalidCursor)?
            .convert()
    }
}

#[cfg(test)]
pub(super) fn encode_v4(cursor: &NativeFileCursor) -> (Vec<u8>, Vec<u8>) {
    let old = CursorV4 {
        revision: 4,
        agent: cursor.agent.clone(),
        session_id: cursor.session_id.clone(),
        source_format: cursor.source_format,
        anchor: cursor.anchor.clone(),
        baseline_offset: cursor.baseline_offset,
        start_offset: cursor.start_offset,
        next_record_index: cursor.next_record_index,
        framer: cursor.framer.clone(),
        adapter: match &cursor.adapter {
            Adapter::Claude(state) => AdapterV4::Claude(Box::new(state.as_v4())),
            Adapter::Codex(state) => AdapterV4::Codex(Box::new(state.as_v4())),
            Adapter::Pi(state) => AdapterV4::Pi(Box::new(state.as_v4())),
        },
        requests: cursor.requests.clone(),
        request_ids: cursor.request_ids.clone(),
        request_bytes: cursor.request_bytes,
        history_complete: cursor.history_complete,
        pending_results: cursor.pending_results.clone(),
    };
    (
        postcard::to_allocvec(&old).unwrap(),
        serde_json::to_vec(&old).unwrap(),
    )
}
