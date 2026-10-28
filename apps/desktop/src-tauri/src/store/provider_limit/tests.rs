use std::path::Path;

use rusqlite::params;

use super::*;
use crate::store::{RETAIN_SESSION_DATA_FOREVER, SessionRecord};

const PROVIDER: &str = "anthropic";
const AGENT: &str = "claude-code";
const MODEL: &str = "claude-opus-4-6";

fn account(character: char) -> String {
    character.to_string().repeat(64)
}

fn memory_store() -> Store {
    Store::open_in_memory(Path::new("/tmp/antiburn-provider-limit-test")).expect("opens store")
}

fn insert_session(store: &Store, session_id: &str) -> SessionKey {
    insert_session_for_agent(store, session_id, AGENT)
}

fn insert_session_for_agent(store: &Store, session_id: &str, agent: &str) -> SessionKey {
    insert_session_in_environment(store, "native", session_id, agent)
}

fn insert_session_in_environment(
    store: &Store,
    environment_key: &str,
    session_id: &str,
    agent: &str,
) -> SessionKey {
    let key = SessionKey::new(environment_key, agent, session_id);
    store
        .upsert_sessions(
            &[SessionRecord {
                key: key.clone(),
                source_kind: "inline".to_string(),
                source_label: "synthetic".to_string(),
                wsl_distro: environment_key.strip_prefix("wsl:").map(str::to_owned),
                title: None,
                title_source: None,
                cwd: None,
                surface: "unknown".to_string(),
                updated_at_epoch: Some(1),
                activity_cursor: "synthetic".to_string(),
                activity_source: "event".to_string(),
                subagent_count: 0,
                fork_parent_session_id: None,
                source_fingerprint: Some("synthetic".to_string()),
            }],
            &[],
        )
        .expect("stores synthetic session");
    key
}

/// Publish one turn at `ts_ms` and mark its session evidence published, so
/// the attribution query's `session_evidence` join matches it.
fn insert_turn(store: &Store, key: &SessionKey, ts_ms: i64, input_tokens: i64) {
    let connection = store.lock();
    connection
        .execute(
            "INSERT OR IGNORE INTO session_evidence (
                 environment_key, agent, session_id, status, published_fence
             ) VALUES (?1, ?2, ?3, 'ready', 1)",
            params![key.environment_key, key.agent, key.session_id],
        )
        .expect("publishes synthetic evidence");
    connection
        .execute(
            "INSERT INTO turn (
                 environment_key, agent, session_id, claim_fence, source_key,
                 thread_id, turn_index, scope, role, ts_ms, model, effort, speed,
                 input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
                 is_compaction_boundary, message_id, uuid, parent_uuid
             ) VALUES (?1, ?2, ?3, 1, 'synthetic', 'synthetic', 0, 'main', 'assistant',
                       ?4, ?5, NULL, NULL, ?6, 0, 0, 0, 0, NULL, NULL, NULL)",
            params![
                key.environment_key,
                key.agent,
                key.session_id,
                ts_ms,
                MODEL,
                input_tokens
            ],
        )
        .expect("stores synthetic turn");
}

/// Publish one turn whose cache-write tokens are all one-hour writes, so the
/// attribution query's cache-write pricing can be checked at the double rate.
fn insert_turn_with_one_hour_cache_write(
    store: &Store,
    key: &SessionKey,
    ts_ms: i64,
    cache_write_tokens: i64,
) {
    let connection = store.lock();
    connection
        .execute(
            "INSERT OR IGNORE INTO session_evidence (
                 environment_key, agent, session_id, status, published_fence
             ) VALUES (?1, ?2, ?3, 'ready', 1)",
            params![key.environment_key, key.agent, key.session_id],
        )
        .expect("publishes synthetic evidence");
    connection
        .execute(
            "INSERT INTO turn (
                 environment_key, agent, session_id, claim_fence, source_key,
                 thread_id, turn_index, scope, role, ts_ms, model, effort, speed,
                 input_tokens, cache_read_tokens, cache_write_tokens,
                 cache_write_1h_tokens, output_tokens,
                 is_compaction_boundary, message_id, uuid, parent_uuid
             ) VALUES (?1, ?2, ?3, 1, 'synthetic', 'synthetic', 0, 'main', 'assistant',
                       ?4, ?5, NULL, NULL, 0, 0, ?6, ?7, 0, 0, NULL, NULL, NULL)",
            params![
                key.environment_key,
                key.agent,
                key.session_id,
                ts_ms,
                MODEL,
                cache_write_tokens,
                cache_write_tokens
            ],
        )
        .expect("stores synthetic turn");
}

fn bind_account(store: &Store, key: &SessionKey, account_key: &str) {
    store
        .lock()
        .execute(
            "INSERT INTO session_provider_account (
                 environment_key, agent, session_id, provider, account_key,
                 provenance, confidence, first_seen_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'provider_live', 'direct', '2026-01-01T00:00:00Z')",
            params![
                key.environment_key,
                key.agent,
                key.session_id,
                PROVIDER,
                account_key
            ],
        )
        .expect("binds session account");
}

fn insert_period(
    store: &Store,
    account_key: &str,
    start: i64,
    reset: i64,
    last_observed: i64,
) -> i64 {
    let connection = store.lock();
    connection
        .execute(
            "INSERT INTO provider_usage_period (
                 provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, duration_seconds, starts_at_epoch,
                 resets_at_epoch, first_observed_epoch, last_observed_epoch
             ) VALUES (?1, ?2, 'five-hour', 'rolling', 'primaryShort',
                       'account', 'account', ?3, ?4, ?5, ?6, ?6)",
            params![
                PROVIDER,
                account_key,
                reset - start,
                start,
                reset,
                last_observed
            ],
        )
        .expect("inserts a synthetic period");
    connection.last_insert_rowid()
}

fn observe_account(store: &Store, account_key: &str) {
    store
        .lock()
        .execute(
            "INSERT INTO provider_account_seen (agent, provider, account_key,
                 first_seen_epoch, last_seen_epoch)
             VALUES (?1, ?2, ?3, 1, 1)",
            params![AGENT, PROVIDER, account_key],
        )
        .expect("records a seen account");
}

#[test]
fn an_unbound_session_with_two_known_accounts_attributes_nothing() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    insert_turn(&store, &key, 150_000, 200_000);
    observe_account(&store, &account('a'));
    observe_account(&store, &account('b'));

    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert!(
        dollars.is_empty(),
        "an ambiguous account set attributes no session"
    );
}

#[test]
fn an_unbound_session_with_one_known_account_falls_back_to_it() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    insert_turn(&store, &key, 150_000, 200_000);
    observe_account(&store, &account('a'));

    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert_eq!(dollars.len(), 1);
    assert_eq!(dollars[0].key, key);
    // claude-opus-4-6 test pricing: 5e-6 dollars per input token.
    assert!((dollars[0].input_usd - 1.0).abs() < 1e-9);
    assert_eq!(dollars[0].turn_count, 1);
}

#[test]
fn cached_remote_turns_do_not_enter_local_provider_limit_inputs() {
    let store = memory_store();
    let native = insert_session_in_environment(&store, "native", "native", AGENT);
    let wsl = insert_session_in_environment(&store, "wsl:ubuntu", "wsl", AGENT);
    let legacy = insert_session_in_environment(&store, "legacy-local", "legacy", AGENT);
    let remote = insert_session_in_environment(&store, "ssh:host-id", "remote", AGENT);
    observe_account(&store, &account('a'));

    for (key, ts_ms) in [
        (&native, 100_000),
        (&wsl, 200_000),
        (&legacy, 300_000),
        (&remote, 400_000),
    ] {
        insert_turn(&store, key, ts_ms, 100_000);
    }

    let mut dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    dollars.sort_by(|left, right| left.key.environment_key.cmp(&right.key.environment_key));
    assert_eq!(
        dollars
            .iter()
            .map(|row| row.key.environment_key.as_str())
            .collect::<Vec<_>>(),
        vec!["legacy-local", "native", "wsl:ubuntu"]
    );

    let mut bucket_keys = store
        .quota_turn_input(0, 1_000)
        .expect("query succeeds")
        .expect("stays within the turn bound")
        .for_account(PROVIDER, &account('a'), None)
        .by_bucket(None)
        .into_iter()
        .map(|row| row.key.environment_key)
        .collect::<Vec<_>>();
    bucket_keys.sort();
    assert_eq!(bucket_keys, vec!["legacy-local", "native", "wsl:ubuntu"]);

    let minutes = store
        .attributed_turn_minutes(0, 1_000)
        .expect("scan succeeds")
        .for_account(PROVIDER, &account('a'));
    assert_eq!(minutes, vec![60, 180, 300]);
}

#[test]
fn a_session_with_no_analysis_row_still_appears() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    insert_turn(&store, &key, 150_000, 200_000);
    observe_account(&store, &account('a'));

    // The group-first rewrite joins `session_analysis` in the outer query.
    // Confirm the fixture carries no such row, so this test actually
    // exercises the LEFT JOIN rather than an INNER JOIN that happens to
    // match.
    let analysis_row_count: i64 = store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM session_analysis
              WHERE environment_key = ?1 AND agent = ?2 AND session_id = ?3",
            params![key.environment_key, key.agent, key.session_id],
            |row| row.get(0),
        )
        .expect("counts analysis rows");
    assert_eq!(analysis_row_count, 0, "test setup carries no analysis row");

    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert_eq!(
        dollars.len(),
        1,
        "a session with turns but no analysis row still appears"
    );
    assert_eq!(dollars[0].key, key);
}

#[test]
fn attribution_prices_one_hour_cache_writes_at_double_the_input_rate() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    insert_turn_with_one_hour_cache_write(&store, &key, 150_000, 100_000);
    observe_account(&store, &account('a'));

    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert_eq!(dollars.len(), 1);
    assert_eq!(dollars[0].key, key);
    // claude-opus-4-6 test pricing: 5e-6 dollars per input token, so the
    // one-hour cache-write subset prices at 1e-5 dollars per token.
    assert!((dollars[0].cache_write_usd - 1.0).abs() < 1e-9);
}

#[test]
fn a_directly_bound_session_ignores_the_single_account_fallback() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    insert_turn(&store, &key, 150_000, 200_000);
    bind_account(&store, &key, &account('a'));
    observe_account(&store, &account('b'));

    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('b'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert!(
        dollars.is_empty(),
        "a session bound to a different account never falls back"
    );

    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert_eq!(dollars.len(), 1);
}

#[test]
fn point_lookup_at_an_epoch_uses_the_latest_point_at_or_before_it_else_the_earliest() {
    let store = memory_store();
    let lane = LANE_FIVE_HOUR;
    store
        .upsert_factor_point(&FactorPoint {
            id: 0,
            provider: PROVIDER.to_string(),
            account_key: account('a'),
            lane: lane.to_string(),
            effective_at_epoch: 1_000,
            usd_per_percent: 0.2,
            method: "delta".to_string(),
            sample_count: 3,
            plan: None,
            plan_tier: None,
        })
        .unwrap();
    store
        .upsert_factor_point(&FactorPoint {
            id: 0,
            provider: PROVIDER.to_string(),
            account_key: account('a'),
            lane: lane.to_string(),
            effective_at_epoch: 2_000,
            usd_per_percent: 0.3,
            method: "delta".to_string(),
            sample_count: 4,
            plan: None,
            plan_tier: None,
        })
        .unwrap();

    let before_any = store
        .factor_point_at(PROVIDER, &account('a'), lane, 500)
        .unwrap()
        .expect("falls back to the earliest point");
    assert_eq!(before_any.effective_at_epoch, 1_000);

    let between = store
        .factor_point_at(PROVIDER, &account('a'), lane, 1_500)
        .unwrap()
        .expect("uses the latest point at or before the epoch");
    assert_eq!(between.effective_at_epoch, 1_000);

    let after_both = store
        .factor_point_at(PROVIDER, &account('a'), lane, 5_000)
        .unwrap()
        .expect("uses the newest point");
    assert_eq!(after_both.effective_at_epoch, 2_000);
}

