//! Append-safe leases and atomic pre-alpha OpenCode journal publication.

use antiburn_local::analysis::{OpenCodeJournalCursor, OpenCodeToolResultPage, PARSER_REVISION};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::{PublicationPolicy, ToolResultBatch, publish_coverage_in, reconcile_with_policy};
use crate::store::{Incarnation, SessionKey, Store};

const CONTINUATION_DELAY_SECS: i64 = 1;
const MAX_READ_ATTEMPTS: i64 = 5;

const CANDIDATES: &str = "WITH source_sessions AS (
    SELECT s.*, NOT EXISTS(SELECT 1 FROM session_relation r
        WHERE r.environment_key = s.environment_key AND r.agent = s.agent
          AND r.session_id = s.session_id AND r.kind = 'forkParent') AS main_session
      FROM session s WHERE s.environment_key = 'native' AND s.agent = 'opencode'
        AND s.source_kind = 'providerDb'
), candidates AS (
    SELECT s.environment_key, s.agent, s.session_id, s.incarnation, s.source_label,
           s.cwd, s.source_generation, s.updated_at_epoch,
           d.cursor_json, d.baseline_sequence, d.retry_count, d.observed_generation,
           d.pending, d.blocked, d.lease_expires_at_epoch, d.next_attempt_at_epoch,
           d.last_attempt_at_epoch, d.history_gap, s.main_session,
           (d.incarnation IS NULL OR d.incarnation <> s.incarnation
               OR d.source_label <> s.source_label OR d.cwd IS NOT s.cwd
               OR d.parser_revision <> ?1 OR d.main_session <> s.main_session) AS reset
      FROM source_sessions s LEFT JOIN opencode_journal_delivery d
        ON d.environment_key = s.environment_key AND d.agent = s.agent
       AND d.session_id = s.session_id
     WHERE s.environment_key = 'native' AND s.agent = 'opencode'
       AND s.source_kind = 'providerDb'
)";

#[derive(Debug, Clone)]
pub(crate) struct OpenCodeJournalClaim {
    pub key: SessionKey,
    incarnation: Incarnation,
    source_label: String,
    cwd: Option<String>,
    source_generation: i64,
    claim_fence: i64,
    cursor_json: Option<String>,
    baseline_sequence: Option<u64>,
    history_gap: bool,
    main_session: bool,
    pub(crate) retry_count: i64,
}

