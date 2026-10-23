//! Collects the `(session, model, speed, chart bucket)` groups of a quota
//! range from the per-minute totals, and from turn rows only where a minute
//! needs them.
//!
//! A minute entry sits at the start of its minute. This is exact where no
//! time cuts the minute. The reader keeps timestamped entries, one per exact
//! timestamp, for each minute that a range edge, a period boundary, a factor
//! point, or an authoritative meter reading cuts.

use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::provider_usage::quota::resolve_periods;
use crate::provider_usage::quota::share::segment_closing_readings;

use super::super::minute_usage::NON_ASSISTANT_INDEX;
use super::{
    ATTRIBUTED_TURN_BUCKET_SQL, BucketTurnRow, CONTRIBUTION_BUCKET_SECS, MAX_ATTRIBUTION_GROUPS,
    ProviderUsagePeriod, QUOTA_PERIODS_FOR_LANE_SQL, SessionKey, all_recorded,
    lane_duration_seconds, lane_for_period, observation_filter_for_lane, row_to_period,
};

type GroupKey = (SessionKey, Option<String>, Option<String>, i64);

#[derive(Clone)]
struct SessionMetadata {
    hints_json: Option<String>,
    accounts_json: String,
}

#[derive(Default)]
struct GroupSums {
    hints_json: Option<String>,
    accounts_json: String,
    input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
    cache_write_1h_tokens: i64,
    turn_count: i64,
    recorded_cost_micros: i64,
    recorded_rows: i64,
    timed_tokens: Vec<[Option<i64>; 7]>,
}

/// The sums of one source row, or of one group that a turn query returned.
struct GroupPart {
    key: GroupKey,
    metadata: SessionMetadata,
    input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
    cache_write_1h_tokens: i64,
    turn_count: i64,
    recorded_cost_micros: i64,
    recorded_rows: i64,
    timed_tokens: Vec<[Option<i64>; 7]>,
}

/// The groups of one range, bounded by the turn and group limits.
pub(super) struct TurnGroups {
    groups: HashMap<GroupKey, GroupSums>,
    turn_count: usize,
    max_turns: usize,
}

impl TurnGroups {
    pub(super) fn new(max_turns: usize) -> Self {
        Self {
            groups: HashMap::new(),
            turn_count: 0,
            max_turns,
        }
    }

    /// Adds a part, or returns false when a limit overflows.
    fn add(&mut self, part: GroupPart) -> bool {
        self.turn_count = self
            .turn_count
            .saturating_add(usize::try_from(part.turn_count).unwrap_or(0));
        if self.turn_count > self.max_turns {
            return false;
        }
        let sums = self.groups.entry(part.key).or_default();
        if sums.accounts_json.is_empty() {
            sums.hints_json = part.metadata.hints_json;
            sums.accounts_json = part.metadata.accounts_json;
        }
        sums.input_tokens += part.input_tokens;
        sums.cache_read_tokens += part.cache_read_tokens;
        sums.cache_write_tokens += part.cache_write_tokens;
        sums.output_tokens += part.output_tokens;
        sums.cache_write_1h_tokens += part.cache_write_1h_tokens;
        sums.turn_count += part.turn_count;
        sums.recorded_cost_micros += part.recorded_cost_micros;
        sums.recorded_rows += part.recorded_rows;
        sums.timed_tokens.extend(part.timed_tokens);
        self.groups.len() <= MAX_ATTRIBUTION_GROUPS
    }