#[test]
fn sample_retention_deletes_old_samples_but_never_points() {
    let store = memory_store();
    let lane = LANE_FIVE_HOUR;
    let sample = FactorSample {
        provider: PROVIDER.to_string(),
        account_key: account('a'),
        lane: lane.to_string(),
        kind: "delta".to_string(),
        period_id: None,
        from_epoch: 0,
        to_epoch: 100,
        from_percent: 0.0,
        to_percent: 5.0,
        input_usd: 1.0,
        output_usd: 0.0,
        cache_read_usd: 0.0,
        cache_write_usd: 0.0,
        turn_count: 1,
        plan: None,
        plan_tier: None,
        source_id: "test".to_string(),
        computed_at_epoch: 100,
    };
    store.upsert_factor_sample(&sample).unwrap();
    store
        .upsert_factor_point(&FactorPoint {
            id: 0,
            provider: PROVIDER.to_string(),
            account_key: account('a'),
            lane: lane.to_string(),
            effective_at_epoch: 100,
            usd_per_percent: 0.2,
            method: "delta".to_string(),
            sample_count: 1,
            plan: None,
            plan_tier: None,
        })
        .unwrap();

    // 20 days elapsed, 30-day retention: the cutoff has not yet reached the
    // sample's to_epoch (100), so nothing is removed yet.
    let now = 20 * 86_400;
    apply_sample_retention_in(&store.lock(), 30, now).unwrap();
    assert_eq!(
        store
            .all_delta_factor_samples(PROVIDER, &account('a'), lane)
            .unwrap()
            .len(),
        1
    );

    // 40 days elapsed, the same 30-day retention: the cutoff has moved past
    // the sample's to_epoch, so the sample goes; the point does not.
    let now = 40 * 86_400;
    apply_sample_retention_in(&store.lock(), 30, now).unwrap();
    assert!(
        store
            .all_delta_factor_samples(PROVIDER, &account('a'), lane)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .latest_factor_point(PROVIDER, &account('a'), lane)
            .unwrap()
            .is_some(),
        "retention never deletes factor points"
    );
}

/// One sample of `kind`, spanning `from_epoch..to_epoch`, otherwise
/// identical to [`sample_retention_deletes_old_samples_but_never_points`]'s
/// fixture.
fn kind_sample(kind: &str, from_epoch: i64, to_epoch: i64) -> FactorSample {
    FactorSample {
        provider: PROVIDER.to_string(),
        account_key: account('a'),
        lane: LANE_FIVE_HOUR.to_string(),
        kind: kind.to_string(),
        period_id: None,
        from_epoch,
        to_epoch,
        from_percent: 0.0,
        to_percent: 5.0,
        input_usd: 1.0,
        output_usd: 0.0,
        cache_read_usd: 0.0,
        cache_write_usd: 0.0,
        turn_count: 1,
        plan: None,
        plan_tier: None,
        source_id: "test".to_string(),
        computed_at_epoch: to_epoch,
    }
}

#[test]
fn a_rollout_sample_counts_the_same_as_a_delta_sample() {
    let store = memory_store();
    let lane = LANE_FIVE_HOUR;
    store
        .upsert_factor_sample(&kind_sample("delta", 0, 100))
        .unwrap();
    store
        .upsert_factor_sample(&kind_sample("rollout", 100, 200))
        .unwrap();
    store
        .upsert_factor_sample(&kind_sample("unattributed", 200, 300))
        .unwrap();
    store
        .upsert_factor_sample(&kind_sample("window_start", 300, 400))
        .unwrap();

    let all = store
        .all_delta_factor_samples(PROVIDER, &account('a'), lane)
        .unwrap();
    assert_eq!(
        all.iter()
            .map(|sample| sample.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["delta", "rollout"],
        "the median's sample pool takes delta and rollout alike, and nothing else"
    );

    let recent = store
        .delta_factor_samples_since(PROVIDER, &account('a'), lane, 0)
        .unwrap();
    assert_eq!(recent.len(), 2);
}

#[test]
fn a_rollout_only_history_satisfies_has_delta_factor_sample() {
    let store = memory_store();
    let lane = LANE_FIVE_HOUR;
    assert!(
        !store
            .has_delta_factor_sample(PROVIDER, &account('a'), lane, None, None)
            .unwrap()
    );

    store
        .upsert_factor_sample(&kind_sample("rollout", 0, 100))
        .unwrap();

    assert!(
        store
            .has_delta_factor_sample(PROVIDER, &account('a'), lane, None, None)
            .unwrap(),
        "a rollout sample blocks a window-start sample from forming, the same as a delta sample"
    );
}

#[test]
fn a_sample_s_period_reference_is_nulled_before_its_period_is_deleted() {
    let store = memory_store();
    let account_key = account('a');
    let period_id = {
        let connection = store.lock();
        connection
            .execute(
                "INSERT INTO provider_usage_period (
                     provider, account_key, window_id, window_kind, window_role,
                     scope_key, scope_label, duration_seconds, starts_at_epoch,
                     resets_at_epoch, first_observed_epoch, last_observed_epoch
                 ) VALUES (?1, ?2, 'five-hour', 'rolling', 'primaryShort',
                           'account', 'account', 18000, 0, 18000, 10, 10)",
                params![PROVIDER, account_key],
            )
            .unwrap();
        let period_id = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO provider_usage_observation (
                     period_id, provider, account_key, window_id, window_kind, window_role,
                     scope_key, scope_label, observed_at_epoch, used_percent, is_fresh,
                     is_authoritative, confidence, source_id
                 ) VALUES (?1, ?2, ?3, 'five-hour', 'rolling', 'primaryShort',
                           'account', 'account', 10, 5.0, 1, 1, 'high', 'test')",
                params![period_id, PROVIDER, account_key],
            )
            .unwrap();
        period_id
    };
    let recent = 190 * 86_400;
    store
        .upsert_factor_sample(&FactorSample {
            provider: PROVIDER.to_string(),
            account_key: account_key.clone(),
            lane: LANE_FIVE_HOUR.to_string(),
            kind: "delta".to_string(),
            period_id: Some(period_id),
            from_epoch: 0,
            to_epoch: recent,
            from_percent: 0.0,
            to_percent: 5.0,
            input_usd: 1.0,
            output_usd: 0.0,
            cache_read_usd: 0.0,
            cache_write_usd: 0.0,
            turn_count: 1,
            plan: None,
            plan_tier: None,
            source_id: "test".to_string(),
            computed_at_epoch: recent,
        })
        .unwrap();

    // A 30-day cutoff removes the old observation and, with it, the now
    // orphaned period. The sample's own `to_epoch` is recent enough to
    // survive that same cutoff, so it must not keep a dangling period id.
    let now = 200 * 86_400;
    Store::apply_provider_usage_retention_in(&store.lock(), 30, now).unwrap();

    let samples = store
        .all_delta_factor_samples(PROVIDER, &account_key, LANE_FIVE_HOUR)
        .unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(
        samples[0].period_id, None,
        "the deleted period's id is nulled, not left dangling"
    );
}

#[test]
fn a_period_with_no_learn_cursor_is_a_candidate_however_old_its_last_reading() {
    let store = memory_store();
    // Last observed two days ago: a "recently observed" rule alone would
    // never pick this up. Bootstrap on upgrade relies on it being a
    // candidate anyway, since it has no cursor row yet.
    let period_id = insert_period(&store, &account('a'), 0, 18_000, 2 * 86_400);

    let now = 3 * 86_400;
    let periods = store
        .provider_limit_candidate_periods(now - 900)
        .expect("query succeeds");
    assert_eq!(periods.len(), 1);
    assert_eq!(periods[0].id, period_id);
}

#[test]
fn a_period_whose_cursor_has_caught_up_and_gone_stale_is_not_a_candidate() {
    let store = memory_store();
    let last_observed = 2 * 86_400;
    let period_id = insert_period(&store, &account('a'), 0, 18_000, last_observed);
    store
        .advance_learn_cursor(period_id, last_observed)
        .expect("advances the cursor");

    let now = 3 * 86_400;
    let periods = store
        .provider_limit_candidate_periods(now - 900)
        .expect("query succeeds");
    assert!(
        periods.is_empty(),
        "a cursor already at the period's last reading, outside the recompute window, is not re-read"
    );
}

#[test]
fn reopening_unpriced_samples_makes_only_their_periods_candidates_again() {
    let store = memory_store();
    let last_observed = 2 * 86_400;
    let unpriced = insert_period(&store, &account('a'), 0, 18_000, last_observed);
    let empty = insert_period(&store, &account('a'), 18_000, 36_000, last_observed);
    let priced = insert_period(&store, &account('a'), 36_000, 54_000, last_observed);
    let unpriced_start = insert_period(&store, &account('a'), 54_000, 72_000, last_observed);
    let priced_start = insert_period(&store, &account('a'), 72_000, 90_000, last_observed);
    let sample = |kind: &str, period_id: i64, from_epoch: i64, turn_count: i64| FactorSample {
        period_id: Some(period_id),
        input_usd: 0.0,
        turn_count,
        ..kind_sample(kind, from_epoch, from_epoch + 100)
    };
    for (sample, period_id) in [
        (sample("unattributed", unpriced, 0, 4), unpriced),
        (sample("unattributed", empty, 18_000, 0), empty),
        (
            FactorSample {
                input_usd: 1.0,
                ..sample("delta", priced, 36_000, 4)
            },
            priced,
        ),
        // A window-start sample keeps its kind at zero dollars.
        (
            sample("window_start", unpriced_start, 54_000, 4),
            unpriced_start,
        ),
        (
            FactorSample {
                input_usd: 1.0,
                ..sample("window_start", priced_start, 72_000, 4)
            },
            priced_start,
        ),
    ] {
        store.upsert_factor_sample(&sample).unwrap();
        store
            .advance_learn_cursor(period_id, last_observed)
            .unwrap();
    }

    assert_eq!(store.reopen_unpriced_factor_samples().unwrap(), 2);

    let kinds: Vec<(i64, String)> = store
        .lock()
        .prepare("SELECT from_epoch, kind FROM provider_limit_factor_sample ORDER BY from_epoch")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        kinds,
        vec![
            (18_000, "unattributed".to_string()),
            (36_000, "delta".to_string()),
            (72_000, "window_start".to_string()),
        ],
        "a sample with no turns, or with dollars, has nothing a price could change"
    );
    let mut candidates: Vec<i64> = store
        .provider_limit_candidate_periods(3 * 86_400 - 900)
        .unwrap()
        .into_iter()
        .map(|period| period.id)
        .collect();
    candidates.sort_unstable();
    assert_eq!(candidates, vec![unpriced, unpriced_start]);
}

#[test]
fn a_period_with_a_cursor_behind_its_last_reading_is_a_candidate() {
    let store = memory_store();
    let last_observed = 2 * 86_400;
    let period_id = insert_period(&store, &account('a'), 0, 18_000, last_observed);
    // A backfill (or an earlier partial pass) left the cursor behind the
    // period's newest reading.
    store
        .advance_learn_cursor(period_id, last_observed - 1)
        .expect("advances the cursor");

    let now = 3 * 86_400;
    let periods = store
        .provider_limit_candidate_periods(now - 900)
        .expect("query succeeds");
    assert_eq!(periods.len(), 1);
    assert_eq!(periods[0].id, period_id);
}

#[test]
fn a_period_s_learn_cursor_is_deleted_before_its_period_is_deleted() {
    let store = memory_store();
    let account_key = account('a');
    let period_id = {
        let connection = store.lock();
        connection
            .execute(
                "INSERT INTO provider_usage_period (
                     provider, account_key, window_id, window_kind, window_role,
                     scope_key, scope_label, duration_seconds, starts_at_epoch,
                     resets_at_epoch, first_observed_epoch, last_observed_epoch
                 ) VALUES (?1, ?2, 'five-hour', 'rolling', 'primaryShort',
                           'account', 'account', 18000, 0, 18000, 10, 10)",
                params![PROVIDER, account_key],
            )
            .unwrap();
        connection.last_insert_rowid()
    };
    store
        .advance_learn_cursor(period_id, 10)
        .expect("advances the cursor");

    let now = 200 * 86_400;
    Store::apply_provider_usage_retention_in(&store.lock(), 400, now).unwrap();

    let remaining: i64 = store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM provider_limit_learn_cursor WHERE period_id = ?1",
            [period_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining, 0,
        "the cursor is deleted, not left pointing at a removed period"
    );
}