impl OpenCodeJournalClaim {
    pub(crate) fn cursor(&self) -> Result<Option<OpenCodeJournalCursor>> {
        self.cursor_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(Into::into)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum JournalFailure {
    SourceChanged,
    IncompleteHistory,
    InvalidCursor,
    Unavailable,
    ReadFailed,
    Cancelled,
}

impl JournalFailure {
    fn label(self) -> &'static str {
        match self {
            Self::SourceChanged => "journal-prefix-changed",
            Self::IncompleteHistory => "journal-history-incomplete",
            Self::InvalidCursor => "journal-cursor-invalid",
            Self::Unavailable => "journal-unavailable",
            Self::ReadFailed => "journal-read-failed",
            Self::Cancelled => "journal-cancelled",
        }
    }

    fn blocked(self) -> bool {
        matches!(self, Self::SourceChanged | Self::InvalidCursor)
    }
}

impl Store {
    pub(crate) fn claim_next_opencode_journal(
        &self,
        now: i64,
        lease_secs: i64,
    ) -> Result<Option<OpenCodeJournalClaim>> {
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidate = tx.query_row(&format!("{CANDIDATES}
            SELECT environment_key, agent, session_id, incarnation, source_label, cwd,
                   source_generation, cursor_json, baseline_sequence,
                   COALESCE(retry_count, 0), reset, observed_generation, history_gap, main_session
              FROM candidates
             WHERE (reset OR (blocked = 0 AND (pending = 1 OR observed_generation <> source_generation)))
               AND (reset OR COALESCE(lease_expires_at_epoch, 0) <= ?2)
               AND (reset OR COALESCE(next_attempt_at_epoch, 0) <= ?2)
             ORDER BY COALESCE(last_attempt_at_epoch, 0), updated_at_epoch DESC, session_id
             LIMIT 1"), params![PARSER_REVISION, now], |row| {
                let reset: bool = row.get(10)?;
                let generation = row.get(6)?;
                Ok(OpenCodeJournalClaim {
                    key: SessionKey::new(row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?),
                    incarnation: Incarnation(row.get(3)?), source_label: row.get(4)?, cwd: row.get(5)?,
                    source_generation: generation, claim_fence: 0,
                    cursor_json: if reset { None } else { row.get(7)? },
                    baseline_sequence: if reset { None } else { row.get(8)? },
                    history_gap: if reset { false } else { row.get(12)? },
                    main_session: row.get(13)?,
                    retry_count: if reset || row.get::<_, Option<i64>>(11)? != Some(generation) { 0 } else { row.get(9)? },
                })
            }).optional()?;
        let Some(mut claim) = candidate else {
            return Ok(None);
        };
        claim.claim_fence = tx.query_row(
            "UPDATE evidence_claim_fence_seq SET value = value + 1 WHERE id = 1 RETURNING value",
            [],
            |row| row.get(0),
        )?;
        tx.execute("INSERT INTO opencode_journal_delivery (
                environment_key, agent, session_id, incarnation, source_label, cwd,
                parser_revision, cursor_json, baseline_sequence, claim_fence,
                lease_expires_at_epoch, last_attempt_at_epoch, retry_count, history_gap, main_session)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(environment_key, agent, session_id) DO UPDATE SET
                incarnation = excluded.incarnation, source_label = excluded.source_label,
                cwd = excluded.cwd, parser_revision = excluded.parser_revision,
                cursor_json = excluded.cursor_json, baseline_sequence = excluded.baseline_sequence,
                pending = 1, blocked = 0, claim_fence = excluded.claim_fence,
                lease_expires_at_epoch = excluded.lease_expires_at_epoch,
                last_attempt_at_epoch = excluded.last_attempt_at_epoch, retry_count = excluded.retry_count,
                history_gap = excluded.history_gap, main_session = excluded.main_session",
            params![claim.key.environment_key, claim.key.agent, claim.key.session_id,
                claim.incarnation.0, claim.source_label, claim.cwd, PARSER_REVISION,
                claim.cursor_json, claim.baseline_sequence, claim.claim_fence,
                now + lease_secs, now, claim.retry_count, claim.history_gap, claim.main_session])?;
        tx.commit()?;
        Ok(Some(claim))
    }

