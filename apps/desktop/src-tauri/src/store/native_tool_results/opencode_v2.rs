//! Fenced capture progress for OpenCode V2 event sessions.

use antiburn_local::analysis::{SourceFormat, ToolResultObservation, ToolResultPosition};
use antiburn_local::discovery::ACTIVE_SESSION_WINDOW_SECS;
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::publications::{
    PublicationPolicy, ToolResultBatch, publish_coverage_in, reconcile_with_policy,
};
use crate::store::native_tool_results::opencode_v2_status::OpenCodeV2SourceStatus;
use crate::store::{Incarnation, SessionKey, SessionRecord, Store};

const MAX_CAPTURE_STARTS: usize = 256;
const MAX_EPOCH_BYTES: usize = 128;
const MAX_GAP_REASON_BYTES: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct OpenCodeV2CaptureStart {
    pub key: SessionKey,
    pub incarnation: Incarnation,
    pub source_generation: i64,
    pub source_label: String,
    pub cwd: String,
    pub baseline_sequence: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct OpenCodeV2CaptureClaim {
    pub key: SessionKey,
    pub incarnation: Incarnation,
    pub source_generation: i64,
    pub source_label: String,
    pub cwd: String,
    pub capture_epoch: String,
    pub claim_fence: i64,
    pub baseline_sequence: Option<u64>,
    pub last_sequence: Option<u64>,
    pub history_gap: bool,
}

impl Store {
    /// Clears capture leases left by a worker that stopped.
    pub(crate) fn recover_opencode_v2_capture_claims(&self, now: i64) -> Result<usize> {
        ensure!(now >= 0, "V2 capture recovery time must be non-negative");
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let claims = {
            let mut statement = tx.prepare(
                "SELECT environment_key, agent, session_id, incarnation, source_generation,
                        source_label, cwd, capture_epoch, claim_fence, baseline_sequence,
                        last_sequence, history_gap, lease_expires_at_epoch
                   FROM opencode_v2_capture
                  WHERE lease_expires_at_epoch IS NOT NULL
                  ORDER BY environment_key, agent, session_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        OpenCodeV2CaptureClaim {
                            key: SessionKey::new(
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ),
                            incarnation: Incarnation(row.get(3)?),
                            source_generation: row.get(4)?,
                            source_label: row.get(5)?,
                            cwd: row.get(6)?,
                            capture_epoch: row.get(7)?,
                            claim_fence: row.get(8)?,
                            baseline_sequence: row
                                .get::<_, Option<i64>>(9)?
                                .map(|value| value as u64),
                            last_sequence: row.get::<_, Option<i64>>(10)?.map(|value| value as u64),
                            history_gap: row.get(11)?,
                        },
                        row.get::<_, i64>(12)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (claim, lease_expires_at_epoch) in &claims {
            if capture_session_matches(&tx, claim)? {
                publish_gap(
                    &tx,
                    claim,
                    if *lease_expires_at_epoch <= now {
                        "capture-expired"
                    } else {
                        "capture-recovered"
                    },
                )?;
            }
            tx.execute(
                "UPDATE opencode_v2_capture SET history_gap = 1,
                        lease_expires_at_epoch = NULL
                  WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
                    AND capture_epoch = ?4 AND claim_fence = ?5
                    AND lease_expires_at_epoch IS NOT NULL",
                params![
                    claim.key.environment_key,
                    claim.key.agent,
                    claim.key.session_id,
                    claim.capture_epoch,
                    claim.claim_fence
                ],
            )?;
        }
        tx.commit()?;
        Ok(claims.len())
    }

    /// Returns recent or previously admitted V2 sessions in an enabled Live scope.
    pub(crate) fn opencode_v2_live_candidates(
        &self,
        now: i64,
        limit: usize,
    ) -> Result<Vec<SessionRecord>> {
        ensure!(limit <= 1024, "candidate limit exceeds 1024");
        if limit == 0 {
            return Ok(Vec::new());
        }
        let connection = self.lock();
        let mut statement = connection.prepare(
            "SELECT s.environment_key, s.agent, s.session_id, s.source_kind,
                    s.source_label, s.wsl_distro, s.title, s.title_source, s.cwd,
                    s.surface, s.updated_at_epoch, s.activity_cursor,
                    s.activity_source, s.subagent_count, s.source_fingerprint
               FROM session s
              WHERE s.environment_key = 'native' AND s.agent = 'opencode'
                AND s.source_kind = 'providerDb' AND s.wsl_distro IS NULL
                AND s.source_label = 'opencode:' || s.session_id || ':open_code_v2_sqlite'
                AND s.cwd IS NOT NULL AND s.cwd <> ''
                AND (s.updated_at_epoch >= ?1 - ?2 OR EXISTS (
                    SELECT 1 FROM opencode_v2_capture d
                     WHERE d.environment_key = s.environment_key AND d.agent = s.agent
                       AND d.session_id = s.session_id AND d.incarnation = s.incarnation
                        AND d.source_label = s.source_label AND d.cwd = s.cwd
                        AND d.capture_epoch <> '' AND d.claim_fence > 0))
                AND NOT EXISTS (
                    SELECT 1 FROM session_relation r
                     WHERE r.environment_key = s.environment_key AND r.agent = s.agent
                       AND ((r.session_id = s.session_id AND r.kind = 'forkParent')
                         OR (r.related_id = s.session_id AND r.kind = 'subagent')))
                AND EXISTS (
                    SELECT 1 FROM burn_check_activation a
                     WHERE a.enabled = 1
                       AND json_extract(a.activation_scope_key, '$.environment') = s.environment_key
                       AND json_extract(a.activation_scope_key, '$.session_cwd') = s.cwd
                       AND json_extract(a.activation_scope_key, '$.run') = 'Live')
              ORDER BY s.updated_at_epoch DESC, s.session_id ASC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![now, ACTIVE_SESSION_WINDOW_SECS, limit as i64],
            |row| {
                Ok(SessionRecord {
                    key: SessionKey::new(
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ),
                    source_kind: row.get(3)?,
                    source_label: row.get(4)?,
                    wsl_distro: row.get(5)?,
                    title: row.get(6)?,
                    title_source: row.get(7)?,
                    cwd: row.get(8)?,
                    surface: row.get(9)?,
                    updated_at_epoch: row.get(10)?,
                    activity_cursor: row.get(11)?,
                    activity_source: row.get(12)?,
                    subagent_count: row.get(13)?,
                    fork_parent_session_id: None,
                    source_fingerprint: row.get(14)?,
                })
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub(crate) fn start_opencode_v2_capture_epoch(
        &self,
        starts: &[OpenCodeV2CaptureStart],
        capture_epoch: &str,
        now: i64,
        lease_secs: i64,
    ) -> Result<Option<Vec<OpenCodeV2CaptureClaim>>> {
        ensure!(
            !starts.is_empty() && starts.len() <= MAX_CAPTURE_STARTS,
            "invalid V2 capture set size"
        );
        ensure!(
            !capture_epoch.is_empty() && capture_epoch.len() <= MAX_EPOCH_BYTES,
            "invalid V2 capture epoch"
        );
        ensure!(lease_secs > 0, "V2 capture lease must be positive");
        let lease_expiry = now
            .checked_add(lease_secs)
            .context("V2 capture lease overflow")?;
        ensure!(starts.iter().all(valid_start), "invalid V2 capture start");
        let mut seen = std::collections::HashSet::new();
        ensure!(
            starts.iter().all(|start| seen.insert(start.key.clone())),
            "duplicate V2 capture session"
        );

        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for start in starts {
            if !scope_matches(
                &tx,
                &start.key,
                start.incarnation,
                Some(start.source_generation),
                &start.source_label,
                &start.cwd,
            )? || tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM opencode_v2_capture
                  WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
                    AND incarnation = ?4 AND source_label = ?5 AND cwd = ?6
                    AND lease_expires_at_epoch > ?7)",
                params![
                    start.key.environment_key,
                    start.key.agent,
                    start.key.session_id,
                    start.incarnation.0,
                    start.source_label,
                    start.cwd,
                    now,
                ],
                |row| row.get::<_, bool>(0),
            )? {
                return Ok(None);
            }
        }
        let mut claims = Vec::with_capacity(starts.len());
        for start in starts {
            let previous_fence: Option<i64> = tx
                .query_row(
                    "SELECT claim_fence FROM opencode_v2_capture
                      WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3",
                    params![
                        start.key.environment_key,
                        start.key.agent,
                        start.key.session_id
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            let fence = previous_fence
                .unwrap_or(0)
                .checked_add(1)
                .context("V2 capture fence overflow")?;
            let baseline = sqlite_sequence(start.baseline_sequence)?;
            tx.execute(
                "INSERT INTO opencode_v2_capture (
                    environment_key, agent, session_id, incarnation, source_generation,
                    source_label, cwd, capture_epoch, claim_fence, baseline_sequence,
                    last_sequence, history_gap, lease_expires_at_epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 1, ?11)
                 ON CONFLICT(environment_key, agent, session_id) DO UPDATE SET
                    incarnation = excluded.incarnation,
                    source_generation = excluded.source_generation,
                    source_label = excluded.source_label, cwd = excluded.cwd,
                    capture_epoch = excluded.capture_epoch, claim_fence = excluded.claim_fence,
                    baseline_sequence = excluded.baseline_sequence,
                    last_sequence = excluded.last_sequence, history_gap = 1,
                    lease_expires_at_epoch = excluded.lease_expires_at_epoch",
                params![
                    start.key.environment_key,
                    start.key.agent,
                    start.key.session_id,
                    start.incarnation.0,
                    start.source_generation,
                    start.source_label,
                    start.cwd,
                    capture_epoch,
                    fence,
                    baseline,
                    lease_expiry
                ],
            )?;
            let claim = OpenCodeV2CaptureClaim {
                key: start.key.clone(),
                incarnation: start.incarnation,
                source_generation: start.source_generation,
                source_label: start.source_label.clone(),
                cwd: start.cwd.clone(),
                capture_epoch: capture_epoch.to_owned(),
                claim_fence: fence,
                baseline_sequence: start.baseline_sequence,
                last_sequence: start.baseline_sequence,
                history_gap: true,
            };
            publish_gap(&tx, &claim, "cold-attach")?;
            save_capture_status(&tx, &claim, OpenCodeV2SourceStatus::Live, now)?;
            claims.push(claim);
        }
        tx.commit()?;
        Ok(Some(claims))
    }

    pub(crate) fn renew_opencode_v2_capture_epoch(
        &self,
        claims: &[OpenCodeV2CaptureClaim],
        now: i64,
        lease_secs: i64,
    ) -> Result<bool> {
        ensure!(lease_secs > 0, "V2 capture lease must be positive");
        let lease_expiry = now
            .checked_add(lease_secs)
            .context("V2 capture lease overflow")?;
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !claims_valid(&tx, claims, now, true)? {
            return Ok(false);
        }
        for claim in claims {
            tx.execute(
                "UPDATE opencode_v2_capture SET lease_expires_at_epoch = ?5
                  WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
                    AND capture_epoch = ?4 AND claim_fence = ?6",
                params![
                    claim.key.environment_key,
                    claim.key.agent,
                    claim.key.session_id,
                    claim.capture_epoch,
                    lease_expiry,
                    claim.claim_fence
                ],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn advance_opencode_v2_capture(
        &self,
        claim: &OpenCodeV2CaptureClaim,
        sequence: u64,
        result: Option<&ToolResultObservation>,
        now: i64,
        lease_secs: i64,
    ) -> Result<bool> {
        ensure!(lease_secs > 0, "V2 capture lease must be positive");
        let lease_expiry = now
            .checked_add(lease_secs)
            .context("V2 capture lease overflow")?;
        let sequence_sql = sqlite_sequence(Some(sequence))?.context("missing V2 event sequence")?;
        if result.is_some_and(|_| {
            claim
                .baseline_sequence
                .is_some_and(|baseline| sequence <= baseline)
        }) {
            return Ok(false);
        }
        if let Some(result) = result
            && (result.source_format != SourceFormat::OpenCodeSqliteV2
                || !matches!(result.position, ToolResultPosition::OpenCodeV2Event { sequence: n, .. } if n == sequence)
                || result.session_id != claim.key.session_id)
        {
            return Ok(false);
        }
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !claims_valid(&tx, std::slice::from_ref(claim), now, true)? {
            return Ok(false);
        }
        let (baseline_sequence, last_sequence): (Option<i64>, Option<i64>) = tx.query_row(
            "SELECT baseline_sequence, last_sequence FROM opencode_v2_capture
              WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
                AND capture_epoch = ?4 AND claim_fence = ?5",
            params![
                claim.key.environment_key,
                claim.key.agent,
                claim.key.session_id,
                claim.capture_epoch,
                claim.claim_fence
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if baseline_sequence != sqlite_sequence(claim.baseline_sequence)?
            || sequence_sql <= baseline_sequence.unwrap_or(-1)
            || last_sequence.is_some_and(|last| sequence_sql <= last)
        {
            return Ok(false);
        }
        tx.execute(
            "UPDATE opencode_v2_capture SET last_sequence = ?6,
                    lease_expires_at_epoch = ?7
              WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
                AND capture_epoch = ?4 AND claim_fence = ?5",
            params![
                claim.key.environment_key,
                claim.key.agent,
                claim.key.session_id,
                claim.capture_epoch,
                claim.claim_fence,
                sequence_sql,
                lease_expiry
            ],
        )?;
        if let Some(result) = result {
            let batch = ToolResultBatch {
                source_format: SourceFormat::OpenCodeSqliteV2,
                results: vec![result.clone()],
                complete: false,
            };
            let complete = result
                .context
                .as_ref()
                .is_some_and(|context| context.main_session && context.complete_through_event);
            reconcile_with_policy(
                &tx,
                &claim.key,
                current_source_generation(&tx, &claim.key)?,
                claim.claim_fence,
                &batch,
                PublicationPolicy {
                    baseline: claim.baseline_sequence,
                    history_complete: Some(complete),
                    gap_reason: (!complete).then_some("v2-history-incomplete"),
                    live_eligible: Some(true),
                    ..PublicationPolicy::default()
                },
            )?;
            tx.execute(
                "UPDATE opencode_v2_capture SET history_gap = ?6
                      WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
                        AND capture_epoch = ?4 AND claim_fence = ?5",
                params![
                    claim.key.environment_key,
                    claim.key.agent,
                    claim.key.session_id,
                    claim.capture_epoch,
                    claim.claim_fence,
                    i64::from(!complete)
                ],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn mark_opencode_v2_capture_gap(
        &self,
        claims: &[OpenCodeV2CaptureClaim],
        reason: &str,
        now: i64,
    ) -> Result<usize> {
        ensure!(valid_gap_reason(reason), "invalid V2 capture gap reason");
        let mut connection = self.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut released = 0;
        for claim in claims {
            if !capture_claims_match(&tx, std::slice::from_ref(claim), now, false)? {
                continue;
            }
            if capture_session_matches(&tx, claim)? {
                publish_gap(&tx, claim, reason)?;
                let status = match reason {
                    "event-version-unsupported" => OpenCodeV2SourceStatus::VersionUnsupported,
                    "source-limit" | "recovery-capacity" => OpenCodeV2SourceStatus::SourceLimit,
                    _ => OpenCodeV2SourceStatus::CaptureGap,
                };
                save_capture_status(&tx, claim, status, now)?;
            }
            tx.execute(
                "UPDATE opencode_v2_capture SET history_gap = 1,
                        lease_expires_at_epoch = NULL
                  WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
                    AND capture_epoch = ?4 AND claim_fence = ?5",
                params![
                    claim.key.environment_key,
                    claim.key.agent,
                    claim.key.session_id,
                    claim.capture_epoch,
                    claim.claim_fence
                ],
            )?;
            released += 1;
        }
        tx.commit()?;
        Ok(released)
    }
}

fn valid_start(start: &OpenCodeV2CaptureStart) -> bool {
    start.key.environment_key == "native"
        && start.key.agent == "opencode"
        && start.source_label == format!("opencode:{}:open_code_v2_sqlite", start.key.session_id)
        && !start.cwd.is_empty()
        && start.cwd.len() <= 4096
        && start.source_generation >= 0
        && start
            .baseline_sequence
            .is_none_or(|sequence| i64::try_from(sequence).is_ok())
}

fn sqlite_sequence(sequence: Option<u64>) -> Result<Option<i64>> {
    sequence
        .map(|value| i64::try_from(value).context("V2 sequence exceeds SQLite range"))
        .transpose()
}

fn scope_matches(
    tx: &Transaction<'_>,
    key: &SessionKey,
    incarnation: Incarnation,
    generation: Option<i64>,
    source_label: &str,
    cwd: &str,
) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM session s
          WHERE s.environment_key = ?1 AND s.agent = ?2 AND s.session_id = ?3
             AND s.incarnation = ?4 AND (?5 IS NULL OR s.source_generation = ?5)
            AND s.source_kind = 'providerDb' AND s.source_label = ?6 AND s.cwd = ?7
            AND s.cwd <> '' AND s.wsl_distro IS NULL
            AND NOT EXISTS (SELECT 1 FROM session_relation r
              WHERE r.environment_key = s.environment_key AND r.agent = s.agent
                AND ((r.session_id = s.session_id AND r.kind = 'forkParent')
                  OR (r.related_id = s.session_id AND r.kind = 'subagent')))
            AND EXISTS (SELECT 1 FROM burn_check_activation a WHERE a.enabled = 1
              AND json_extract(a.activation_scope_key, '$.environment') = s.environment_key
              AND json_extract(a.activation_scope_key, '$.session_cwd') = s.cwd
              AND json_extract(a.activation_scope_key, '$.run') = 'Live'))",
        params![
            key.environment_key,
            key.agent,
            key.session_id,
            incarnation.0,
            generation,
            source_label,
            cwd
        ],
        |row| row.get(0),
    )?)
}

fn claims_valid(
    tx: &Transaction<'_>,
    claims: &[OpenCodeV2CaptureClaim],
    now: i64,
    require_lease: bool,
) -> Result<bool> {
    if !capture_claims_match(tx, claims, now, require_lease)? {
        return Ok(false);
    }
    for claim in claims {
        if !scope_matches(
            tx,
            &claim.key,
            claim.incarnation,
            None,
            &claim.source_label,
            &claim.cwd,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn capture_claims_match(
    tx: &Transaction<'_>,
    claims: &[OpenCodeV2CaptureClaim],
    now: i64,
    require_lease: bool,
) -> Result<bool> {
    if claims.is_empty() || claims.len() > MAX_CAPTURE_STARTS {
        return Ok(false);
    }
    let mut seen = std::collections::HashSet::new();
    for claim in claims {
        if !seen.insert(claim.key.clone()) {
            return Ok(false);
        }
        let valid: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM opencode_v2_capture
              WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3
                AND incarnation = ?4 AND source_generation = ?5 AND source_label = ?6
                AND cwd = ?7 AND capture_epoch = ?8 AND claim_fence = ?9
                AND (?10 = 0 OR lease_expires_at_epoch > ?11))",
            params![
                claim.key.environment_key,
                claim.key.agent,
                claim.key.session_id,
                claim.incarnation.0,
                claim.source_generation,
                claim.source_label,
                claim.cwd,
                claim.capture_epoch,
                claim.claim_fence,
                i64::from(require_lease),
                now
            ],
            |row| row.get(0),
        )?;
        if !valid {
            return Ok(false);
        }
    }
    Ok(true)
}

fn capture_session_matches(tx: &Transaction<'_>, claim: &OpenCodeV2CaptureClaim) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM session s
          WHERE s.environment_key = ?1 AND s.agent = ?2 AND s.session_id = ?3
             AND s.incarnation = ?4
             AND s.source_kind = 'providerDb' AND s.source_label = ?5 AND s.cwd = ?6
            AND s.wsl_distro IS NULL)",
        params![
            claim.key.environment_key,
            claim.key.agent,
            claim.key.session_id,
            claim.incarnation.0,
            claim.source_label,
            claim.cwd
        ],
        |row| row.get(0),
    )?)
}

fn publish_gap(tx: &Transaction<'_>, claim: &OpenCodeV2CaptureClaim, reason: &str) -> Result<()> {
    let batch = ToolResultBatch {
        source_format: SourceFormat::OpenCodeSqliteV2,
        results: Vec::new(),
        complete: false,
    };
    publish_coverage_in(
        tx,
        &claim.key,
        current_source_generation(tx, &claim.key)?,
        claim.claim_fence,
        &batch,
        false,
        Some(reason),
    )
}

fn current_source_generation(tx: &Transaction<'_>, key: &SessionKey) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT source_generation FROM session
          WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3",
        params![key.environment_key, key.agent, key.session_id],
        |row| row.get(0),
    )?)
}

fn valid_gap_reason(reason: &str) -> bool {
    !reason.is_empty()
        && reason.len() <= MAX_GAP_REASON_BYTES
        && reason
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn save_capture_status(
    tx: &Transaction<'_>,
    claim: &OpenCodeV2CaptureClaim,
    status: OpenCodeV2SourceStatus,
    now: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO opencode_v2_source_status
          (environment_key, agent, session_id, incarnation, source_generation,
           source_label, cwd, code, observed_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(environment_key, agent, session_id) DO UPDATE SET
           incarnation = excluded.incarnation, source_generation = excluded.source_generation,
           source_label = excluded.source_label, cwd = excluded.cwd,
           code = excluded.code, observed_at_epoch = excluded.observed_at_epoch",
        params![
            claim.key.environment_key,
            claim.key.agent,
            claim.key.session_id,
            claim.incarnation.0,
            current_source_generation(tx, &claim.key)?,
            claim.source_label,
            claim.cwd,
            status.code(),
            now
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "opencode_v2_tests.rs"]
mod tests;