#[test]
fn forever_retention_keeps_every_provider_usage_row() {
    let store = memory_store();
    let account_key = account('a');
    let period_id = insert_period(&store, &account_key, 0, 18_000, 10);
    store
        .lock()
        .execute(
            "INSERT INTO provider_usage_observation (
                 period_id, provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, observed_at_epoch, used_percent, is_fresh,
                 is_authoritative, confidence, source_id
             ) VALUES (?1, ?2, ?3, 'five-hour', 'rolling', 'primaryShort',
                       'account', 'account', 10, 5.0, 1, 1, 'high', 'test')",
            params![period_id, PROVIDER, account_key],
        )
        .unwrap();
    store
        .upsert_factor_sample(&FactorSample {
            provider: PROVIDER.to_string(),
            account_key: account_key.clone(),
            lane: LANE_FIVE_HOUR.to_string(),
            kind: "delta".to_string(),
            period_id: Some(period_id),
            from_epoch: 0,
            to_epoch: 10,
            from_percent: 0.0,
            to_percent: 5.0,
            input_usd: 1.0,
            output_usd: 0.0,
            cache_read_usd: 0.0,
            cache_write_usd: 0.0,
            turn_count: 1,
            plan: None,
            plan_tier: None,
            source_id: "test".to_string(),
            computed_at_epoch: 10,
        })
        .unwrap();
    store
        .upsert_limit_residual(period_id, 10, 5.0, 4.0)
        .unwrap();

    // 200 days old, well past any of the caps this rule used to enforce.
    let now = 200 * 86_400;
    let removed =
        Store::apply_provider_usage_retention_in(&store.lock(), RETAIN_SESSION_DATA_FOREVER, now)
            .unwrap();
    assert_eq!(removed, 0, "forever prunes nothing");

    let connection = store.lock();
    let count = |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(
        count("provider_usage_observation"),
        1,
        "forever keeps the observation"
    );
    assert_eq!(
        count("provider_usage_period"),
        1,
        "forever keeps the period"
    );
    assert_eq!(
        count("provider_limit_factor_sample"),
        1,
        "forever keeps the sample"
    );
    assert_eq!(
        count("provider_limit_residual"),
        1,
        "forever keeps the residual"
    );
}

/// A period reports at most once, ever: marking it is idempotent, and the
/// marker is what a later pass checks before deciding to report it again.
#[test]
#[cfg(feature = "analytics")]
fn marking_a_quota_window_reported_is_idempotent_and_durable() {
    let store = memory_store();
    let account_key = account('a');
    let period_id = insert_period(&store, &account_key, 0, 18_000, 18_000);

    assert!(!store.quota_window_already_reported(period_id).unwrap());

    store.mark_quota_window_reported(period_id, 20_000).unwrap();
    assert!(store.quota_window_already_reported(period_id).unwrap());

    // A second mark of the same period does not error and does not
    // duplicate the row.
    store.mark_quota_window_reported(period_id, 30_000).unwrap();
    let count: i64 = store
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM quota_window_reported WHERE period_id = ?1",
            [period_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn finite_retention_prunes_the_residual_and_learn_cursor_with_their_period() {
    let store = memory_store();
    let account_key = account('a');
    let now = 45 * 86_400;
    let old = now - 40 * 86_400;
    let period_id = insert_period(&store, &account_key, old - 18_000, old, old);
    store
        .lock()
        .execute(
            "INSERT INTO provider_usage_observation (
                 period_id, provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, observed_at_epoch, used_percent, is_fresh,
                 is_authoritative, confidence, source_id
             ) VALUES (?1, ?2, ?3, 'five-hour', 'rolling', 'primaryShort',
                       'account', 'account', ?4, 5.0, 1, 1, 'high', 'test')",
            params![period_id, PROVIDER, account_key, old],
        )
        .unwrap();
    store
        .advance_learn_cursor(period_id, old)
        .expect("advances the cursor");
    store
        .upsert_limit_residual(period_id, old, 5.0, 4.0)
        .expect("stores the residual");
    #[cfg(feature = "analytics")]
    store
        .mark_quota_window_reported(period_id, old)
        .expect("marks the window reported");

    // PRAGMA foreign_keys is on for the store; a residual or cursor row left
    // dangling on a deleted period would raise an error here, not just a
    // stale row, so `.unwrap()` below is itself the no-error assertion.
    let removed = Store::apply_provider_usage_retention_in(&store.lock(), 30, now).unwrap();
    assert_eq!(removed, 1, "the 40-day-old observation is removed");

    let connection = store.lock();
    let count = |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(
        count("provider_usage_period"),
        0,
        "the orphaned period is removed"
    );
    assert_eq!(
        count("provider_limit_residual"),
        0,
        "the residual is pruned with its period, not left dangling"
    );
    assert_eq!(
        count("provider_limit_learn_cursor"),
        0,
        "the cursor is pruned with its period"
    );
    assert_eq!(
        count("quota_window_reported"),
        0,
        "the quota-window-closed marker is pruned with its period"
    );
}

/// A synthetic period with only the fields [`lane_for_period`] reads set;
/// every other field is a harmless placeholder.
fn period_for_lane(
    window_role: &str,
    window_kind: &str,
    window_id: &str,
    scope_key: &str,
    scope_label: &str,
) -> ProviderUsagePeriod {
    ProviderUsagePeriod {
        id: 1,
        provider: PROVIDER.to_string(),
        account_key: account('a'),
        window_id: window_id.to_string(),
        window_kind: window_kind.to_string(),
        window_role: window_role.to_string(),
        scope_key: scope_key.to_string(),
        scope_label: scope_label.to_string(),
        duration_seconds: None,
        starts_at_epoch: None,
        resets_at_epoch: None,
        first_observed_epoch: 0,
        last_observed_epoch: 0,
    }
}

#[test]
fn lane_for_period_maps_the_account_wide_and_model_scoped_branches() {
    assert_eq!(
        lane_for_period(&period_for_lane(
            "primaryShort",
            "rolling",
            "five-hour",
            "account",
            "account"
        )),
        Some(LANE_FIVE_HOUR.to_string())
    );
    assert_eq!(
        lane_for_period(&period_for_lane(
            "primaryLong",
            "weekly",
            "seven-day",
            "account",
            "account"
        )),
        Some(LANE_WEEKLY.to_string())
    );
    // The slug comes from the window id when it has the expected shape.
    assert_eq!(
        lane_for_period(&period_for_lane(
            "supplemental",
            "weekly",
            "weekly-fable",
            "model:Fable",
            "Fable"
        )),
        Some("model:fable".to_string())
    );
    // A window id of another shape falls back to slugifying the scope label.
    assert_eq!(
        lane_for_period(&period_for_lane(
            "supplemental",
            "weekly",
            "unexpected-shape",
            "model:Codex Feature",
            "Codex Feature"
        )),
        Some("model:codex-feature".to_string())
    );
    // Anything else — a role or kind this app does not attribute a factor
    // to — carries no lane.
    assert_eq!(
        lane_for_period(&period_for_lane(
            "other:custom",
            "other:custom",
            "custom",
            "account",
            "account"
        )),
        None
    );
}

#[test]
fn candidate_periods_admit_a_model_scoped_weekly_period_and_exclude_an_account_scoped_supplemental()
{
    let store = memory_store();
    let account_key = account('a');
    let connection = store.lock();
    connection
        .execute(
            "INSERT INTO provider_usage_period (
                 provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, duration_seconds, starts_at_epoch,
                 resets_at_epoch, first_observed_epoch, last_observed_epoch
             ) VALUES (?1, ?2, 'weekly-fable', 'weekly', 'supplemental',
                       'model:Fable', 'Fable', 604800, 0, 604800, 100, 100)",
            params![PROVIDER, account_key],
        )
        .expect("inserts a model-scoped candidate period");
    connection
        .execute(
            "INSERT INTO provider_usage_period (
                 provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, duration_seconds, starts_at_epoch,
                 resets_at_epoch, first_observed_epoch, last_observed_epoch
             ) VALUES (?1, ?2, 'weekly-something', 'weekly', 'supplemental',
                       'account', 'account', 604800, 0, 604800, 100, 100)",
            params![PROVIDER, account_key],
        )
        .expect("inserts an account-scoped supplemental period");
    drop(connection);

    let periods = store
        .provider_limit_candidate_periods(0)
        .expect("query succeeds");
    assert_eq!(periods.len(), 1);
    assert_eq!(periods[0].scope_key, "model:Fable");
}

#[test]
fn latest_observation_plan_for_a_model_lane_reads_the_supplemental_windows_own_reading() {
    let store = memory_store();
    let account_key = account('a');
    let connection = store.lock();
    connection
        .execute(
            "INSERT INTO provider_usage_period (
                 provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, duration_seconds, starts_at_epoch,
                 resets_at_epoch, first_observed_epoch, last_observed_epoch
             ) VALUES (?1, ?2, 'weekly-fable', 'weekly', 'supplemental',
                       'model:Fable', 'Fable', 604800, 0, 604800, 100, 100)",
            params![PROVIDER, account_key],
        )
        .expect("inserts a model-scoped period");
    let model_period_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO provider_usage_period (
                 provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, duration_seconds, starts_at_epoch,
                 resets_at_epoch, first_observed_epoch, last_observed_epoch
             ) VALUES (?1, ?2, 'seven-day', 'weekly', 'primaryLong',
                       'account', 'account', 604800, 0, 604800, 100, 100)",
            params![PROVIDER, account_key],
        )
        .expect("inserts an account-wide weekly period");
    let account_period_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO provider_usage_observation (
                 period_id, provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, observed_at_epoch, used_percent, is_fresh,
                 is_authoritative, confidence, source_id, plan, plan_tier
             ) SELECT id, provider, account_key, window_id, window_kind, window_role,
                      scope_key, scope_label, 100, 5.0, 1, 1, 'high', 'test', 'max', 'model_tier'
                 FROM provider_usage_period WHERE id = ?1",
            params![model_period_id],
        )
        .expect("stores the model window's own reading");
    connection
        .execute(
            "INSERT INTO provider_usage_observation (
                 period_id, provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, observed_at_epoch, used_percent, is_fresh,
                 is_authoritative, confidence, source_id, plan, plan_tier
             ) SELECT id, provider, account_key, window_id, window_kind, window_role,
                      scope_key, scope_label, 100, 5.0, 1, 1, 'high', 'test', 'max', 'account_tier'
                 FROM provider_usage_period WHERE id = ?1",
            params![account_period_id],
        )
        .expect("stores the account window's own reading");
    drop(connection);

    let (plan, plan_tier) = store
        .latest_observation_plan(PROVIDER, &account_key, "model:fable")
        .expect("query succeeds")
        .expect("a reading exists");
    assert_eq!(plan.as_deref(), Some("max"));
    assert_eq!(
        plan_tier.as_deref(),
        Some("model_tier"),
        "a model lane reads the supplemental window's own observation, not the account window's"
    );
}

/// A per-step migration test for v52, kept for review; per repo convention
/// it may be deleted after merge, since the ladder test in `store::tests` is
/// the durable coverage.
#[test]
fn v52_widens_the_lane_check_and_resets_the_model_lane_cursor() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for &sql in &crate::store::schema::MIGRATIONS[..49] {
        connection.execute_batch(sql).unwrap();
    }
    connection.pragma_update(None, "user_version", 49).unwrap();
    connection
        .execute(
            "INSERT INTO provider_limit_factor_sample (
                 provider, account_key, lane, kind, from_epoch, to_epoch,
                 from_percent, to_percent, input_usd, output_usd, cache_read_usd,
                 cache_write_usd, turn_count, source_id, computed_at_epoch
             ) VALUES ('anthropic', ?1, 'weekly', 'delta', 0, 100, 0.0, 5.0,
                       1.0, 0.0, 0.0, 0.0, 1, 'test', 100)",
            params![account('a')],
        )
        .expect("inserts a pre-migration sample row");
    connection
        .execute(
            "INSERT INTO provider_limit_factor_point (
                 provider, account_key, lane, effective_at_epoch, usd_per_percent,
                 method, sample_count
             ) VALUES ('anthropic', ?1, 'weekly', 100, 0.2, 'delta', 1)",
            params![account('a')],
        )
        .expect("inserts a pre-migration point row");
    connection
        .execute(
            "INSERT INTO provider_usage_period (
                 id, provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, duration_seconds, starts_at_epoch,
                 resets_at_epoch, first_observed_epoch, last_observed_epoch
             ) VALUES (1, 'anthropic', ?1, 'weekly-fable', 'weekly', 'supplemental',
                       'model:fable', 'Fable', 604800, 0, 604800, 0, 100)",
            params![account('a')],
        )
        .expect("inserts a model-scoped period");
    connection
        .execute(
            "INSERT INTO provider_usage_period (
                 id, provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, duration_seconds, starts_at_epoch,
                 resets_at_epoch, first_observed_epoch, last_observed_epoch
             ) VALUES (2, 'anthropic', ?1, 'seven-day', 'weekly', 'primaryLong',
                       'account', 'account', 604800, 0, 604800, 0, 100)",
            params![account('a')],
        )
        .expect("inserts an account-wide weekly period");
    connection
        .execute(
            "INSERT INTO provider_limit_learn_cursor (period_id, learned_through_epoch)
             VALUES (1, 100)",
            [],
        )
        .expect("inserts the model-lane period's cursor row");
    connection
        .execute(
            "INSERT INTO provider_limit_learn_cursor (period_id, learned_through_epoch)
             VALUES (2, 100)",
            [],
        )
        .expect("inserts the weekly period's cursor row");

    let store = Store::from_connection(connection, Path::new("/tmp/antiburn-v52-test").into())
        .expect("migration reaches the head");
    assert_eq!(
        store.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );

    let connection = store.lock();
    let samples: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM provider_limit_factor_sample",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(samples, 1, "an existing sample row survives the rebuild");
    let points: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM provider_limit_factor_point",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(points, 1, "an existing point row survives the rebuild");

    connection
        .execute(
            "INSERT INTO provider_limit_factor_sample (
                 provider, account_key, lane, kind, from_epoch, to_epoch,
                 from_percent, to_percent, input_usd, output_usd, cache_read_usd,
                 cache_write_usd, turn_count, source_id, computed_at_epoch
             ) VALUES ('anthropic', ?1, 'model:fable', 'delta', 200, 300, 0.0, 5.0,
                       1.0, 0.0, 0.0, 0.0, 1, 'test', 300)",
            params![account('b')],
        )
        .expect("a model-scoped lane is now allowed by the widened check");

    let rejected = connection.execute(
        "INSERT INTO provider_limit_factor_sample (
             provider, account_key, lane, kind, from_epoch, to_epoch,
             from_percent, to_percent, input_usd, output_usd, cache_read_usd,
             cache_write_usd, turn_count, source_id, computed_at_epoch
         ) VALUES ('anthropic', ?1, 'bogus', 'delta', 400, 500, 0.0, 5.0,
                   1.0, 0.0, 0.0, 0.0, 1, 'test', 500)",
        params![account('c')],
    );
    assert!(
        rejected.is_err(),
        "an unrecognized lane still fails the check"
    );

    let model_lane_cursor: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM provider_limit_learn_cursor WHERE period_id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        model_lane_cursor, 0,
        "the model-scoped period's cursor row is gone, so the learner walks it again"
    );
    let weekly_cursor: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM provider_limit_learn_cursor WHERE period_id = 2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        weekly_cursor, 1,
        "the account-wide period's cursor row is untouched"
    );
}

