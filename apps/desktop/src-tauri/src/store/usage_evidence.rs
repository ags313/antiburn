use antiburn_local::analysis::Usage;
use antiburn_local::pricing::ModelTokens;
use anyhow::{Context, Result};
use rusqlite::params;
use serde::Deserialize;

use super::{SessionKey, Store, UsageEvidenceRecord, UsageOrigins, UsageTurnDay};

/// [`Store::usage_evidence`]'s query. `?1` is the earliest activity epoch,
/// `?2` is the reader's offset from UTC in seconds, and `?3` is true when the
/// read includes synced remote sessions.
///
/// The `turn_days` subquery uses the same row filter as
/// `antiburn_local::analysis::query_model_breakdown` at the published fence.
/// It groups the rows by the local day of `ts_ms`, the model, and the speed.
/// A row without a positive timestamp gets a `NULL` day. A schema test checks
/// that the subquery seeks `turn_assistant_session` for each session and does
/// not scan the turn table.
// TOTAL avoids integer overflow. The INTEGER cast saturates each sum at i64::MAX.
pub(super) const USAGE_EVIDENCE_SQL: &str = "SELECT s.agent, COALESCE(s.updated_at_epoch, 0),
        a.model_breakdown_json, a.pricing_breakdown_json, a.provider_hints_json,
        COALESCE((
            SELECT json_group_array(json_object(
                'provider', spa.provider,
                'accountKey', spa.account_key
            ))
              FROM session_provider_account spa
             WHERE spa.environment_key = s.environment_key
               AND spa.agent = s.agent
               AND spa.session_id = s.session_id
        ), '[]'),
        (
            SELECT json_group_array(json_object(
                'day_start', d.day_start,
                'model', d.model,
                'speed', d.speed,
                'input_tokens', d.input_tokens,
                'output_tokens', d.output_tokens,
                'cache_read_tokens', d.cache_read_tokens,
                'cache_write_tokens', d.cache_write_tokens,
                'cache_write_1h_tokens', d.cache_write_1h_tokens,
                'recorded_cost_micros', d.recorded_cost_micros,
                'recorded_rows', d.recorded_rows,
                'row_count', d.row_count
            ))
              FROM (
                SELECT CASE WHEN t.ts_ms > 0
                            THEN ((t.ts_ms / 1000 + ?2) / 86400) * 86400 - ?2
                       END AS day_start,
                       t.model, t.speed,
                       CAST(TOTAL(t.input_tokens) AS INTEGER) AS input_tokens,
                       CAST(TOTAL(t.output_tokens) AS INTEGER) AS output_tokens,
                       CAST(TOTAL(t.cache_read_tokens) AS INTEGER) AS cache_read_tokens,
                       CAST(TOTAL(t.cache_write_tokens) AS INTEGER) AS cache_write_tokens,
                       CAST(TOTAL(t.cache_write_1h_tokens) AS INTEGER) AS cache_write_1h_tokens,
                       CAST(TOTAL(t.recorded_cost_micros) AS INTEGER) AS recorded_cost_micros,
                       COUNT(t.recorded_cost_micros) AS recorded_rows,
                       COUNT(*) AS row_count
                  FROM session_evidence e
                  JOIN turn t
                    ON t.environment_key = e.environment_key
                   AND t.agent = e.agent
                   AND t.session_id = e.session_id
                   AND t.claim_fence = e.published_fence
                 WHERE e.environment_key = s.environment_key
                   AND e.agent = s.agent
                   AND e.session_id = s.session_id
                   AND t.role = 'assistant'
                   AND t.model IS NOT NULL
                   AND (t.input_tokens != 0 OR t.output_tokens != 0
                        OR t.cache_read_tokens != 0 OR t.cache_write_tokens != 0)
                 GROUP BY day_start, t.model, t.speed
              ) d
        ) AS turn_days, s.environment_key, s.session_id
   FROM session s
   LEFT JOIN session_analysis a
     ON a.environment_key = s.environment_key
    AND a.agent = s.agent
    AND a.session_id = s.session_id
  WHERE COALESCE(s.updated_at_epoch, 0) >= ?1
    AND (?3 OR s.environment_key NOT LIKE 'ssh:%')
  ORDER BY COALESCE(s.updated_at_epoch, 0) DESC";

