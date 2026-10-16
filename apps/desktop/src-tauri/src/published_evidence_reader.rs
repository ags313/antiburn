//! Adapter from Store publication events to the checks source contract.

use antiburn_checks::domain::NativeOrderKey;
use antiburn_checks::source::{
    PublicationCursor, PublicationRead, PublishedContext, PublishedEvidenceBatch,
    PublishedEvidenceChange, PublishedEvidenceEvent, PublishedEvidenceReader,
    PublishedResultStatus, PublishedToolResult, SourceError,
};

use sha2::{Digest, Sha256};

use crate::store::native_tool_results::ToolResultPublicationOutcome;
use crate::store::{SessionKey, Store, native_tool_results::ToolResultPublication};

/// Reads committed native-result events from the Store outbox.
///
/// Context comes from the immutable payload, including root-session admission.
pub struct StorePublishedEvidenceReader<'a> {
    store: &'a Store,
    project: Option<(&'a str, &'a str)>,
    active_since: Option<i64>,
    repository_root: Option<&'a str>,
}

impl<'a> StorePublishedEvidenceReader<'a> {
    /// Binds the reader to the application's Store.
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            project: None,
            active_since: None,
            repository_root: None,
        }
    }

    /// Binds runtime reads to one exact stored project and environment.
    pub fn for_project(store: &'a Store, environment_key: &'a str, project_cwd: &'a str) -> Self {
        Self {
            store,
            project: Some((environment_key, project_cwd)),
            active_since: None,
            repository_root: None,
        }
    }

    /// Admits new source generations from active sessions after their first snapshot.
    pub fn for_live_project(
        store: &'a Store,
        environment_key: &'a str,
        project_cwd: &'a str,
        active_since: i64,
    ) -> Self {
        Self {
            active_since: Some(active_since),
            ..Self::for_project(store, environment_key, project_cwd)
        }
    }

    /// Sets the resolved repository root without changing the exact source scope.
    pub fn with_repository_root(mut self, root: Option<&'a str>) -> Self {
        self.repository_root = root;
        self
    }

    fn live_publication(
        &self,
        publication: &ToolResultPublication,
        active_since: i64,
    ) -> Result<bool, SourceError> {
        self.store
            .lock()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM session s
             WHERE s.environment_key = ?1 AND s.agent = ?2 AND s.session_id = ?3
                AND COALESCE(?6, s.updated_at_epoch >= ?4 AND ?5 > (SELECT MIN(p.source_generation) FROM native_tool_result_publication p
                    WHERE p.environment_key = ?1 AND p.agent = ?2 AND p.session_id = ?3)))",
                rusqlite::params![
                    publication.session.environment_key,
                    publication.session.agent,
                    publication.session.session_id,
                    active_since,
                    publication.source_generation,
                    publication.live_eligible,
                ],
                |row| row.get(0),
            )
            .map_err(|error| SourceError::Reader(error.to_string()))
    }
}

impl PublishedEvidenceReader for StorePublishedEvidenceReader<'_> {
    fn read_batch(&self, request: PublicationRead) -> Result<PublishedEvidenceBatch, SourceError> {
        let after = i64::try_from(request.after.0)
            .map_err(|error| SourceError::Reader(error.to_string()))?;
        let read = |after, limit| match self.project {
            Some((environment, project)) => self
                .store
                .native_tool_result_publications_for_project_after(
                    environment,
                    project,
                    after,
                    limit,
                ),
            None => self
                .store
                .native_tool_result_publications_after(after, limit),
        };
        let publications =
            read(after, request.limit).map_err(|error| SourceError::Reader(error.to_string()))?;
        let next = match publications.last() {
            Some(publication) => PublicationCursor(
                u64::try_from(publication.publication_sequence)
                    .map_err(|error| SourceError::Reader(error.to_string()))?,
            ),
            None => request.after,
        };
        let next_sequence =
            i64::try_from(next.0).map_err(|error| SourceError::Reader(error.to_string()))?;
        let has_more = !read(next_sequence, 1)
            .map_err(|error| SourceError::Reader(error.to_string()))?
            .is_empty();
        let events = publications
            .into_iter()
            .map(|publication| {
                let ignored = if let Some(active_since) = self.active_since {
                    publication.outcome == ToolResultPublicationOutcome::Update
                        && !self.live_publication(&publication, active_since)?
                } else {
                    false
                };
                let mut event = publication_event(publication, self.repository_root)?;
                if ignored {
                    event.change = PublishedEvidenceChange::Ignored;
                }
                Ok(event)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PublishedEvidenceBatch {
            events,
            next,
            has_more,
        })
    }
}