/// Publish one turn at `ts_ms` under a chosen model, so a model-scope filter
/// test can use a model id [`crate::provider_usage::factor::model_matches_scope`]
/// does or does not match.
fn insert_turn_with_model(
    store: &Store,
    key: &SessionKey,
    ts_ms: i64,
    input_tokens: i64,
    model: &str,
) {
    let connection = store.lock();
    connection
        .execute(
            "INSERT OR IGNORE INTO session_evidence (
                 environment_key, agent, session_id, status, published_fence
             ) VALUES (?1, ?2, ?3, 'ready', 1)",
            params![key.environment_key, key.agent, key.session_id],
        )
        .expect("publishes synthetic evidence");
    connection
        .execute(
            "INSERT INTO turn (
                 environment_key, agent, session_id, claim_fence, source_key,
                 thread_id, turn_index, scope, role, ts_ms, model, effort, speed,
                 input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
                 is_compaction_boundary, message_id, uuid, parent_uuid
             ) VALUES (?1, ?2, ?3, 1, 'synthetic', 'synthetic', 0, 'main', 'assistant',
                       ?4, ?5, NULL, NULL, ?6, 0, 0, 0, 0, NULL, NULL, NULL)",
            params![
                key.environment_key,
                key.agent,
                key.session_id,
                ts_ms,
                model,
                input_tokens
            ],
        )
        .expect("stores synthetic turn");
}

/// A turn claimed under a fence that never became `published_fence`, so the
/// bucketed query's `session_evidence` join excludes it.
fn insert_unpublished_turn(store: &Store, key: &SessionKey, ts_ms: i64, input_tokens: i64) {
    let connection = store.lock();
    connection
        .execute(
            "INSERT OR IGNORE INTO session_evidence (
                 environment_key, agent, session_id, status, published_fence
             ) VALUES (?1, ?2, ?3, 'ready', 1)",
            params![key.environment_key, key.agent, key.session_id],
        )
        .expect("publishes synthetic evidence");
    connection
        .execute(
            "INSERT INTO turn (
                 environment_key, agent, session_id, claim_fence, source_key,
                 thread_id, turn_index, scope, role, ts_ms, model, effort, speed,
                 input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
                 is_compaction_boundary, message_id, uuid, parent_uuid
             ) VALUES (?1, ?2, ?3, 2, 'synthetic', 'synthetic', 0, 'main', 'assistant',
                       ?4, ?5, NULL, NULL, ?6, 0, 0, 0, 0, NULL, NULL, NULL)",
            params![
                key.environment_key,
                key.agent,
                key.session_id,
                ts_ms,
                MODEL,
                input_tokens
            ],
        )
        .expect("stores an unpublished synthetic turn");
}

#[test]
fn bucketed_query_groups_turns_by_fifteen_minute_bucket() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    observe_account(&store, &account('a'));
    // Both inside the first bucket (0..900s).
    insert_turn(&store, &key, 100_000, 100_000);
    insert_turn(&store, &key, 800_000, 100_000);
    // The next bucket (900..1800s).
    insert_turn(&store, &key, 900_000 + 1, 100_000);

    let rows = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), None, 0, 2_000)
        .expect("query succeeds")
        .expect("stays within the group bound");
    let mut buckets: Vec<i64> = rows.iter().map(|row| row.bucket_start_epoch).collect();
    buckets.sort_unstable();
    assert_eq!(buckets, vec![0, 900]);
    let first_bucket = rows
        .iter()
        .find(|row| row.bucket_start_epoch == 0)
        .expect("first bucket present");
    // Two turns of 100_000 input tokens each (200_000 total), merged into one
    // bucket. claude-opus-4-6 test pricing: 5e-6 dollars per input token.
    assert!((first_bucket.usd - 1.0).abs() < 1e-9);
    assert_eq!(first_bucket.turn_count, 2);
    assert_eq!(first_bucket.account, Resolved::Bound(account('a')));
}

fn record_cost(store: &Store, ts_ms: i64, micros: i64) {
    store
        .lock()
        .execute(
            "UPDATE turn SET recorded_cost_micros = ?1 WHERE ts_ms = ?2",
            params![micros, ts_ms],
        )
        .expect("records a cost");
}

#[test]
fn a_recorded_cost_replaces_the_catalog_price_in_limit_learning_and_the_hud_buckets() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    observe_account(&store, &account('a'));
    insert_turn(&store, &key, 100_000, 200_000);
    insert_turn(&store, &key, 200_000, 200_000);
    record_cost(&store, 100_000, 0);
    record_cost(&store, 200_000, 0);

    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert_eq!(dollars.len(), 1);
    assert_eq!(dollars[0].turn_count, 2);
    assert_eq!(dollars[0].input_usd, 0.0);

    let rows = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), None, 0, 2_000)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].usd, 0.0);
    assert!(rows[0].usage.iter().all(|&(_, usd)| usd == 0.0));
}

#[test]
fn a_group_with_an_unrecorded_turn_keeps_the_catalog_price_for_that_turn() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    observe_account(&store, &account('a'));
    insert_turn(&store, &key, 100_000, 200_000);
    insert_turn(&store, &key, 200_000, 200_000);
    record_cost(&store, 100_000, 250_000);

    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 1_000, None)
        .expect("query succeeds")
        .expect("stays within the group bound");
    // One turn lacks a recorded cost, so the group prices from the catalog:
    // 400_000 tokens at 5e-6 dollars each.
    assert!((dollars[0].input_usd - 2.0).abs() < 1e-9);

    let rows = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), None, 0, 2_000)
        .expect("query succeeds")
        .expect("stays within the group bound");
    let mut usage = rows[0].usage.clone();
    usage.sort_by_key(|&(ts_ms, _)| ts_ms);
    assert!((usage[0].1 - 0.25).abs() < 1e-9);
    assert!((usage[1].1 - 1.0).abs() < 1e-9);
}

#[test]
fn bucketed_query_excludes_a_turn_on_an_unpublished_fence() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    observe_account(&store, &account('a'));
    insert_unpublished_turn(&store, &key, 100_000, 100_000);

    let rows = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), None, 0, 2_000)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert!(
        rows.is_empty(),
        "a turn on an unpublished fence never joins session_evidence"
    );
}

#[test]
fn bucketed_query_drops_another_account_and_keeps_unbound_as_unbound() {
    let store = memory_store();
    let bound_to_other = insert_session(&store, "bound-to-other");
    bind_account(&store, &bound_to_other, &account('b'));
    insert_turn(&store, &bound_to_other, 100_000, 100_000);

    let unbound = insert_session(&store, "unbound");
    observe_account(&store, &account('a'));
    observe_account(&store, &account('b'));
    insert_turn(&store, &unbound, 100_000, 200_000);

    let rows = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), None, 0, 2_000)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert_eq!(rows.len(), 1, "only the unbound session's row survives");
    assert_eq!(rows[0].key, unbound);
    assert_eq!(rows[0].account, Resolved::Unbound);
}

#[test]
fn bucketed_query_model_scope_filter_excludes_a_non_matching_model() {
    let store = memory_store();
    let key = insert_session(&store, "session");
    observe_account(&store, &account('a'));
    insert_turn_with_model(&store, &key, 100_000, 100_000, "claude-fable-5-1");

    let matching = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), Some("Fable"), 0, 2_000)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert_eq!(matching.len(), 1);

    let non_matching = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), Some("Opus"), 0, 2_000)
        .expect("query succeeds")
        .expect("stays within the group bound");
    assert!(non_matching.is_empty());
}

#[test]
fn quota_periods_for_lane_returns_overlapping_periods_plus_the_anchor_before_the_range() {
    let store = memory_store();
    let account_key = account('a');
    // Anchor: resets well before the query range.
    insert_period(&store, &account_key, 0, 1_000, 1_000);
    // Overlaps the query range.
    insert_period(&store, &account_key, 5_000, 6_000, 6_000);

    let periods = store
        .quota_periods_for_lane(PROVIDER, &account_key, LANE_FIVE_HOUR, 4_000, 7_000)
        .expect("query succeeds");
    let mut resets: Vec<i64> = periods
        .iter()
        .filter_map(|period| period.resets_at_epoch)
        .collect();
    resets.sort_unstable();
    assert_eq!(
        resets,
        vec![1_000, 6_000],
        "the anchor and the overlapping period both come back"
    );
}

#[test]
fn factor_points_for_lane_orders_by_effective_at_epoch() {
    let store = memory_store();
    let lane = LANE_WEEKLY;
    for effective_at_epoch in [2_000, 1_000, 3_000] {
        store
            .upsert_factor_point(&FactorPoint {
                id: 0,
                provider: PROVIDER.to_string(),
                account_key: account('a'),
                lane: lane.to_string(),
                effective_at_epoch,
                usd_per_percent: 0.1,
                method: "delta".to_string(),
                sample_count: 1,
                plan: None,
                plan_tier: None,
            })
            .unwrap();
    }
    let points = store
        .factor_points_for_lane(PROVIDER, &account('a'), lane)
        .expect("query succeeds");
    let epochs: Vec<i64> = points
        .iter()
        .map(|point| point.effective_at_epoch)
        .collect();
    assert_eq!(epochs, vec![1_000, 2_000, 3_000]);
}

#[test]
fn attributed_turn_epochs_rounds_to_the_minute_and_respects_the_two_step_rule() {
    let store = memory_store();
    let bound = insert_session(&store, "bound");
    observe_account(&store, &account('a'));
    insert_turn(&store, &bound, 100_123, 1);
    insert_turn(&store, &bound, 100_456, 1);

    let other = insert_session(&store, "other");
    bind_account(&store, &other, &account('b'));
    insert_turn(&store, &other, 200_000, 1);

    let epochs = store
        .attributed_turn_epochs(PROVIDER, &account('a'), 0, 1_000)
        .expect("query succeeds");
    // Both turns (100_123ms and 100_456ms) round into the same minute
    // (60..120s) and dedupe to one epoch; the session bound to a different
    // account contributes nothing.
    assert_eq!(epochs, vec![60]);
}