    pub(super) fn into_rows(self) -> Vec<BucketTurnRow> {
        let mut rows: Vec<BucketTurnRow> = self
            .groups
            .into_iter()
            .map(|((key, model, speed, bucket), sums)| BucketTurnRow {
                key,
                hints_json: sums.hints_json,
                accounts_json: sums.accounts_json,
                model,
                speed,
                input_tokens: sums.input_tokens,
                cache_read_tokens: sums.cache_read_tokens,
                cache_write_tokens: sums.cache_write_tokens,
                output_tokens: sums.output_tokens,
                turn_count: sums.turn_count,
                cache_write_1h_tokens: sums.cache_write_1h_tokens,
                recorded_cost_micros: all_recorded(
                    Some(sums.recorded_cost_micros),
                    sums.recorded_rows,
                    sums.turn_count,
                ),
                bucket_start_epoch: bucket * CONTRIBUTION_BUCKET_SECS,
                timed_tokens: sums.timed_tokens,
            })
            .collect();
        rows.sort_by(|left, right| {
            (&left.key, left.bucket_start_epoch, &left.model, &left.speed).cmp(&(
                &right.key,
                right.bucket_start_epoch,
                &right.model,
                &right.speed,
            ))
        });
        rows
    }

    /// Adds the turn rows with `start_ms < ts_ms <= end_ms`. Returns false
    /// when a limit overflows.
    pub(super) fn read_turn_rows(
        &mut self,
        connection: &Connection,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<bool> {
        let remaining = self.max_turns.saturating_sub(self.turn_count);
        let mut statement = connection.prepare_cached(ATTRIBUTED_TURN_BUCKET_SQL)?;
        let mut query = statement.query(params![
            start_ms,
            end_ms,
            (MAX_ATTRIBUTION_GROUPS + 1) as i64,
            (remaining + 1) as i64
        ])?;
        while let Some(row) = query.next()? {
            let part = GroupPart {
                key: (
                    SessionKey::new(
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ),
                    row.get(5)?,
                    row.get(6)?,
                    row.get(13)?,
                ),
                metadata: SessionMetadata {
                    hints_json: row.get(3)?,
                    accounts_json: row.get(4)?,
                },
                input_tokens: row.get(7)?,
                cache_read_tokens: row.get(8)?,
                cache_write_tokens: row.get(9)?,
                output_tokens: row.get(10)?,
                turn_count: row.get(11)?,
                cache_write_1h_tokens: row.get(12)?,
                recorded_cost_micros: row.get::<_, Option<i64>>(15)?.unwrap_or(0),
                recorded_rows: row.get(16)?,
                timed_tokens: serde_json::from_str(&row.get::<_, String>(14)?)?,
            };
            if !self.add(part) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Adds the range `[from_epoch, to_epoch)` from the minute totals. A
    /// minute that the range, a period boundary, or `now_epoch` cuts comes
    /// from turn rows.
    /// Returns false when a limit overflows.
    pub(super) fn read_minutes(
        &mut self,
        connection: &Connection,
        from_epoch: i64,
        to_epoch: i64,
        now_epoch: i64,
    ) -> Result<bool> {
        let cut_minutes = cut_minutes(connection, from_epoch, to_epoch, now_epoch)?;
        let mut metadata: HashMap<SessionKey, Option<SessionMetadata>> = HashMap::new();
        let mut statement = connection.prepare_cached(
            "SELECT environment_key, agent, session_id, model, speed, minute_epoch,
                    row_count, input_tokens, output_tokens, cache_read_tokens,
                    cache_write_tokens, cache_write_1h_tokens,
                    unrecorded_input_tokens, unrecorded_output_tokens,
                    unrecorded_cache_read_tokens, unrecorded_cache_write_tokens,
                    unrecorded_cache_write_1h_tokens,
                    recorded_cost_micros, recorded_rows
               FROM session_minute_usage
              WHERE minute_epoch >= ?1 AND minute_epoch <= ?2
                AND environment_key NOT LIKE 'ssh:%'",
        )?;
        // Whole minutes only: the cut minutes at both ends come from turn rows.
        let mut query = statement.query(params![(from_epoch + 59) / 60 * 60, to_epoch - 60])?;
        while let Some(row) = query.next()? {
            let minute_epoch: i64 = row.get(5)?;
            if cut_minutes.contains(&minute_epoch) {
                continue;
            }
            let key = SessionKey::new(
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            );
            let Some(session) = session_metadata(connection, &mut metadata, &key)? else {
                continue;
            };
            let speed: String = row.get(4)?;
            let unrecorded = |index| row.get::<_, i64>(index);
            let recorded_cost_micros: i64 = row.get(17)?;
            let recorded_rows: i64 = row.get(18)?;
            let part = GroupPart {
                key: (
                    key,
                    row.get(3)?,
                    Some(speed).filter(|speed| !speed.is_empty()),
                    minute_epoch / CONTRIBUTION_BUCKET_SECS,
                ),
                metadata: session,
                input_tokens: row.get(7)?,
                output_tokens: row.get(8)?,
                cache_read_tokens: row.get(9)?,
                cache_write_tokens: row.get(10)?,
                cache_write_1h_tokens: row.get(11)?,
                turn_count: row.get(6)?,
                recorded_cost_micros,
                recorded_rows,
                timed_tokens: vec![[
                    Some(minute_epoch.saturating_mul(1_000)),
                    Some(unrecorded(12)?),
                    Some(unrecorded(13)?),
                    Some(unrecorded(14)?),
                    Some(unrecorded(15)?),
                    Some(unrecorded(16)?),
                    (recorded_rows > 0).then_some(recorded_cost_micros),
                ]],
            };
            if !self.add(part) {
                return Ok(false);
            }
        }
        drop(query);
        drop(statement);
        let start_ms = from_epoch.saturating_mul(1_000);
        let end_ms = to_epoch.saturating_mul(1_000);
        for &minute in &cut_minutes {
            let low = minute.saturating_mul(1_000).max(start_ms);
            let high = minute.saturating_add(60).saturating_mul(1_000).min(end_ms);
            if low < high && !self.read_timed_groups(connection, &mut metadata, low, high, None)? {
                return Ok(false);
            }
        }
        // The minute table holds assistant rows only. Turns of another role
        // in the uncut whole minutes come from the partial index.
        let low = (from_epoch + 59) / 60 * 60;
        let high = to_epoch / 60 * 60;
        if low < high
            && !self.read_timed_groups(
                connection,
                &mut metadata,
                low.saturating_mul(1_000),
                high.saturating_mul(1_000),
                Some(&cut_minutes),
            )?
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// Adds the turns with `low_ms <= ts_ms < high_ms`, summed per session,
    /// model, speed, and exact timestamp. Equal timestamps price and sort
    /// alike, so one entry per timestamp keeps the allocation exact.
    ///
    /// With `uncut_minutes_of` set, the read keeps only turns of a role other
    /// than `assistant` and skips the listed minutes, which a full read covers.
    fn read_timed_groups(
        &mut self,
        connection: &Connection,
        metadata: &mut HashMap<SessionKey, Option<SessionMetadata>>,
        low_ms: i64,
        high_ms: i64,
        uncut_minutes_of: Option<&BTreeSet<i64>>,
    ) -> Result<bool> {
        let (index, role_filter) = if uncut_minutes_of.is_some() {
            (NON_ASSISTANT_INDEX, "AND t.role IS NOT 'assistant'")
        } else {
            ("turn_usage_timestamp", "")
        };
        let mut statement = connection.prepare_cached(&format!(
            "SELECT t.environment_key, t.agent, t.session_id, t.model, t.speed, t.ts_ms,
                    COUNT(*),
                    CAST(TOTAL(t.input_tokens) AS INTEGER),
                    CAST(TOTAL(t.output_tokens) AS INTEGER),
                    CAST(TOTAL(t.cache_read_tokens) AS INTEGER),
                    CAST(TOTAL(t.cache_write_tokens) AS INTEGER),
                    CAST(TOTAL(t.cache_write_1h_tokens) AS INTEGER),
                    CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.input_tokens, 0)) AS INTEGER),
                    CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.output_tokens, 0)) AS INTEGER),
                    CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.cache_read_tokens, 0)) AS INTEGER),
                    CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.cache_write_tokens, 0)) AS INTEGER),
                    CAST(TOTAL(IIF(t.recorded_cost_micros IS NULL, t.cache_write_1h_tokens, 0)) AS INTEGER),
                    CAST(TOTAL(t.recorded_cost_micros) AS INTEGER),
                    COUNT(t.recorded_cost_micros)
               FROM turn t INDEXED BY {index}
               JOIN session_evidence e
                 ON e.environment_key = t.environment_key
                AND e.agent = t.agent AND e.session_id = t.session_id
                AND e.published_fence = t.claim_fence
              WHERE t.ts_ms >= ?1 AND t.ts_ms < ?2
                AND t.environment_key NOT LIKE 'ssh:%'
                {role_filter}
                AND (t.input_tokens > 0 OR t.output_tokens > 0
                     OR t.cache_read_tokens > 0 OR t.cache_write_tokens > 0)
              GROUP BY t.environment_key, t.agent, t.session_id, t.model, t.speed, t.ts_ms"
        ))?;
        let mut query = statement.query(params![low_ms, high_ms])?;
        while let Some(row) = query.next()? {
            let key = SessionKey::new(
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            );
            let Some(session) = session_metadata(connection, metadata, &key)? else {
                continue;
            };
            let ts_ms: i64 = row.get(5)?;
            if uncut_minutes_of.is_some_and(|cut| cut.contains(&(ts_ms.div_euclid(60_000) * 60))) {
                continue;
            }
            let recorded_cost_micros: i64 = row.get(17)?;
            let recorded_rows: i64 = row.get(18)?;
            let part = GroupPart {
                key: (
                    key,
                    row.get(3)?,
                    row.get(4)?,
                    ts_ms / (CONTRIBUTION_BUCKET_SECS * 1_000),
                ),
                metadata: session,
                turn_count: row.get(6)?,
                input_tokens: row.get(7)?,
                output_tokens: row.get(8)?,
                cache_read_tokens: row.get(9)?,
                cache_write_tokens: row.get(10)?,
                cache_write_1h_tokens: row.get(11)?,
                recorded_cost_micros,
                recorded_rows,
                timed_tokens: vec![[
                    Some(ts_ms),
                    Some(row.get(12)?),
                    Some(row.get(13)?),
                    Some(row.get(14)?),
                    Some(row.get(15)?),
                    Some(row.get(16)?),
                    (recorded_rows > 0).then_some(recorded_cost_micros),
                ]],
            };
            if !self.add(part) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// One session's analysis hints and account bindings, or `None` when the
/// session row is gone.
fn session_metadata(
    connection: &Connection,
    cache: &mut HashMap<SessionKey, Option<SessionMetadata>>,
    key: &SessionKey,
) -> Result<Option<SessionMetadata>> {
    if let Some(cached) = cache.get(key) {
        return Ok(cached.clone());
    }
    let mut statement = connection.prepare_cached(
        "SELECT a.provider_hints_json,
                COALESCE((
                    SELECT json_group_array(json_object(
                        'provider', spa.provider,
                        'accountKey', spa.account_key
                    ))
                      FROM session_provider_account spa
                     WHERE spa.environment_key = s.environment_key
                       AND spa.agent = s.agent AND spa.session_id = s.session_id
                ), '[]')
           FROM session s
           LEFT JOIN session_analysis a
             ON a.environment_key = s.environment_key
            AND a.agent = s.agent AND a.session_id = s.session_id
          WHERE s.environment_key = ?1 AND s.agent = ?2 AND s.session_id = ?3",
    )?;
    let found = statement
        .query_row(
            params![key.environment_key, key.agent, key.session_id],
            |row| {
                Ok(SessionMetadata {
                    hints_json: row.get(0)?,
                    accounts_json: row.get(1)?,
                })
            },
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            error => Err(error),
        })?;
    cache.insert(key.clone(), found.clone());
    Ok(found)
}