/// Decode the `turn_days` column of [`USAGE_EVIDENCE_SQL`].
///
/// This query writes the JSON itself, so a decode failure is an error and
/// not missing evidence.
fn usage_turn_days(json: &str) -> Result<Vec<UsageTurnDay>> {
    let rows: Vec<TurnDayRow> =
        serde_json::from_str(json).context("failed to decode usage turn days")?;
    let count = |value: i64| u64::try_from(value).unwrap_or(0);
    Ok(rows
        .into_iter()
        .map(|row| {
            // Keep a recorded cost only when every row has one.
            let recorded_cost_micros = row
                .recorded_cost_micros
                .filter(|_| row.recorded_rows == row.row_count)
                .map(count);
            UsageTurnDay {
                day_start_epoch: row.day_start,
                model: row.model,
                speed: row.speed,
                tokens: ModelTokens::from_usage(
                    Usage {
                        input_tokens: count(row.input_tokens),
                        output_tokens: count(row.output_tokens),
                        cache_read_tokens: count(row.cache_read_tokens),
                        cache_creation_tokens: count(row.cache_write_tokens),
                        cache_creation_1h_tokens: count(row.cache_write_1h_tokens),
                    },
                    recorded_cost_micros,
                ),
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct TurnDayRow {
    day_start: Option<i64>,
    model: String,
    speed: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    cache_write_1h_tokens: i64,
    recorded_cost_micros: Option<i64>,
    recorded_rows: i64,
    row_count: i64,
}

impl Store {
    /// Agent, activity timestamp, token breakdown, and provider hints for every
    /// session at or after `since_epoch` that `origins` covers.
    ///
    /// One join rather than a listing plus a lookup per row: provider usage
    /// walks every retained session, and the N+1 shape would take the
    /// connection lock once per session.
    ///
    /// A session with no analysis row still comes back, with `None` for its
    /// breakdown — the aggregation counts it as an unattributed session rather
    /// than pretending it spent nothing.
    ///
    /// Each row also carries the session's published billable turn tokens per
    /// local day of a reader `utc_offset_seconds` from UTC. These are the rows
    /// `query_model_breakdown` sums into the analysis breakdown, so the days of
    /// one session add up to its breakdown. One statement reads both, so a
    /// concurrent publish cannot change the fence between the two reads.
    ///
    /// Opted-in Cursor events replace matched sessions' transcript evidence.
    /// Account-only events appear only for `LocalAndRemote` (Overview).
    pub fn usage_evidence(
        &self,
        since_epoch: i64,
        utc_offset_seconds: i32,
        origins: UsageOrigins,
    ) -> Result<Vec<UsageEvidenceRecord>> {
        let include_remote = origins == UsageOrigins::LocalAndRemote;
        let cursor_usage = self.cursor_usage_evidence(since_epoch, utc_offset_seconds, origins)?;
        let connection = self.lock();
        let mut statement = connection.prepare(USAGE_EVIDENCE_SQL)?;
        let params = params![since_epoch, utc_offset_seconds, include_remote];
        let rows = statement.query_map(params, |row| {
            Ok((
                UsageEvidenceRecord {
                    account_level: false,
                    agent: row.get(0)?,
                    updated_at_epoch: row.get(1)?,
                    model_breakdown_json: row.get(2)?,
                    pricing_breakdown_json: row.get(3)?,
                    provider_hints_json: row.get(4)?,
                    provider_accounts_json: row.get(5)?,
                    turn_days: Vec::new(),
                },
                row.get::<_, String>(6)?,
                SessionKey::new(
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(8)?,
                ),
            ))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (mut record, turn_days_json, key) = row?;
            if cursor_usage.contains_key(&Some(key)) {
                continue;
            }
            record.turn_days = usage_turn_days(&turn_days_json)?;
            records.push(record);
        }
        records.extend(cursor_usage.into_values());
        Ok(records)
    }
}