#[test]
fn attributed_turn_minutes_for_account_matches_attributed_turn_epochs_for_every_account() {
    let store = memory_store();
    let bound = insert_session(&store, "bound");
    observe_account(&store, &account('a'));
    insert_turn(&store, &bound, 100_123, 1);
    insert_turn(&store, &bound, 100_456, 1);

    let other = insert_session(&store, "other");
    bind_account(&store, &other, &account('b'));
    insert_turn(&store, &other, 200_000, 1);

    let minutes = store
        .attributed_turn_minutes(0, 1_000)
        .expect("scan succeeds");
    for account_key in [account('a'), account('b')] {
        assert_eq!(
            minutes.for_account(PROVIDER, &account_key),
            store
                .attributed_turn_epochs(PROVIDER, &account_key, 0, 1_000)
                .expect("query succeeds"),
            "the shared scan must resolve the same epochs as the per-account query"
        );
    }
}

#[test]
fn active_minutes_match_scan_for_partial_windows_and_accounts() {
    let store = memory_store();
    observe_account(&store, &account('a'));
    let negative = insert_session(&store, "negative-time");
    insert_turn(&store, &negative, -1, 1);
    for (environment, id, binding) in [
        ("native", "fallback", None),
        ("native", "bound", Some(account('b'))),
        ("ssh:test", "remote", Some(account('a'))),
    ] {
        let key = insert_session_in_environment(&store, environment, id, AGENT);
        if let Some(binding) = binding {
            bind_account(&store, &key, &binding);
        }
        for timestamp in [
            0, 1, 2, 59_999, 60_000, 60_001, 100_123, 100_456, 120_000, 200_000,
        ] {
            insert_turn(&store, &key, timestamp, 1);
        }
    }
    let windows = [
        (0, 1000),
        (0, 60),
        (60, 120),
        (100, 101),
        (100, 100),
        (-60, 1),
        (200, 201),
        (0, 0),
    ];
    let expected: Vec<_> = windows
        .iter()
        .map(|&(from, to)| store.attributed_turn_minutes(from, to).unwrap())
        .collect();
    super::super::active_minutes::rebuild_in(
        &store.lock(),
        &SessionKey::new("native", AGENT, "bound"),
    )
    .unwrap();
    for (&(from, to), expected) in windows.iter().zip(&expected) {
        let partial = store.attributed_turn_minutes(from, to).unwrap();
        assert_eq!(partial.minutes_by_session, expected.minutes_by_session);
    }
    store.backfill_active_minutes().unwrap();
    for ((from, to), expected) in windows.into_iter().zip(expected) {
        let actual = store.attributed_turn_minutes(from, to).unwrap();
        assert_eq!(
            actual.minutes_by_session, expected.minutes_by_session,
            "{from}..{to}"
        );
        assert_eq!(actual.accounts_by_session, expected.accounts_by_session);
        assert_eq!(actual.known_accounts, expected.known_accounts);
        for key in [account('a'), account('b'), account('c')] {
            assert_eq!(
                actual.for_account(PROVIDER, &key),
                expected.for_account(PROVIDER, &key)
            );
        }
    }
}

#[test]
fn quota_accounts_reports_label_has_factor_and_the_current_open_period() {
    let store = memory_store();
    let account_key = account('a');
    // A weekly period still open at `now`.
    insert_period(&store, &account_key, 0, 10_000, 10_000);
    store
        .lock()
        .execute(
            "UPDATE provider_usage_period SET window_role = 'primaryLong'
              WHERE account_key = ?1",
            params![account_key],
        )
        .unwrap();
    // A five-hour period, inserted after the update above so it keeps its
    // default primaryShort role, and a model-scoped lane, so the account
    // carries all three lane kinds for the ordering assertion below.
    insert_period(&store, &account_key, 0, 18_000, 18_000);
    store
        .lock()
        .execute(
            "INSERT INTO provider_usage_period (
                 provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, duration_seconds, starts_at_epoch,
                 resets_at_epoch, first_observed_epoch, last_observed_epoch
             ) VALUES (?1, ?2, 'weekly-zeta', 'weekly', 'supplemental',
                       'model:zeta', 'Zeta', 604800, 0, 604800, 100, 100)",
            params![PROVIDER, account_key],
        )
        .unwrap();
    store
        .upsert_factor_point(&FactorPoint {
            id: 0,
            provider: PROVIDER.to_string(),
            account_key: account_key.clone(),
            lane: LANE_WEEKLY.to_string(),
            effective_at_epoch: 1,
            usd_per_percent: 0.2,
            method: "delta".to_string(),
            sample_count: 1,
            plan: None,
            plan_tier: None,
        })
        .unwrap();

    let accounts = store.quota_accounts(5_000).expect("query succeeds");
    let account = accounts
        .iter()
        .find(|account| account.account_key == account_key)
        .expect("the account is reported");
    let lane_names: Vec<&str> = account
        .lanes
        .iter()
        .map(|lane| lane.lane.as_str())
        .collect();
    assert_eq!(
        lane_names,
        vec![LANE_WEEKLY, LANE_FIVE_HOUR, "model:zeta"],
        "lanes are ordered weekly, then fiveHour, then model lanes alphabetically"
    );
    let lane = account
        .lanes
        .iter()
        .find(|lane| lane.lane == LANE_WEEKLY)
        .expect("the weekly lane is reported");
    assert_eq!(lane.label, "Weekly");
    assert!(lane.has_factor);
    assert_eq!(
        lane.current_period,
        Some((0, 10_000)),
        "the period has not reset yet"
    );
    assert_eq!(
        lane.first_observed_epoch, 10_000,
        "the lane's first reading is the earliest across its periods"
    );

    // Once `now` passes the reset, the same period no longer counts as open.
    let accounts_after_reset = store.quota_accounts(20_000).expect("query succeeds");
    let lane_after_reset = accounts_after_reset
        .iter()
        .find(|account| account.account_key == account_key)
        .and_then(|account| account.lanes.iter().find(|lane| lane.lane == LANE_WEEKLY))
        .expect("the weekly lane is still reported");
    assert_eq!(
        lane_after_reset.current_period, None,
        "a period that has already reset is not the current one"
    );
}

#[test]
fn shared_turn_input_preserves_model_speed_and_account_scopes_without_a_store_lock() {
    let store = memory_store();
    let key = insert_session(&store, "mixed-models");
    bind_account(&store, &key, &account('a'));
    insert_turn(&store, &key, 100_000, 100_000);
    insert_turn(&store, &key, 200_000, 100_000);
    store
        .lock()
        .execute("UPDATE turn SET speed = 'fast' WHERE ts_ms = 200000", [])
        .unwrap();
    insert_turn_with_model(&store, &key, 300_000, 100_000, "claude-fable-5-1");
    let other = insert_session(&store, "other-account");
    bind_account(&store, &other, &account('b'));
    insert_turn(&store, &other, 100_000, 500_000);

    let input = store.quota_turn_input(0, 2_000).unwrap().unwrap();
    let expected = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 0, 2_000, Some("Opus"))
        .unwrap()
        .unwrap();
    let expected_usd = expected[0].input_usd;
    let _guard = store.lock();
    let dollars = input.for_account(PROVIDER, &account('a'), None);
    let all = dollars.by_bucket(None);
    let opus = dollars.by_bucket(Some("Opus"));
    let fable = dollars.by_bucket(Some("Fable"));
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].key, key);
    assert_eq!(all[0].turn_count, 3);
    assert_eq!(opus[0].turn_count, 2);
    assert!((opus[0].usd - expected_usd).abs() < 1e-9);
    assert!((all[0].usd - opus[0].usd - fable[0].usd).abs() < 1e-9);
    assert!(dollars.by_bucket(Some("unmatched-model")).is_empty());
    assert_eq!(
        input
            .for_account(PROVIDER, &account('b'), None)
            .by_bucket(None)[0]
            .key,
        other
    );
}

#[test]
fn shared_turn_input_keeps_provider_bindings_separate_and_preserves_unknown_accounts() {
    let store = memory_store();
    let claude = insert_session(&store, "claude");
    bind_account(&store, &claude, &account('a'));
    store.lock().execute(
        "INSERT INTO session_provider_account
         SELECT environment_key, agent, session_id, 'openai', ?1, provenance, confidence, first_seen_at
         FROM session_provider_account WHERE agent = ?2 AND session_id = ?3",
        params![account('b'), AGENT, claude.session_id],
    ).unwrap();
    insert_turn(&store, &claude, 100_000, 100_000);
    let codex = insert_session_for_agent(&store, "codex", "codex");
    bind_account(&store, &codex, &account('b'));
    store
        .lock()
        .execute(
            "UPDATE session_provider_account SET provider = 'openai' WHERE agent = 'codex'",
            [],
        )
        .unwrap();
    insert_turn_with_model(&store, &codex, 100_000, 100_000, "gpt-6-astra");
    let unbound = insert_session(&store, "unbound");
    insert_turn(&store, &unbound, 100_000, 100_000);
    observe_account(&store, &account('a'));
    observe_account(&store, &account('b'));
    let input = store.quota_turn_input(0, 2_000).unwrap().unwrap();
    let anthropic = input
        .for_account(PROVIDER, &account('a'), None)
        .by_bucket(None);
    assert_eq!(anthropic.len(), 2);
    assert!(
        anthropic
            .iter()
            .any(|row| row.key == claude && row.account == Resolved::Bound(account('a')))
    );
    assert!(
        anthropic
            .iter()
            .any(|row| row.key == unbound && row.account == Resolved::Unbound)
    );
    let openai = input
        .for_account("openai", &account('b'), None)
        .by_bucket(None);
    assert_eq!(openai.len(), 1);
    assert_eq!(openai[0].key, codex);
    assert_eq!(openai[0].account, Resolved::Bound(account('b')));
}

#[test]
fn shared_turn_input_keeps_the_group_limit_before_account_or_model_filtering() {
    let store = memory_store();
    let key = insert_session(&store, "many-buckets");
    observe_account(&store, &account('a'));
    insert_turn(&store, &key, 100_000, 1);
    {
        let connection = store.lock();
        connection.execute(
            "WITH RECURSIVE buckets(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM buckets WHERE n < ?1)
             INSERT INTO turn (environment_key, agent, session_id, claim_fence, source_key,
                 thread_id, turn_index, scope, role, ts_ms, model, input_tokens,
                 cache_read_tokens, cache_write_tokens, output_tokens, is_compaction_boundary)
             SELECT 'native', ?2, ?3, 1, 'synthetic', 'synthetic', n, 'main', 'assistant',
                 n * 900000 + 100000, ?4, 1, 0, 0, 0, 0 FROM buckets",
            params![MAX_ATTRIBUTION_GROUPS as i64, AGENT, key.session_id, MODEL],
        ).unwrap();
    }
    assert!(
        store
            .quota_turn_input(0, (MAX_ATTRIBUTION_GROUPS as i64 + 1) * 900)
            .unwrap()
            .is_none()
    );
}

#[test]
fn quota_buckets_keep_exact_usage_times_and_half_open_period_boundaries() {
    let store = memory_store();
    let key = insert_session(&store, "timed-usage");
    observe_account(&store, &account('a'));
    for timestamp in [99_999, 100_000, 100_001, 199_999, 200_000] {
        insert_turn(&store, &key, timestamp, 100_000);
    }
    insert_unpublished_turn(&store, &key, 150_000, 900_000);
    let rows = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), None, 100, 200)
        .unwrap()
        .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    let mut timestamps: Vec<_> = row.usage.iter().map(|&(ts, _)| ts).collect();
    timestamps.sort_unstable();
    assert_eq!(timestamps, vec![100_000, 100_001, 199_999]);
    assert!((row.usd - row.usage.iter().map(|&(_, usd)| usd).sum::<f64>()).abs() < 1e-9);
    let current = row.in_period(100, 200, 150).unwrap();
    assert_eq!(current.usage.len(), 2);
    assert!((current.usd - row.usd * 2.0 / 3.0).abs() < 1e-9);
}

#[test]
fn quota_turn_limit_rejects_a_truncated_result_even_inside_one_bucket() {
    const TEST_TURN_LIMIT: usize = 100;
    let store = memory_store();
    let key = insert_session(&store, "bounded-usage");
    observe_account(&store, &account('a'));
    insert_turn(&store, &key, 100_000, 1);
    store
        .lock()
        .execute(
            "WITH RECURSIVE turns(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM turns WHERE n < ?1)
         INSERT INTO turn (environment_key, agent, session_id, claim_fence, source_key,
             thread_id, turn_index, scope, role, ts_ms, model, input_tokens,
             cache_read_tokens, cache_write_tokens, output_tokens, is_compaction_boundary)
         SELECT 'native', ?2, ?3, 1, 'synthetic', 'synthetic', n, 'main', 'assistant',
             100000, ?4, 1, 0, 0, 0, 0 FROM turns",
            params![TEST_TURN_LIMIT as i64, AGENT, key.session_id, MODEL],
        )
        .unwrap();
    assert!(
        read_quota_turn_input(&store.lock(), 0, 900, i64::MAX, TEST_TURN_LIMIT)
            .unwrap()
            .is_none()
    );
    let exact = read_quota_turn_input(&store.lock(), 0, 900, i64::MAX, TEST_TURN_LIMIT + 1)
        .unwrap()
        .unwrap();
    assert_eq!(
        exact.rows.iter().map(|row| row.turn_count).sum::<i64>(),
        101
    );
}