/// The start of every minute that a range edge, a period boundary, a factor
/// point, or an authoritative reading cuts.
///
/// The boundaries are the ones that [`resolve_periods`] returns for each lane
/// without turn gaps. A turn gap starts at a minute start, and it ends at a
/// minute start or at a period start that this list holds already.
fn cut_minutes(
    connection: &Connection,
    from_epoch: i64,
    to_epoch: i64,
    now_epoch: i64,
) -> Result<BTreeSet<i64>> {
    let mut cut = BTreeSet::new();
    // The caller drops turns after `now_epoch` inclusive. This holds for the
    // whole minute of `now_epoch`, also when it starts on a minute.
    let now_minute = now_epoch.div_euclid(60) * 60;
    if now_minute < to_epoch && now_minute.saturating_add(60) > from_epoch {
        cut.insert(now_minute);
    }
    let mut cut_at = |epoch: i64| {
        if epoch % 60 != 0 && epoch >= from_epoch && epoch <= to_epoch {
            cut.insert(epoch.div_euclid(60) * 60);
        }
    };
    cut_at(from_epoch);
    cut_at(to_epoch);
    let periods = period_boundaries(connection, from_epoch, to_epoch)?;
    for period in &periods {
        cut_at(period.starts_at_epoch);
        cut_at(period.resets_at_epoch);
    }
    let mut statement = connection.prepare_cached(
        "SELECT effective_at_epoch FROM provider_limit_factor_point
          WHERE effective_at_epoch >= ?1 AND effective_at_epoch <= ?2",
    )?;
    for epoch in statement.query_map(params![from_epoch, to_epoch], |row| row.get::<_, i64>(0))? {
        cut_at(epoch?);
    }
    // A reading that closes a meter segment separates a turn at its own
    // second from later turns, so even a reading at a minute start cuts its
    // minute. The other readings divide no usage.
    let mut statement = connection.prepare_cached(
        "SELECT observed_at_epoch, used_percent FROM provider_usage_observation
          WHERE period_id = ?1 AND is_authoritative = 1 AND used_percent IS NOT NULL
            AND observed_at_epoch >= ?2 AND observed_at_epoch < ?3
          ORDER BY observed_at_epoch, id",
    )?;
    for period in &periods {
        let Some(period_id) = period.period_id else {
            continue;
        };
        let readings = statement
            .query_map(
                params![period_id, period.starts_at_epoch, period.resets_at_epoch],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (epoch, _) in segment_closing_readings(&readings) {
            cut.insert(epoch.div_euclid(60) * 60);
        }
    }
    Ok(cut)
}

fn period_boundaries(
    connection: &Connection,
    from_epoch: i64,
    to_epoch: i64,
) -> Result<Vec<crate::provider_usage::quota::QuotaPeriod>> {
    let mut lanes: BTreeSet<(String, String, String)> = BTreeSet::new();
    {
        let mut statement = connection.prepare_cached(
            "SELECT id, provider, account_key, window_id, window_kind,
                    window_role, scope_key, scope_label, duration_seconds, starts_at_epoch,
                    resets_at_epoch, first_observed_epoch, last_observed_epoch
               FROM provider_usage_period",
        )?;
        let periods = statement
            .query_map([], row_to_period)?
            .collect::<rusqlite::Result<Vec<ProviderUsagePeriod>>>()?;
        for period in &periods {
            if let Some(lane) = lane_for_period(period) {
                lanes.insert((period.provider.clone(), period.account_key.clone(), lane));
            }
        }
    }
    let mut statement = connection.prepare_cached(QUOTA_PERIODS_FOR_LANE_SQL)?;
    let mut boundaries = Vec::new();
    for (provider, account_key, lane) in lanes {
        let (window_role, window_id) = observation_filter_for_lane(&lane);
        let duration = lane_duration_seconds(&lane);
        let observed = statement
            .query_map(
                params![
                    provider,
                    account_key,
                    window_role,
                    window_id,
                    from_epoch,
                    to_epoch,
                    duration
                ],
                row_to_period,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        boundaries.extend(resolve_periods(
            &lane,
            duration,
            &observed,
            &[],
            from_epoch,
            to_epoch,
            to_epoch,
        ));
    }
    Ok(boundaries)
}