    pub(crate) fn renew_opencode_journal_lease(
        &self,
        claim: &OpenCodeJournalClaim,
        now: i64,
        lease_secs: i64,
    ) -> Result<bool> {
        let mut connection = self.lock();
        let tx = connection.transaction()?;
        if current_generation(&tx, claim, now)?.is_none() {
            return Ok(false);
        }
        tx.execute(
            "UPDATE opencode_journal_delivery SET lease_expires_at_epoch = ?4
            WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3",
            params![
                claim.key.environment_key,
                claim.key.agent,
                claim.key.session_id,
                now + lease_secs
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn publish_opencode_journal_page(
        &self,
        claim: &OpenCodeJournalClaim,
        page: &OpenCodeToolResultPage,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(generation) = current_generation(&tx, claim, now)? else {
            return Ok(false);
        };
        let cursor_json = serde_json::to_string(&page.cursor)?;
        ensure!(
            !page.has_more || claim.cursor_json.as_ref() != Some(&cursor_json),
            "journal page does not advance its cursor"
        );
        let baseline = claim.baseline_sequence.unwrap_or(page.snapshot_end);
        let mut batch = ToolResultBatch {
            source_format: antiburn_local::analysis::SourceFormat::OpenCodeSqliteV2,
            results: page.results.clone(),
            complete: false,
        };
        if !claim.main_session {
            use antiburn_local::analysis::ToolResultCompleteness;
            for result in &mut batch.results {
                if matches!(
                    result.completeness,
                    ToolResultCompleteness::Complete | ToolResultCompleteness::Incomplete
                ) {
                    result.completeness = ToolResultCompleteness::UnsafeOwnership;
                }
                if let Some(context) = &mut result.context {
                    context.main_session = false;
                }
            }
        }
        let history_complete = page.history_complete && !claim.history_gap && claim.main_session;
        let gap_reason = if !claim.main_session {
            Some("journal-main-session-unavailable")
        } else {
            (!history_complete).then_some("journal-history-incomplete")
        };
        reconcile_with_policy(
            &tx,
            &claim.key,
            generation,
            claim.claim_fence,
            &batch,
            PublicationPolicy {
                baseline: Some(baseline),
                history_complete: Some(history_complete),
                gap_reason,
            },
        )?;
        let pending = page.has_more || generation != claim.source_generation;
        let updated = tx.execute(
            "UPDATE opencode_journal_delivery
            SET cursor_json = ?4, baseline_sequence = ?5, observed_generation = ?6,
                pending = ?7, claim_fence = NULL, lease_expires_at_epoch = NULL,
                next_attempt_at_epoch = ?8, retry_count = 0, last_error = ?9, history_gap = ?12
            WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
              AND claim_fence = ?10 AND cursor_json IS ?11",
            params![
                claim.key.environment_key,
                claim.key.agent,
                claim.key.session_id,
                cursor_json,
                baseline,
                claim.source_generation,
                pending,
                pending.then_some(now + CONTINUATION_DELAY_SECS),
                gap_reason,
                claim.claim_fence,
                claim.cursor_json,
                !history_complete
            ],
        )?;
        ensure!(updated == 1, "journal cursor compare-and-swap failed");
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn fail_opencode_journal(
        &self,
        claim: &OpenCodeJournalClaim,
        failure: JournalFailure,
        now: i64,
        retry_at: i64,
    ) -> Result<bool> {
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(generation) = current_generation(&tx, claim, now)? else {
            return Ok(false);
        };
        let cancelled = matches!(failure, JournalFailure::Cancelled);
        let retries = claim.retry_count + i64::from(!cancelled);
        let pending = !failure.blocked() && retries < MAX_READ_ATTEMPTS;
        if !cancelled {
            publish_coverage_in(
                &tx,
                &claim.key,
                generation,
                claim.claim_fence,
                &ToolResultBatch {
                    source_format: antiburn_local::analysis::SourceFormat::OpenCodeSqliteV2,
                    results: Vec::new(),
                    complete: false,
                },
                false,
                Some(failure.label()),
            )?;
        }
        tx.execute(
            "UPDATE opencode_journal_delivery
            SET pending = ?4, blocked = ?5, observed_generation = ?6,
                claim_fence = NULL, lease_expires_at_epoch = NULL,
                next_attempt_at_epoch = ?7, retry_count = ?8, last_error = ?9
            WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3",
            params![
                claim.key.environment_key,
                claim.key.agent,
                claim.key.session_id,
                pending,
                failure.blocked(),
                claim.source_generation,
                pending.then_some(if cancelled {
                    now + CONTINUATION_DELAY_SECS
                } else {
                    retry_at
                }),
                retries,
                failure.label()
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn next_opencode_journal_attempt(&self, now: i64) -> Result<Option<i64>> {
        Ok(self.lock().query_row(&format!("{CANDIDATES}
            SELECT MIN(CASE WHEN reset THEN ?2 ELSE MAX(COALESCE(lease_expires_at_epoch, ?2),
                COALESCE(next_attempt_at_epoch, ?2)) END) FROM candidates
            WHERE reset OR (blocked = 0 AND (pending = 1 OR observed_generation <> source_generation))"),
            params![PARSER_REVISION, now], |row| row.get(0))?)
    }
}

fn current_generation(
    tx: &Transaction<'_>,
    claim: &OpenCodeJournalClaim,
    now: i64,
) -> Result<Option<i64>> {
    Ok(tx.query_row("SELECT s.source_generation FROM opencode_journal_delivery d JOIN session s
        ON s.environment_key = d.environment_key AND s.agent = d.agent AND s.session_id = d.session_id
        WHERE d.environment_key = ?1 AND d.agent = ?2 AND d.session_id = ?3
          AND d.claim_fence = ?4 AND d.cursor_json IS ?5 AND d.lease_expires_at_epoch > ?6
          AND d.incarnation = ?7 AND s.incarnation = ?7
          AND s.source_kind = 'providerDb' AND s.source_label = ?8 AND s.cwd IS ?9
          AND d.source_label = ?8 AND d.cwd IS ?9 AND d.parser_revision = ?10
          AND d.main_session = ?11 AND ?11 = (NOT EXISTS (
              SELECT 1 FROM session_relation r WHERE r.environment_key = s.environment_key
                AND r.agent = s.agent AND r.session_id = s.session_id AND r.kind = 'forkParent'))",
        params![claim.key.environment_key, claim.key.agent, claim.key.session_id,
            claim.claim_fence, claim.cursor_json, now, claim.incarnation.0,
            claim.source_label, claim.cwd, PARSER_REVISION, claim.main_session], |row| row.get(0)).optional()?)
}

#[cfg(test)]
mod tests;