#[test]
fn timed_quota_costs_keep_one_hour_cache_prices_and_do_not_invent_unknown_fast_prices() {
    let store = memory_store();
    let key = insert_session(&store, "timed-pricing");
    observe_account(&store, &account('a'));
    insert_turn_with_one_hour_cache_write(&store, &key, 100_000, 100_000);
    insert_turn(&store, &key, 200_000, 100_000);
    store
        .lock()
        .execute("UPDATE turn SET speed = 'fast' WHERE ts_ms = 200000", [])
        .unwrap();
    let rows = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), None, 0, 900)
        .unwrap()
        .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.usage.len(), 2);
    for &(ts_ms, usd) in &row.usage {
        let expected = if ts_ms == 100_000 {
            100_000.0
                * lookup_turn_pricing(MODEL, None)
                    .unwrap()
                    .input_cost_per_token
                * 2.0
        } else {
            assert!(lookup_turn_pricing(MODEL, Some("fast")).is_none());
            0.0
        };
        assert!((usd - expected).abs() < 1e-9);
    }
    assert!((row.usd - row.usage.iter().map(|&(_, usd)| usd).sum::<f64>()).abs() < 1e-9);
}

#[test]
fn timed_quota_costs_keep_a_known_fast_mode_price() {
    let store = memory_store();
    let key = insert_session_for_agent(&store, "fast-pricing", "codex");
    bind_account(&store, &key, &account('a'));
    store
        .lock()
        .execute(
            "UPDATE session_provider_account SET provider = 'openai'",
            [],
        )
        .unwrap();
    insert_turn_with_model(&store, &key, 100_000, 100_000, "gpt-6-astra");
    store
        .lock()
        .execute("UPDATE turn SET speed = 'fast'", [])
        .unwrap();
    let rows = store
        .attributed_turn_dollars_by_bucket("openai", &account('a'), None, 0, 900)
        .unwrap()
        .unwrap();
    let expected = lookup_turn_pricing("gpt-6-astra", Some("fast"))
        .unwrap()
        .input_cost_per_token
        * 100_000.0;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].usage, vec![(100_000, expected)]);
    assert_eq!(rows[0].usd, expected);
}

#[test]
fn quota_turn_input_keeps_a_small_account_beside_a_seventy_day_workload() {
    let store = memory_store();
    let small = insert_session(&store, "small-account");
    let busy = insert_session(&store, "busy-account");
    bind_account(&store, &small, &account('a'));
    bind_account(&store, &busy, &account('b'));
    insert_turn(&store, &small, 100_000, 100_000);
    insert_turn(&store, &busy, 1, 1);
    let workload = 70 * (240_000 / 30);
    let end_epoch = 70 * 86_400;
    store
        .lock()
        .execute(
            "WITH RECURSIVE turns(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM turns WHERE n < ?1)
         INSERT INTO turn (environment_key, agent, session_id, claim_fence, source_key,
             thread_id, turn_index, scope, role, ts_ms, model, input_tokens,
             cache_read_tokens, cache_write_tokens, output_tokens, is_compaction_boundary)
         SELECT 'native', ?2, ?3, 1, 'synthetic', 'synthetic', n, 'main', 'assistant',
             n * 10800, ?4, 1, 0, 0, 0, 0 FROM turns",
            params![workload - 1, AGENT, busy.session_id, MODEL],
        )
        .unwrap();
    let input = store
        .quota_turn_input(0, end_epoch)
        .unwrap()
        .expect("the full supported workload fits");
    assert_eq!(
        input.rows.iter().map(|row| row.turn_count).sum::<i64>(),
        workload + 1
    );
    let rows = input
        .for_account(PROVIDER, &account('a'), None)
        .by_bucket(None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, small);
    assert_eq!(rows[0].usage.len(), 1);
    assert!(rows[0].usd > 0.0);
    assert_eq!(
        input
            .for_account(PROVIDER, &account('b'), None)
            .by_bucket(None)
            .iter()
            .map(|row| row.turn_count)
            .sum::<i64>(),
        workload
    );
}

#[test]
fn timed_pricing_keeps_model_specific_provider_routes_before_period_clipping() {
    let hints = serde_json::json!([
        {"model": MODEL, "provider": "openrouter"},
        {"model": "gpt-6-astra", "provider": "openai"}
    ])
    .to_string();
    let row = |model: &str| BucketTurnRow {
        key: SessionKey::new("native", "opencode", "mixed-provider"),
        hints_json: Some(hints.clone()),
        accounts_json: "[]".into(),
        model: Some(model.into()),
        speed: None,
        input_tokens: 400_000,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        output_tokens: 0,
        turn_count: 2,
        cache_write_1h_tokens: 0,
        recorded_cost_micros: None,
        bucket_start_epoch: 0,
        timed_tokens: vec![
            [
                Some(100_000),
                Some(100_000),
                Some(0),
                Some(0),
                Some(0),
                Some(0),
                None,
            ],
            [
                Some(300_000),
                Some(300_000),
                Some(0),
                Some(0),
                Some(0),
                Some(0),
                None,
            ],
        ],
    };
    let mut input = QuotaTurnInput {
        rows: vec![row(MODEL), row("gpt-6-astra")],
        known_accounts: HashMap::from([
            (
                ("openrouter".into(), "opencode".into()),
                BTreeSet::from([account('a')]),
            ),
            (
                ("openai".into(), "opencode".into()),
                BTreeSet::from([account('b')]),
            ),
        ]),
    };
    for (provider, account_key, model) in [
        ("openrouter", account('a'), MODEL),
        ("openai", account('b'), "gpt-6-astra"),
    ] {
        let rows = input
            .for_account(provider, &account_key, None)
            .by_bucket(None);
        assert_eq!(rows.len(), 1);
        let expected = lookup_turn_pricing(model, None)
            .unwrap()
            .input_cost_per_token
            * 400_000.0;
        assert_eq!(rows[0].usd, expected);
        assert_eq!(rows[0].in_period(0, 900, 900).unwrap().usd, expected);
        assert_eq!(rows[0].in_period(0, 200, 200).unwrap().usd, expected / 4.0);
    }
    assert!(
        input
            .for_account("anthropic", &account('a'), None)
            .by_bucket(None)
            .is_empty()
    );

    input.rows[0].hints_json = Some(
        serde_json::json!([
            {"model": MODEL, "provider": "openrouter"},
            {"model": MODEL, "provider": "openai"}
        ])
        .to_string(),
    );
    assert!(
        input
            .for_account("openrouter", &account('a'), None)
            .by_bucket(None)
            .is_empty()
    );
    let openai = input
        .for_account("openai", &account('b'), None)
        .by_bucket(None);
    assert_eq!(openai.len(), 1);
    assert_eq!(openai[0].usage.len(), 2);
    assert_eq!(
        openai[0].usd,
        lookup_turn_pricing("gpt-6-astra", None)
            .unwrap()
            .input_cost_per_token
            * 400_000.0
    );
}

/// Per bucket, the dollars that fall inside each period, for one account.
fn dollars_by_period(
    input: &QuotaTurnInput,
    account_key: &str,
    model_scope: Option<&str>,
    periods: &[(i64, i64)],
) -> Vec<(SessionKey, i64, usize, f64)> {
    let rows = input
        .for_account(PROVIDER, account_key, model_scope)
        .by_bucket(model_scope);
    let mut dollars = Vec::new();
    for (index, &(start, reset)) in periods.iter().enumerate() {
        for row in &rows {
            if let Some(row) = row.in_period(start, reset, i64::MAX / 1_000) {
                dollars.push((row.key.clone(), row.bucket_start_epoch, index, row.usd));
            }
        }
    }
    dollars.sort_by(|left, right| (&left.0, left.1, left.2).cmp(&(&right.0, right.1, right.2)));
    dollars
}

fn timed_entries(input: &QuotaTurnInput) -> usize {
    input.rows.iter().map(|row| row.timed_tokens.len()).sum()
}

#[test]
fn minute_totals_give_the_same_period_dollars_as_the_turn_rows() {
    let store = memory_store();
    observe_account(&store, &account('a'));
    observe_account(&store, &account('b'));
    let first = insert_session(&store, "bound-a");
    let second = insert_session(&store, "bound-b");
    let unbound = insert_session(&store, "unbound");
    let remote = insert_session_in_environment(&store, "ssh:host", "remote", AGENT);
    bind_account(&store, &first, &account('a'));
    bind_account(&store, &second, &account('b'));
    for minute in 0..30_i64 {
        for (index, offset) in [0, 10_000, 29_999, 30_000, 45_500].into_iter().enumerate() {
            let ts_ms = 600_000 + minute * 60_000 + offset;
            let tokens = 1_000 + minute * 10 + index as i64;
            insert_turn(&store, &first, ts_ms, tokens);
            insert_turn_with_model(&store, &second, ts_ms + 1, tokens, "claude-sonnet-4-6");
            insert_turn_with_one_hour_cache_write(&store, &unbound, ts_ms + 2, tokens);
            insert_turn(&store, &remote, ts_ms + 3, tokens);
        }
    }
    {
        let connection = store.lock();
        // Mix recorded and unrecorded turns inside minutes and buckets.
        connection
            .execute(
                "UPDATE turn SET recorded_cost_micros = input_tokens * 7
                  WHERE ts_ms % 3 = 0 AND session_id != 'unbound'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE turn SET speed = 'fast' WHERE session_id = 'bound-b' AND ts_ms % 2 = 0",
                [],
            )
            .unwrap();
    }
    // Both boundaries of the first period, and the end of the second, fall
    // inside a minute that holds turns before and after them.
    let periods = [(1_050, 2_010), (601, 1_500), (0, 4_000)];
    insert_period(&store, &account('a'), 1_050, 2_010, 2_010);
    insert_period(&store, &account('b'), 640, 1_525, 1_525);
    let ranges = [(0, 4_000), (637, 2_333), (60, 3_600), (1_050, 2_010)];
    let old: Vec<_> = ranges
        .iter()
        .map(|&(from, to)| store.quota_turn_input(from, to).unwrap().unwrap())
        .collect();

    // A partial fill keeps the turn rows.
    super::super::minute_usage::rebuild_in(&store.lock(), &first).unwrap();
    for (&(from, to), old) in ranges.iter().zip(&old) {
        let partial = store.quota_turn_input(from, to).unwrap().unwrap();
        assert_eq!(timed_entries(&partial), timed_entries(old), "{from}..{to}");
    }

    store.backfill_minute_usage().unwrap();
    for (&(from, to), old) in ranges.iter().zip(&old) {
        let new = store.quota_turn_input(from, to).unwrap().unwrap();
        assert!(
            timed_entries(&new) < timed_entries(old),
            "{from}..{to} reads minute totals"
        );
        assert_eq!(
            new.rows.iter().map(|row| row.turn_count).sum::<i64>(),
            old.rows.iter().map(|row| row.turn_count).sum::<i64>(),
        );
        for (account_key, scope) in [
            (account('a'), None),
            (account('b'), None),
            (account('b'), Some("Sonnet")),
            (account('c'), None),
        ] {
            let expected = dollars_by_period(old, &account_key, scope, &periods);
            let actual = dollars_by_period(&new, &account_key, scope, &periods);
            assert_eq!(expected.len(), actual.len(), "{from}..{to}");
            for (expected, actual) in expected.iter().zip(&actual) {
                assert_eq!(
                    (&expected.0, expected.1, expected.2),
                    (&actual.0, actual.1, actual.2)
                );
                assert!(
                    (expected.3 - actual.3).abs() <= 1e-9 * expected.3.abs().max(1.0),
                    "{from}..{to} {expected:?} {actual:?}"
                );
            }
        }
        // The whole-group dollars are exact too.
        for (old_row, new_row) in old.rows.iter().zip(&new.rows) {
            assert_eq!(old_row.key, new_row.key);
            assert_eq!(old_row.bucket_start_epoch, new_row.bucket_start_epoch);
            assert_eq!(old_row.recorded_cost_micros, new_row.recorded_cost_micros);
            assert_eq!(old_row.input_tokens, new_row.input_tokens);
            assert_eq!(old_row.turn_count, new_row.turn_count);
        }
    }
}