fn publication_event(
    publication: ToolResultPublication,
    project_root: Option<&str>,
) -> Result<PublishedEvidenceEvent, SourceError> {
    let cursor = PublicationCursor(
        u64::try_from(publication.publication_sequence)
            .map_err(|error| SourceError::Reader(error.to_string()))?,
    );
    let branch = publication.session.to_identity_json();
    let context = publication
        .payload
        .as_ref()
        .and_then(|result| result.context.as_ref())
        .map(|context| published_context(context, &publication.session, project_root))
        .transpose()?;
    let change = match publication.outcome {
        ToolResultPublicationOutcome::Update => {
            let result = publication
                .payload
                .ok_or_else(|| SourceError::Reader("update is missing its payload".to_owned()))?;
            let selection_safe =
                result.admission() == antiburn_local::analysis::ToolResultAdmission::Admitted;
            PublishedEvidenceChange::Upsert(PublishedToolResult {
                identity: publication.native_identity.ok_or_else(|| {
                    SourceError::Reader("update is missing its stable identity".to_owned())
                })?,
                native_order: NativeOrderKey(
                    serde_json::to_string(
                        result
                            .context
                            .as_ref()
                            .map_or(&result.position, |context| &context.result_position),
                    )
                    .map_err(|error| SourceError::Reader(error.to_string()))?,
                ),
                revision: result.revision,
                tool_name: result.tool_name,
                category: result.tool_category.map(category_name),
                arguments: result.arguments,
                content: result.content,
                status: match result.status {
                    antiburn_local::analysis::ToolResultStatus::Success => {
                        PublishedResultStatus::Completed
                    }
                    antiburn_local::analysis::ToolResultStatus::Error => {
                        PublishedResultStatus::Error
                    }
                    antiburn_local::analysis::ToolResultStatus::Interrupted => {
                        PublishedResultStatus::Interrupted
                    }
                    antiburn_local::analysis::ToolResultStatus::Unknown => {
                        PublishedResultStatus::Unknown
                    }
                },
                finality: result.finality,
                complete: result.finality
                    && result.completeness
                        == antiburn_local::analysis::ToolResultCompleteness::Complete,
                selection_safe,
            })
        }
        ToolResultPublicationOutcome::Removal => PublishedEvidenceChange::Removal {
            identity: publication
                .native_identity
                .ok_or_else(|| SourceError::Reader("removal is missing its identity".to_owned()))?,
            revision: u64::try_from(publication.native_revision.ok_or_else(|| {
                SourceError::Reader("removal is missing its revision".to_owned())
            })?)
            .map_err(|error| SourceError::Reader(error.to_string()))?,
        },
        ToolResultPublicationOutcome::CoverageGap => PublishedEvidenceChange::Gap {
            reason: publication
                .gap_reason
                .unwrap_or_else(|| "published source membership is incomplete".to_owned()),
        },
        ToolResultPublicationOutcome::CoverageRestored => PublishedEvidenceChange::CoverageRestored,
    };
    Ok(PublishedEvidenceEvent {
        cursor,
        branch,
        context,
        change,
    })
}

fn published_context(
    context: &antiburn_local::analysis::ToolResultContext,
    session: &SessionKey,
    project_root: Option<&str>,
) -> Result<PublishedContext, SourceError> {
    use antiburn_local::analysis::{UserMessageChange, UserMessageCompleteness};

    let encoded =
        serde_json::to_vec(context).map_err(|error| SourceError::Reader(error.to_string()))?;
    let complete = context.complete_through_event
        && context.main_session
        && context.user_messages.iter().all(|message| {
            message.is_bounded()
                && message.session_id == session.session_id
                && message.change == UserMessageChange::Present
                && message.completeness == UserMessageCompleteness::Complete
        });
    Ok(PublishedContext {
        requests: context.user_messages.iter().map(|message| {
            (message.message_id.clone(), serde_json::json!({
                "source": session.agent,
                "kind": "user_message",
                "text": message.parts.iter().map(|part| part.text.as_str()).collect::<Vec<_>>().join("\n"),
            }))
        }).collect(),
        session_id: session.session_id.clone(),
        agent_name: session.agent.clone(),
        project_root: project_root.map(str::to_owned),
        version: Sha256::digest(encoded).iter().map(|byte| format!("{byte:02x}")).collect(),
        main_session: context.main_session,
        complete_through_event: complete,
    })
}

fn category_name(category: antiburn_local::analysis::ToolCategory) -> String {
    use antiburn_local::analysis::ToolCategory;
    match category {
        ToolCategory::Bash | ToolCategory::Test => "shell",
        ToolCategory::Edit => "edit",
        ToolCategory::Read => "read",
        ToolCategory::Search => "search",
        ToolCategory::Other => "other",
    }
    .to_owned()
}