#[test]
fn a_period_boundary_inside_a_minute_keeps_that_minute_s_turn_rows() {
    let store = memory_store();
    observe_account(&store, &account('a'));
    let key = insert_session(&store, "split-minute");
    bind_account(&store, &key, &account('a'));
    for ts_ms in [1_020_000, 1_040_000, 1_060_000, 1_080_000] {
        insert_turn(&store, &key, ts_ms, 100_000);
    }
    insert_period(&store, &account('a'), 1_050, 2_010, 2_010);
    store.backfill_minute_usage().unwrap();
    let input = store.quota_turn_input(0, 4_000).unwrap().unwrap();
    let rows = input
        .for_account(PROVIDER, &account('a'), None)
        .by_bucket(None);
    assert_eq!(rows.len(), 1);
    // The minute at 1,020 s is cut at 1,050 s: its two turns stay separate.
    let mut times: Vec<_> = rows[0].usage.iter().map(|&(ts, _)| ts).collect();
    times.sort_unstable();
    assert_eq!(times, vec![1_020_000, 1_040_000, 1_060_000, 1_080_000]);
    let inside = rows[0].in_period(1_050, 2_010, 3_000).unwrap();
    assert_eq!(inside.usage.len(), 2);
    assert!((inside.usd - rows[0].usd / 2.0).abs() < 1e-9);
}

fn insert_reading(store: &Store, period_id: i64, account_key: &str, at: i64, percent: f64) {
    store
        .lock()
        .execute(
            "INSERT INTO provider_usage_observation (
                 period_id, provider, account_key, window_id, window_kind, window_role,
                 scope_key, scope_label, observed_at_epoch, used_percent, is_fresh,
                 is_authoritative, confidence, source_id
             ) VALUES (?1, ?2, ?3, 'five-hour', 'rolling', 'primaryShort', 'account',
                       'account', ?4, ?5, 1, 1, 'high', 'synthetic')",
            params![period_id, PROVIDER, account_key, at, percent],
        )
        .expect("inserts a synthetic reading");
}

fn insert_factor_point(store: &Store, account_key: &str, at: i64, usd_per_percent: f64) {
    store
        .lock()
        .execute(
            "INSERT INTO provider_limit_factor_point (
                 provider, account_key, lane, effective_at_epoch, usd_per_percent,
                 method, sample_count
             ) VALUES (?1, ?2, 'fiveHour', ?3, ?4, 'delta', 1)",
            params![PROVIDER, account_key, at, usd_per_percent],
        )
        .expect("inserts a synthetic factor point");
}

/// A bucket's session, start, percent, and dollars.
type AllocatedBucket = (SessionKey, i64, Option<f64>, f64);

/// The meter allocation of one open period: each bucket's percent, the
/// unexplained segments, and each bucket's dollars.
fn allocation(
    store: &Store,
    input: &QuotaTurnInput,
    period: (i64, i64),
    now: i64,
) -> (Vec<AllocatedBucket>, Vec<(i64, f64)>) {
    use crate::provider_usage::quota::share::{ShareInput, share_period};
    let account_key = account('a');
    let mut rows: Vec<_> = input
        .for_account(PROVIDER, &account_key, None)
        .by_bucket(None)
        .into_iter()
        .filter_map(|row| row.in_period(period.0, period.1, now))
        .collect();
    rows.sort_by(|left, right| {
        (&left.key, left.bucket_start_epoch).cmp(&(&right.key, right.bucket_start_epoch))
    });
    let readings: Vec<(i64, f64)> = store
        .lock()
        .prepare(
            "SELECT observed_at_epoch, used_percent FROM provider_usage_observation
              WHERE is_authoritative = 1 ORDER BY observed_at_epoch",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let points = store
        .factor_points_for_lane(PROVIDER, &account_key, LANE_FIVE_HOUR)
        .unwrap();
    let buckets: Vec<&[(i64, f64)]> = rows.iter().map(|row| row.usage.as_slice()).collect();
    let shared = share_period(&ShareInput {
        start: period.0,
        reset: period.1,
        readings: &readings,
        buckets: &buckets,
        points: &points,
    });
    let rows = rows
        .iter()
        .zip(shared.bucket_percent)
        .map(|(row, percent)| (row.key.clone(), row.bucket_start_epoch, percent, row.usd))
        .collect();
    (rows, shared.unexplained)
}

/// Builds the store, reads the old and new allocation, and compares them.
fn assert_same_allocation(
    store: &Store,
    period: (i64, i64),
    now: i64,
    expected_total_percent: Option<f64>,
) {
    let old = store.quota_turn_input(0, 4_000).unwrap().unwrap();
    store.backfill_minute_usage().unwrap();
    let new = store.quota_turn_input(0, 4_000).unwrap().unwrap();
    assert!(timed_entries(&new) <= timed_entries(&old));
    let (old_rows, old_unexplained) = allocation(store, &old, period, now);
    let (new_rows, new_unexplained) = allocation(store, &new, period, now);
    assert_eq!(old_rows.len(), new_rows.len());
    for (old, new) in old_rows.iter().zip(&new_rows) {
        assert_eq!((&old.0, old.1), (&new.0, new.1));
        let same = match (old.2, new.2) {
            (Some(left), Some(right)) => (left - right).abs() < 1e-9,
            (left, right) => left == right,
        };
        assert!(same, "percent {old:?} {new:?}");
        assert!((old.3 - new.3).abs() < 1e-9, "dollars {old:?} {new:?}");
    }
    assert_eq!(old_unexplained.len(), new_unexplained.len());
    for (old, new) in old_unexplained.iter().zip(&new_unexplained) {
        assert_eq!(old.0, new.0);
        assert!((old.1 - new.1).abs() < 1e-9);
    }
    if let Some(expected) = expected_total_percent {
        let total: f64 = old_rows.iter().filter_map(|row| row.2).sum::<f64>()
            + old_unexplained
                .iter()
                .map(|&(_, percent)| percent)
                .sum::<f64>();
        assert!((total - expected).abs() < 1e-9, "fixture total {total}");
    }
}

fn two_sessions(store: &Store) -> (SessionKey, SessionKey) {
    observe_account(store, &account('a'));
    let first = insert_session(store, "first");
    let second = insert_session(store, "second");
    bind_account(store, &first, &account('a'));
    bind_account(store, &second, &account('a'));
    (first, second)
}

#[test]
fn a_reading_inside_a_minute_splits_measured_and_tail_spend_as_before() {
    let store = memory_store();
    let (first, second) = two_sessions(&store);
    let period = insert_period(&store, &account('a'), 0, 900, 900);
    insert_turn(&store, &first, 140_000, 1);
    insert_turn(&store, &second, 160_000, 1);
    store
        .lock()
        .execute("UPDATE turn SET recorded_cost_micros = 1000000", [])
        .unwrap();
    insert_reading(&store, period, &account('a'), 150, 20.0);
    insert_factor_point(&store, &account('a'), 0, 1.0);
    // $1 is measured, $1 is tail: 20% plus 1%.
    assert_same_allocation(&store, (0, 900), 500, Some(21.0));
}

#[test]
fn a_reading_at_a_minute_start_separates_its_own_instant_from_later_turns() {
    let store = memory_store();
    let (first, second) = two_sessions(&store);
    let period = insert_period(&store, &account('a'), 0, 900, 900);
    insert_turn(&store, &first, 120_000, 1);
    insert_turn(&store, &second, 120_500, 1);
    store
        .lock()
        .execute("UPDATE turn SET recorded_cost_micros = 1000000", [])
        .unwrap();
    insert_reading(&store, period, &account('a'), 120, 10.0);
    insert_factor_point(&store, &account('a'), 0, 1.0);
    assert_same_allocation(&store, (0, 900), 500, Some(11.0));
}

#[test]
fn a_factor_change_inside_a_minute_prices_the_tail_as_before() {
    let store = memory_store();
    let (first, second) = two_sessions(&store);
    let period = insert_period(&store, &account('a'), 0, 900, 900);
    insert_turn(&store, &first, 60_000, 1);
    insert_turn(&store, &first, 190_000, 1);
    insert_turn(&store, &second, 230_000, 1);
    store
        .lock()
        .execute("UPDATE turn SET recorded_cost_micros = 1000000", [])
        .unwrap();
    insert_reading(&store, period, &account('a'), 90, 5.0);
    insert_factor_point(&store, &account('a'), 0, 1.0);
    insert_factor_point(&store, &account('a'), 210, 4.0);
    // The tail holds $1 at the first factor and $1 at the second.
    assert_same_allocation(&store, (0, 900), 500, Some(5.0 + 1.0 + 0.25));
}

#[test]
fn the_now_cutoff_inside_a_minute_clips_the_same_turns_as_the_turn_rows() {
    let store = memory_store();
    observe_account(&store, &account('a'));
    let key = insert_session(&store, "clipped");
    bind_account(&store, &key, &account('a'));
    for ts_ms in [60_500, 119_000, 119_500, 150_000] {
        insert_turn(&store, &key, ts_ms, 100_000);
    }
    insert_period(&store, &account('a'), 0, 900, 900);
    for (to, now) in [(120, 119), (120, 60), (300, 119), (300, 120), (300, 149)] {
        let old = store.quota_turn_input_until(0, to, now).unwrap().unwrap();
        let dollars = |input: &QuotaTurnInput| -> f64 {
            input
                .for_account(PROVIDER, &account('a'), None)
                .by_bucket(None)
                .iter()
                .filter_map(|row| row.in_period(0, 900, now))
                .map(|row| row.usd)
                .sum()
        };
        let expected = dollars(&old);
        store.backfill_minute_usage().unwrap();
        let new = store.quota_turn_input_until(0, to, now).unwrap().unwrap();
        assert!(
            (dollars(&new) - expected).abs() < 1e-9,
            "to {to} now {now}: {} vs {expected}",
            dollars(&new)
        );
        store
            .lock()
            .execute(
                "DELETE FROM setting WHERE key = 'internal:minuteUsageBackfillV2'",
                [],
            )
            .unwrap();
    }
}

fn insert_other_role_turn(
    store: &Store,
    key: &SessionKey,
    role: &str,
    ts_ms: i64,
    recorded_cost_micros: Option<i64>,
) {
    store
        .lock()
        .execute(
            "INSERT INTO turn (
                 environment_key, agent, session_id, claim_fence, source_key,
                 thread_id, turn_index, scope, role, ts_ms, model, input_tokens,
                 cache_read_tokens, cache_write_tokens, output_tokens,
                 is_compaction_boundary, recorded_cost_micros
             ) VALUES (?1, ?2, ?3, 1, 'synthetic', 'synthetic', ?4, 'main', ?5,
                       ?6, ?7, 50000, 0, 0, 0, 0, ?8)",
            params![
                key.environment_key,
                key.agent,
                key.session_id,
                ts_ms,
                role,
                ts_ms,
                MODEL,
                recorded_cost_micros
            ],
        )
        .expect("stores a turn of another role");
}

#[test]
fn turns_of_another_role_count_once_in_cut_and_uncut_minutes() {
    let store = memory_store();
    observe_account(&store, &account('a'));
    let key = insert_session(&store, "other-roles");
    bind_account(&store, &key, &account('a'));
    for ts_ms in [70_000, 130_000, 1_030_000, 1_070_000] {
        insert_turn(&store, &key, ts_ms, 100_000);
    }
    // The period starts at 1,050 s, so the minute at 1,020 s is cut. The
    // others are not.
    insert_period(&store, &account('a'), 1_050, 2_010, 2_010);
    insert_other_role_turn(&store, &key, "user", 72_000, Some(2_000_000));
    insert_other_role_turn(&store, &key, "system", 132_000, None);
    insert_other_role_turn(&store, &key, "tool", 1_040_000, Some(3_000_000));
    insert_other_role_turn(&store, &key, "user", 1_060_000, None);
    // Outside the population: no tokens, or an unpublished fence.
    insert_unpublished_turn(&store, &key, 74_000, 900_000);
    let periods = [(1_050, 2_010), (0, 4_000)];
    let dollars = |input: &QuotaTurnInput, period: (i64, i64)| -> f64 {
        input
            .for_account(PROVIDER, &account('a'), None)
            .by_bucket(None)
            .iter()
            .filter_map(|row| row.in_period(period.0, period.1, 4_000))
            .map(|row| row.usd)
            .sum()
    };
    let old = store
        .quota_turn_input_until(0, 4_000, 4_000)
        .unwrap()
        .unwrap();
    let expected: Vec<f64> = periods
        .iter()
        .map(|&period| dollars(&old, period))
        .collect();
    // Four assistant turns, four turns of another role, priced per turn.
    assert!(expected[1] > 5.0 + 4.0 * 0.4, "{expected:?}");
    // A partial fill keeps the turn rows.
    store.backfill_minute_usage().unwrap();
    let new = store
        .quota_turn_input_until(0, 4_000, 4_000)
        .unwrap()
        .unwrap();
    assert!(timed_entries(&new) <= timed_entries(&old));
    assert_eq!(
        new.rows.iter().map(|row| row.turn_count).sum::<i64>(),
        old.rows.iter().map(|row| row.turn_count).sum::<i64>()
    );
    for (period, expected) in periods.iter().zip(expected) {
        assert!(
            (dollars(&new, *period) - expected).abs() < 1e-9,
            "{period:?}"
        );
    }
}

#[test]
fn the_other_role_read_uses_the_partial_index() {
    let store = memory_store();
    store.backfill_minute_usage().unwrap();
    let connection = store.lock();
    let plan: Vec<String> = connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT t.ts_ms FROM turn t INDEXED BY turn_usage_non_assistant_timestamp
              WHERE t.ts_ms >= 0 AND t.ts_ms < 1000 AND t.role IS NOT 'assistant'
                AND (t.input_tokens > 0 OR t.output_tokens > 0
                     OR t.cache_read_tokens > 0 OR t.cache_write_tokens > 0)",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("turn_usage_non_assistant_timestamp")),
        "{plan:?}"
    );
}

#[test]
fn history_reads_skip_unpublished_turn_generations() {
    let store = memory_store();
    let key = insert_session(&store, "published-history");
    observe_account(&store, &account('a'));
    insert_turn(&store, &key, 100_123, 200_000);
    let connection = store.lock();
    let queries: [(&str, Vec<rusqlite::types::Value>); 3] = [
        (
            ATTRIBUTED_TURN_SQL,
            vec![
                1.into(),
                200_000.into(),
                100.into(),
                PROVIDER.to_owned().into(),
            ],
        ),
        (
            ATTRIBUTED_TURN_BUCKET_SQL,
            vec![1.into(), 200_000.into(), 100.into(), 100.into()],
        ),
        (ATTRIBUTED_TURN_MINUTES_SQL, vec![1.into(), 200_000.into()]),
    ];
    let read = |sql: &str, values: &[rusqlite::types::Value]| {
        let mut statement = connection.prepare(sql).unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map(rusqlite::params_from_iter(values), |row| {
                (0..columns)
                    .map(|column| row.get::<_, rusqlite::types::Value>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (
            rows,
            statement.get_status(rusqlite::StatementStatus::VmStep),
        )
    };
    let before: Vec<_> = queries
        .iter()
        .map(|(sql, values)| read(sql, values))
        .collect();
    connection
        .execute_batch(
            "WITH RECURSIVE n(value) AS (
             VALUES(1) UNION ALL SELECT value + 1 FROM n WHERE value < 10000
         )
         INSERT INTO turn (environment_key, agent, session_id, claim_fence, source_key,
             thread_id, turn_index, scope, role, ts_ms, model, input_tokens,
             cache_read_tokens, cache_write_tokens, output_tokens, is_compaction_boundary)
         SELECT 'native', 'claude-code', 'published-history', 2, 'stale', 'stale',
                value, 'main', 'assistant', 100123, 'claude-opus-4-6', 999999, 0, 0, 0, 0 FROM n;",
        )
        .unwrap();
    for ((sql, values), (rows, steps)) in queries.iter().zip(before) {
        let (after_rows, after_steps) = read(sql, values);
        assert_eq!(after_rows, rows);
        assert!(
            after_steps < steps + 200,
            "unpublished generations increase history work: {steps} -> {after_steps}"
        );
    }
}

#[test]
fn narrow_learning_reads_do_not_scan_unrelated_sessions() {
    let store = memory_store();
    let key = insert_session(&store, "recent");
    observe_account(&store, &account('a'));
    insert_turn(&store, &key, 100_123, 200_000);
    let connection = store.lock();
    let steps = || {
        let sql = history_scan_sql(
            &connection,
            90_000,
            110_000,
            ATTRIBUTED_TURN_SQL,
            ATTRIBUTED_TURN_TIMESTAMP_SQL,
        )
        .expect("selects the narrow access path");
        assert_eq!(sql, ATTRIBUTED_TURN_TIMESTAMP_SQL);
        let mut statement = connection.prepare(sql).expect("prepares narrow history");
        let mut rows = statement
            .query(params![
                90_000,
                110_000,
                MAX_ATTRIBUTION_GROUPS as i64 + 1,
                PROVIDER
            ])
            .expect("reads narrow history");
        assert!(rows.next().unwrap().is_some());
        assert!(rows.next().unwrap().is_none());
        drop(rows);
        statement.get_status(rusqlite::StatementStatus::VmStep)
    };
    let before = steps();
    connection
        .execute_batch(
            "WITH RECURSIVE n(value) AS (
             VALUES(1) UNION ALL SELECT value + 1 FROM n WHERE value < 2000
         )
         INSERT INTO session (
             environment_key, agent, session_id, source_kind, source_label,
             first_seen_at, last_seen_at
         )
         SELECT 'native', 'claude-code', 'archived-' || value, 'inline', 'synthetic',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z' FROM n;
         INSERT INTO session_evidence (
             environment_key, agent, session_id, status, published_fence
         )
         SELECT environment_key, agent, session_id, 'ready', 1 FROM session
          WHERE session_id LIKE 'archived-%';",
        )
        .expect("adds unrelated session evidence");
    let after = steps();
    assert!(
        after < before + 100,
        "unrelated sessions increase narrow work: {before} -> {after}"
    );
    drop(connection);
    let dollars = store
        .attributed_turn_dollars_between(PROVIDER, &account('a'), 90, 110, None)
        .unwrap()
        .unwrap();
    assert_eq!(dollars.len(), 1);
    assert_eq!(dollars[0].turn_count, 1);
    assert!((dollars[0].input_usd - 1.0).abs() < 1e-9);
}

#[test]
fn all_history_paths_compare_range_size_with_a_large_catalog() {
    let store = memory_store();
    let key = insert_session(&store, "crossover");
    observe_account(&store, &account('a'));
    insert_turn(&store, &key, 100_123, 200_000);
    let connection = store.lock();
    connection
        .execute_batch(
            "WITH RECURSIVE n(value) AS (
             VALUES(1) UNION ALL SELECT value + 1 FROM n WHERE value < 2000
         )
         INSERT INTO session (
             environment_key, agent, session_id, source_kind, source_label,
             first_seen_at, last_seen_at
         )
         SELECT 'native', 'claude-code', 'archived-' || value, 'inline', 'synthetic',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z' FROM n;
         INSERT INTO session_evidence (
             environment_key, agent, session_id, status, published_fence
         )
         SELECT environment_key, agent, session_id, 'ready', 1 FROM session
          WHERE session_id LIKE 'archived-%';",
        )
        .unwrap();
    let insert_stale = |count| {
        connection
            .execute(
                "WITH RECURSIVE n(value) AS (
                 VALUES(1) UNION ALL SELECT value + 1 FROM n WHERE value < ?1
             )
             INSERT INTO turn (
                 environment_key, agent, session_id, claim_fence, source_key,
                 thread_id, turn_index, scope, role, ts_ms, input_tokens,
                 cache_read_tokens, cache_write_tokens, output_tokens, is_compaction_boundary
             )
             SELECT 'native', 'claude-code', 'crossover', 2, 'stale',
                    'stale', value, 'main', 'assistant', 100123, 999999, 0, 0, 0, 0 FROM n",
                [count],
            )
            .unwrap();
    };
    insert_stale(TIMESTAMP_SCAN_TURN_LIMIT);
    let queries: [(&str, &str, Vec<rusqlite::types::Value>); 3] = [
        (
            ATTRIBUTED_TURN_MINUTES_SQL,
            ATTRIBUTED_TURN_MINUTES_TIMESTAMP_SQL,
            vec![90_000.into(), 110_000.into()],
        ),
        (
            ATTRIBUTED_TURN_BUCKET_SQL,
            ATTRIBUTED_TURN_BUCKET_TIMESTAMP_SQL,
            vec![
                90_000.into(),
                110_000.into(),
                20001.into(),
                (MAX_QUOTA_TURNS as i64 + 1).into(),
            ],
        ),
        (
            ATTRIBUTED_TURN_SQL,
            ATTRIBUTED_TURN_TIMESTAMP_SQL,
            vec![
                90_000.into(),
                110_000.into(),
                20001.into(),
                PROVIDER.to_string().into(),
            ],
        ),
    ];
    for (published, timestamp, values) in &queries {
        let chosen = history_scan_sql(&connection, 90_000, 110_000, published, timestamp).unwrap();
        assert_eq!(chosen, *timestamp);
        let read = |sql: &str| {
            let mut statement = connection.prepare(sql).unwrap();
            let columns = statement.column_count();
            statement
                .query_map(rusqlite::params_from_iter(values), |row| {
                    (0..columns)
                        .map(|column| row.get::<_, rusqlite::types::Value>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(read(chosen), read(published));
        assert_eq!(read(chosen).len(), 1);
    }
    insert_stale(2001);
    for (published, timestamp, _) in &queries {
        assert_eq!(
            history_scan_sql(&connection, 90_000, 110_000, published, timestamp).unwrap(),
            *published
        );
        for (start, end) in [(100_123, 100_124), (90_000, 100_122)] {
            assert_eq!(
                history_scan_sql(&connection, start, end, published, timestamp).unwrap(),
                *timestamp
            );
        }
    }
    drop(connection);
    assert_eq!(
        store
            .attributed_turn_epochs(PROVIDER, &account('a'), 90, 110)
            .unwrap(),
        vec![60]
    );
    let buckets = store
        .attributed_turn_dollars_by_bucket(PROVIDER, &account('a'), None, 90, 110)
        .unwrap()
        .unwrap();
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].turn_count, 1);
    assert!((buckets[0].usd - 1.0).abs() < 1e-9);
}

#[test]
fn quota_history_index_upgrade_preserves_timed_usage() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for migration in &crate::store::schema::MIGRATIONS[..80] {
        connection.execute_batch(migration).unwrap();
    }
    connection.pragma_update(None, "user_version", 80).unwrap();
    connection
        .execute(
            "INSERT INTO session (environment_key, agent, session_id, source_kind,
                 source_label, first_seen_at, last_seen_at)
             VALUES ('native', ?1, 'index-upgrade', 'inline', 'synthetic',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [AGENT],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO session_evidence (environment_key, agent, session_id,
                 status, published_fence)
             VALUES ('native', ?1, 'index-upgrade', 'ready', 1)",
            [AGENT],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO turn (environment_key, agent, session_id, claim_fence,
                 source_key, thread_id, turn_index, scope, role, ts_ms, model,
                 input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
                 is_compaction_boundary)
             VALUES ('native', ?1, 'index-upgrade', 1, 'synthetic', 'synthetic', 0,
                 'main', 'assistant', 100123, ?2, 200000, 0, 0, 0, 0)",
            params![AGENT, MODEL],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO provider_account_seen (agent, provider, account_key,
                 first_seen_epoch, last_seen_epoch) VALUES (?1, ?2, ?3, 1, 1)",
            params![AGENT, PROVIDER, account('a')],
        )
        .unwrap();
    connection
        .execute_batch(antiburn_local::analysis::TURN_RECORDED_COST_SQL)
        .unwrap();
    let before = read_quota_turn_input(&connection, 0, 1_000, 1_000, MAX_QUOTA_TURNS)
        .unwrap()
        .unwrap();
    assert_eq!(before.rows.len(), 1);
    assert!(!before.rows[0].timed_tokens.is_empty());
    let store = Store::from_connection(
        connection,
        Path::new("/tmp/antiburn-provider-limit-test").into(),
    )
    .unwrap();
    store.migrate().unwrap();
    assert_eq!(
        store.schema_version().unwrap(),
        crate::store::schema::MIGRATIONS.len() as i64
    );
    let index_exists: bool = store
        .lock()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema
                 WHERE type = 'index' AND name = 'turn_quota_published')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(index_exists);
    let after = store.quota_turn_input(0, 1_000).unwrap().unwrap();
    assert_eq!(after.rows.len(), before.rows.len());
    assert_eq!(after.known_accounts, before.known_accounts);
    for (after, before) in after.rows.iter().zip(&before.rows) {
        assert_eq!(after.turn_count, before.turn_count);
        assert_eq!(after.timed_tokens, before.timed_tokens);
        assert_eq!(after.input_tokens, before.input_tokens);
        assert_eq!(after.accounts_json, before.accounts_json);
    }
}
