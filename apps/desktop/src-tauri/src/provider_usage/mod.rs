//! Per-provider usage, derived from the sessions already on this machine.
//!
//! # Where the numbers come from
//!
//! Exactly one place: the local database. Each session's published turn rows
//! give its billable tokens per local day, model, and speed; the cached
//! session analysis gives its provider hints and, for a session without turn
//! rows, its whole billable-token breakdown per model. This module attributes
//! those models to providers ([`providers`]), sums them over the calendar
//! windows and the daily series, prices them against the active runtime
//! snapshot, and reports how confident the result is.
//!
//! # Its ceiling, and why
//!
//! This module's whole input is a `Vec` of rows the caller already read out
//! of the app's own SQLite file, plus a clock: it derives spend purely from
//! local transcript rows. It needs no provider figure, no credential, and no
//! call of any kind, because a transcript already records what was *spent*.
//! That is also its hard ceiling: an allowance, a percentage, a remaining
//! balance, and a reset time are none of them things a transcript states, so
//! they are absent from the payload rather than estimated into it.
//!
//! # How a session's tokens reach a day
//!
//! 1. **Tokens land on the local day their turn ran.** A thread that ran from
//!    Sunday to Wednesday adds each turn to that turn's day, and to each
//!    window that holds the day. A window or a day counts the session once,
//!    however many of its turns it holds. A turn older than the lookback
//!    adds to no window.
//! 2. **Without a turn timestamp, tokens land on the last-activity day.** A
//!    session with no published billable turn rows (not yet analyzed, or
//!    analyzed without a turn store) puts its whole cached breakdown on the
//!    day of its last activity. A turn row without a timestamp does the same.
//!    This is the previous rule, and it remains only where no turn time
//!    exists.

pub mod allowance;
pub(crate) mod codex_rollout_history;
pub(crate) mod factor;
pub mod live;
pub mod providers;
pub(crate) mod quota;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use antiburn_local::analysis::{price_breakdown, strip_window_tag, turn_pricing_key};
use antiburn_local::pricing::ModelTokens;
use time::{Date, OffsetDateTime, Time, UtcOffset};

use crate::dto::{
    ProviderAgentDayUsage, ProviderAgentUsage, ProviderUsage, ProviderUsageDay,
    ProviderUsageStaleness, ProviderUsageState, ProviderUsageSummary, ProviderUsageWindow,
    ProviderUsageWindows,
};
use crate::store::{UsageEvidenceRecord, iso_from_epoch};

use antiburn_local::analysis::ProviderHint;
use providers::{HintResolution, Route};
use serde::Deserialize;

/// How old a provider's newest session may be before its totals are marked as
/// describing the past rather than the present.
///
/// A day, because that is the span the narrowest window covers: once the
/// newest evidence is older than that, every rolling figure on the surface is
/// necessarily zero and the reader deserves to know why.
pub const STALE_EVIDENCE_SECS: i64 = 24 * 60 * 60;

/// Widest offset from UTC any real time zone uses (+14:00 / −12:00, clamped
/// symmetrically). Guards the offset the webview supplies, which is input.
const MAX_OFFSET_MINUTES: i32 = 14 * 60;

/// The three windows, as unix-second lower bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBounds {
    /// Local midnight today.
    pub today_start: i64,
    /// Local midnight six days before today — the trailing seven calendar days.
    pub week_start: i64,
    /// Local midnight on the first of the current month.
    pub month_start: i64,
    /// Local midnight twenty-nine days before today.
    pub last_30_days_start: i64,
    /// Local midnight thirty days before `last_30_days_start`: the start of
    /// the comparison period that precedes the trailing thirty days.
    pub previous_30_days_start: i64,
}

/// Seconds in one local day. Daylight-saving changes are ignored here, as
/// they are in [`window_bounds`].
pub(crate) const DAY: i64 = 86_400;

/// Days in each of the two daily series.
pub(crate) const SERIES_DAYS: usize = 30;

impl WindowBounds {
    /// The earliest instant any window reaches back to.
    fn earliest(&self) -> i64 {
        self.week_start
            .min(self.month_start)
            .min(self.last_30_days_start)
            .min(self.previous_30_days_start)
    }
}

/// The window bounds for `now`, in a reader whose clock is `utc_offset_minutes`
/// from UTC.
///
/// The offset is applied to past dates as well as today's, so a boundary that
/// falls on the far side of a daylight-saving change is off by that change —
/// an hour, once or twice a year, on a figure already labelled an estimate.
/// The alternative is a time-zone database, which is a large dependency for a
/// one-hour edge case.
pub fn window_bounds(now: i64, utc_offset_minutes: i32) -> WindowBounds {
    let offset = local_offset(utc_offset_minutes);
    let at = OffsetDateTime::from_unix_timestamp(now)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset);
    let date = at.date();

    let today_start = date
        .with_time(Time::MIDNIGHT)
        .assume_offset(offset)
        .unix_timestamp();
    let month_start = date
        .replace_day(1)
        .unwrap_or(date)
        .with_time(Time::MIDNIGHT)
        .assume_offset(offset)
        .unix_timestamp();

    WindowBounds {
        today_start,
        // Today plus the six days before it. Anchored on local midnight rather
        // than on `now` so the week does not slide through the day, which
        // would make the same session appear and disappear as the clock moves.
        week_start: today_start - 6 * 86_400,
        month_start,
        last_30_days_start: today_start - 29 * DAY,
        previous_30_days_start: today_start - 59 * DAY,
    }
}

/// The earliest activity timestamp the aggregation needs to read.
///
/// Exposed so the caller can bound its query instead of loading every retained
/// session and discarding most of them.
pub fn lookback_start(now: i64, utc_offset_minutes: i32) -> i64 {
    window_bounds(now, utc_offset_minutes).earliest()
}

/// A validated [`UtcOffset`], falling back to UTC for anything implausible.
pub(crate) fn local_offset(minutes: i32) -> UtcOffset {
    let minutes = minutes.clamp(-MAX_OFFSET_MINUTES, MAX_OFFSET_MINUTES);
    UtcOffset::from_whole_seconds(minutes * 60).unwrap_or(UtcOffset::UTC)
}

/// Which window a timestamp belongs to. A session can be in more than one.
#[derive(Debug, Clone, Copy, Default)]
struct Membership {
    today: bool,
    week: bool,
    month: bool,
    last_30_days: bool,
    /// In the thirty days before `last_30_days`. Never overlaps a window.
    previous_30_days: bool,
}

impl Membership {
    fn of(epoch: i64, bounds: &WindowBounds) -> Membership {
        Membership {
            today: epoch >= bounds.today_start,
            week: epoch >= bounds.week_start,
            month: epoch >= bounds.month_start,
            last_30_days: epoch >= bounds.last_30_days_start,
            previous_30_days: epoch >= bounds.previous_30_days_start
                && epoch < bounds.last_30_days_start,
        }
    }

    /// True when the row falls outside every window and the comparison
    /// period, which makes it irrelevant — a session whose activity is older
    /// than the widest bound, or missing entirely (stored as zero).
    fn is_empty(self) -> bool {
        !self.in_a_window() && !self.previous_30_days
    }

    /// True when the row counts toward at least one provider window.
    fn in_a_window(self) -> bool {
        self.today || self.week || self.month || self.last_30_days
    }

    /// Add the windows of `other`, for a session with usage on more than one day.
    fn include(&mut self, other: Membership) {
        self.today |= other.today;
        self.week |= other.week;
        self.month |= other.month;
        self.last_30_days |= other.last_30_days;
        self.previous_30_days |= other.previous_30_days;
    }

    /// The flags in the order of [`ProviderUsageWindows`]'s four windows.
    fn windows(self) -> [bool; 4] {
        [self.today, self.week, self.month, self.last_30_days]
    }
}

/// Local midnight of the day that holds `epoch`, for a reader
/// `offset_seconds` from UTC. [`window_bounds`] uses the same fixed offset.
fn local_day_start(epoch: i64, offset_seconds: i64) -> i64 {
    (epoch + offset_seconds).div_euclid(DAY) * DAY - offset_seconds
}

/// The index of `epoch` in a daily series that starts at `start`, when it
/// falls inside the series.
fn day_index(epoch: i64, start: i64) -> Option<usize> {
    let days = (epoch - start).div_euclid(DAY);
    usize::try_from(days)
        .ok()
        .filter(|index| *index < SERIES_DAYS)
}

/// The reader's calendar date for the day that starts at `day_start`.
pub(crate) fn local_date(day_start: i64, offset: UtcOffset) -> String {
    let date: Date = OffsetDateTime::from_unix_timestamp(day_start)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset)
        .date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

/// Turn a run of daily buckets into their wire shape.
fn days_of(buckets: &[DayBucket], start: i64, offset: UtcOffset) -> Vec<ProviderUsageDay> {
    buckets
        .iter()
        .enumerate()
        .map(|(index, bucket)| ProviderUsageDay {
            local_date: local_date(start + index as i64 * DAY, offset),
            usage: window_of(&bucket.total),
            agents: bucket
                .agents
                .iter()
                .map(|(agent, usage)| ProviderAgentDayUsage {
                    agent: agent.clone(),
                    usage: window_of(usage),
                })
                .collect(),
        })
        .collect()
}

/// Tokens and session count accumulating for one provider in one window.
///
/// Models are keyed in a `BTreeMap` rather than a `HashMap` so the pricing sum
/// below runs in a fixed order: floating-point addition is not associative, and
/// a total that changed in its last digit between runs would be a needless
/// source of flicker.
#[derive(Debug, Default)]
struct Bucket {
    /// Actual model identities supply displayed token totals.
    models: BTreeMap<String, ModelTokens>,
    /// Catalog pricing keys preserve the observed speed tier.
    pricing: BTreeMap<String, ModelTokens>,
    pricing_incomplete: bool,
    session_count: u32,
}

#[derive(Debug, Default)]
struct DayBucket {
    total: Bucket,
    agents: BTreeMap<String, Bucket>,
}

/// Add `tokens` to `entry`. [`ModelTokens::add`] keeps a recorded cost only
/// when both records have one.
fn add_model_tokens(entry: &mut ModelTokens, tokens: &ModelTokens) {
    entry.add(tokens);
}

impl Bucket {
    /// Add one provider's share of one session day. The session count is
    /// separate, because a session with several days counts once.
    fn add_usage(
        &mut self,
        models: &BTreeMap<String, ModelTokens>,
        pricing: &BTreeMap<String, ModelTokens>,
        pricing_incomplete: bool,
    ) {
        self.pricing_incomplete |= pricing_incomplete;
        for (model, tokens) in models {
            add_model_tokens(self.models.entry(model.clone()).or_default(), tokens);
        }
        for (model, tokens) in pricing {
            add_model_tokens(self.pricing.entry(model.clone()).or_default(), tokens);
        }
    }
}

/// Everything accumulating for one provider.
#[derive(Debug, Default)]
struct Accumulator {
    windows: WindowBuckets,
    /// Every model seen in the covered span, used for the provider's state.
    /// Kept separately because the state describes the provider, not a window.
    all: Bucket,
    last_activity: Option<i64>,
    explicit_provider_detected: bool,
    agents: BTreeMap<String, WindowBuckets>,
}

#[derive(Debug, Default)]
struct WindowBuckets {
    today: Bucket,
    week: Bucket,
    month_to_date: Bucket,
    last_30_days: Bucket,
}

impl WindowBuckets {
    /// The buckets in the order of [`Membership::windows`].
    fn each_mut(&mut self) -> [&mut Bucket; 4] {
        [
            &mut self.today,
            &mut self.week,
            &mut self.month_to_date,
            &mut self.last_30_days,
        ]
    }

    fn to_wire(&self) -> ProviderUsageWindows {
        ProviderUsageWindows {
            today: window_of(&self.today),
            week: window_of(&self.week),
            month_to_date: window_of(&self.month_to_date),
            last_30_days: window_of(&self.last_30_days),
        }
    }
}

/// What pricing could say about a set of models.
#[derive(Debug, Default, Clone, Copy)]
struct Priced {
    usd: f64,
    /// At least one model has a price in the runtime snapshot.
    any_priced: bool,
    /// At least one model does not, so `usd` is a floor and not a total.
    any_unpriced: bool,
}

/// Price a bucket's models, one at a time.
///
/// The engine's `price_breakdown` refuses a partial total — one unpriceable
/// model and the whole subject returns `None`, which is right for a session's
/// headline figure. Across a month of many models that rule would blank the
/// entire surface for one unknown name, so this prices model by model and
/// reports the shortfall through [`Priced::any_unpriced`] instead. The view
/// turns that into "some models could not be priced", never into silence.
fn price(models: &BTreeMap<String, ModelTokens>) -> Priced {
    let mut priced = Priced::default();
    for (model, tokens) in models {
        let single = HashMap::from([(model.clone(), tokens.clone())]);
        match price_breakdown(&single) {
            Some(cost) => {
                priced.usd += cost.total_usd;
                priced.any_priced = true;
            }
            None => priced.any_unpriced = true,
        }
    }
    priced
}

/// Turn one bucket into its wire shape.
fn window_of(bucket: &Bucket) -> ProviderUsageWindow {
    let mut priced = price(&bucket.pricing);
    priced.any_unpriced |= bucket.pricing_incomplete;
    let mut window = ProviderUsageWindow {
        session_count: bucket.session_count,
        estimated_usd: priced.any_priced.then_some(priced.usd),
        cost_complete: !priced.any_unpriced,
        ..ProviderUsageWindow::default()
    };
    for tokens in bucket.models.values() {
        // Fresh input plus cache writes, matching the engine's `tokens_in`:
        // both are prompt tokens the reader paid to send.
        window.tokens_in = window
            .tokens_in
            .saturating_add(tokens.input_tokens)
            .saturating_add(tokens.cache_creation_tokens);
        window.tokens_out = window.tokens_out.saturating_add(tokens.output_tokens);
        window.cache_read = window.cache_read.saturating_add(tokens.cache_read_tokens);
    }
    window
}

/// The state the covered evidence supports.
fn state_of(accumulator: &Accumulator) -> ProviderUsageState {
    let has_nonzero_tokens = accumulator.all.models.values().any(has_tokens);
    if !has_nonzero_tokens {
        return if accumulator.explicit_provider_detected {
            ProviderUsageState::Detected
        } else {
            ProviderUsageState::Unknown
        };
    }
    let mut priced = price(&accumulator.all.pricing);
    priced.any_unpriced |= accumulator.all.pricing_incomplete;
    if priced.any_unpriced || !priced.any_priced {
        ProviderUsageState::Observed
    } else {
        ProviderUsageState::Estimated
    }
}

pub(crate) fn has_tokens(tokens: &ModelTokens) -> bool {
    tokens.input_tokens > 0
        || tokens.output_tokens > 0
        || tokens.cache_read_tokens > 0
        || tokens.cache_creation_tokens > 0
        || tokens.cache_creation_1h_tokens > 0
}

fn token_totals<'a>(tokens: impl Iterator<Item = &'a ModelTokens>) -> [u128; 5] {
    tokens.fold([0; 5], |mut totals, tokens| {
        totals[0] += u128::from(tokens.input_tokens);
        totals[1] += u128::from(tokens.output_tokens);
        totals[2] += u128::from(tokens.cache_read_tokens);
        totals[3] += u128::from(tokens.cache_creation_tokens);
        totals[4] += u128::from(tokens.cache_creation_1h_tokens);
        totals
    })
}

/// Whether the newest evidence still describes now.
fn staleness_of(last_activity: Option<i64>, now: i64) -> ProviderUsageStaleness {
    match last_activity {
        Some(at) if now.saturating_sub(at) < STALE_EVIDENCE_SECS => ProviderUsageStaleness::Fresh,
        Some(_) => ProviderUsageStaleness::Stale,
        None => ProviderUsageStaleness::Unknown,
    }
}

/// The billable tokens one session recorded, per normalized model key.
///
/// Unparseable JSON reads as no evidence rather than as an error: a cache row
/// written by an older build is a state the surface already renders honestly.
fn breakdown_json(json: Option<&str>) -> BTreeMap<String, ModelTokens> {
    let Some(json) = json else {
        return BTreeMap::new();
    };
    serde_json::from_str::<BTreeMap<String, ModelTokens>>(json)
        .unwrap_or_default()
        .into_iter()
        .filter(|(model, _)| !model.trim().is_empty())
        .collect()
}

fn breakdown_of(record: &UsageEvidenceRecord) -> BTreeMap<String, ModelTokens> {
    breakdown_json(record.model_breakdown_json.as_deref())
}

fn pricing_breakdown_of(record: &UsageEvidenceRecord) -> BTreeMap<String, ModelTokens> {
    breakdown_json(record.pricing_breakdown_json.as_deref())
}

/// One session's tokens on one local day.
#[derive(Debug, Default)]
struct DaySlice {
    /// Billable tokens per model key, as `model_breakdown_json` keys them.
    models: BTreeMap<String, ModelTokens>,
    /// Billable tokens per pricing key, as `pricing_breakdown_json` keys them.
    pricing: BTreeMap<String, ModelTokens>,
}

/// Split one session's tokens by local day, keyed by local midnight.
///
/// With published turn rows, each turn's tokens go on the day of its
/// timestamp. A turn without a timestamp goes on the day of the session's
/// last activity. The model and pricing keys follow `query_model_breakdown`
/// and `query_pricing_breakdown`, so the days sum to the cached breakdowns.
///
/// Without published billable turn rows, the whole cached breakdown goes on
/// the day of the last activity. A session that analysis has not reached yet
/// still counts on that day.
fn day_slices(record: &UsageEvidenceRecord, offset_seconds: i64) -> BTreeMap<i64, DaySlice> {
    let last_day = local_day_start(record.updated_at_epoch, offset_seconds);
    let mut slices: BTreeMap<i64, DaySlice> = BTreeMap::new();
    for day in &record.turn_days {
        let model = strip_window_tag(&day.model).trim();
        if model.is_empty() {
            continue;
        }
        let day_start = day
            .day_start_epoch
            .map_or(last_day, |epoch| local_day_start(epoch, offset_seconds));
        let slice = slices.entry(day_start).or_default();
        add_model_tokens(
            slice.models.entry(model.to_string()).or_default(),
            &day.tokens,
        );
        add_model_tokens(
            slice
                .pricing
                .entry(turn_pricing_key(model, day.speed.as_deref()))
                .or_default(),
            &day.tokens,
        );
    }
    if slices.is_empty() {
        slices.insert(
            last_day,
            DaySlice {
                models: breakdown_of(record),
                pricing: pricing_breakdown_of(record),
            },
        );
    }
    slices
}

fn provider_hints_of(record: &UsageEvidenceRecord) -> Vec<ProviderHint> {
    record
        .provider_hints_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderAccountObservation {
    provider: String,
    account_key: String,
}

fn account_for(record: &UsageEvidenceRecord, provider: &str) -> Option<String> {
    let observations: Vec<ProviderAccountObservation> =
        serde_json::from_str(&record.provider_accounts_json).unwrap_or_default();
    let mut accounts = observations
        .into_iter()
        .filter(|observation| observation.provider == provider)
        .map(|observation| observation.account_key)
        .filter(|account| account.len() == 64);
    let first = accounts.next()?;
    accounts.all(|account| account == first).then_some(first)
}

#[derive(Debug, Default)]
pub(crate) struct Attributed {
    pub(crate) models: BTreeMap<String, ModelTokens>,
    explicit: bool,
}

fn hints_for_model<'a>(model: &str, hints: &'a [ProviderHint]) -> Vec<&'a ProviderHint> {
    let exact: Vec<_> = hints
        .iter()
        .filter(|hint| {
            hint.model
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(model))
        })
        .collect();
    if exact.is_empty() {
        hints.iter().filter(|hint| hint.model.is_none()).collect()
    } else {
        exact
    }
}

fn explicit_providers<'a>(hints: impl IntoIterator<Item = &'a ProviderHint>) -> Vec<&'static str> {
    let mut providers = Vec::new();
    for hint in hints {
        let provider = match providers::provider_for_hint(&hint.provider) {
            HintResolution::Known(provider) => provider,
            HintResolution::UnknownExplicit => providers::UNKNOWN,
        };
        if !providers.contains(&provider) {
            providers.push(provider);
        }
    }
    providers
}

/// Attribute one session's models to providers.
///
/// A fixed-route agent puts every model under its vendor; a bring-your-own
/// agent splits them, so one session can appear under two providers with the
/// tokens divided between them rather than double-counted.
pub(crate) fn attribute(
    agent: &str,
    breakdown: BTreeMap<String, ModelTokens>,
    hints: &[ProviderHint],
) -> BTreeMap<&'static str, Attributed> {
    let route = providers::route_for_agent(agent);
    let mut by_provider: BTreeMap<&'static str, Attributed> = BTreeMap::new();
    for (model, tokens) in breakdown {
        let provider = match route {
            Route::Fixed(provider) => provider,
            Route::ByModel => {
                let explicit = explicit_providers(hints_for_model(&model, hints));
                if explicit.len() == 1 {
                    explicit[0]
                } else if explicit.len() > 1 {
                    for provider in explicit {
                        by_provider.entry(provider).or_default().explicit = true;
                    }
                    providers::UNKNOWN
                } else {
                    providers::provider_for_model(&model)
                }
            }
        };
        let attributed = by_provider.entry(provider).or_default();
        attributed.explicit |=
            matches!(route, Route::ByModel) && !hints_for_model(&model, hints).is_empty();
        attributed.models.insert(model, tokens);
    }
    if matches!(route, Route::ByModel) {
        for provider in explicit_providers(hints) {
            by_provider.entry(provider).or_default().explicit = true;
        }
    }
    by_provider
}

/// The provider an unanalyzed session belongs to, when that is knowable.
///
/// A fixed-route agent still names its vendor with no models at all — a Claude
/// Code session that has not been analyzed yet is unambiguously Anthropic's.
/// A bring-your-own session with no models names nothing, so it lands in the
/// unattributed bucket, which is the honest answer rather than a dropped row.
fn provider_without_models(agent: &str) -> &'static str {
    match providers::route_for_agent(agent) {
        Route::Fixed(provider) => provider,
        Route::ByModel => providers::UNKNOWN,
    }
}

/// Attribute one session day's models to providers.
///
/// A day with no models still names a provider when the agent's route or its
/// hints name one, so an unanalyzed session keeps its session count.
fn attribute_day(
    agent: &str,
    models: BTreeMap<String, ModelTokens>,
    hints: &[ProviderHint],
) -> BTreeMap<&'static str, Attributed> {
    if !models.is_empty() {
        return attribute(agent, models, hints);
    }
    match providers::route_for_agent(agent) {
        Route::Fixed(provider) => BTreeMap::from([(provider, Attributed::default())]),
        Route::ByModel if hints.is_empty() => {
            BTreeMap::from([(provider_without_models(agent), Attributed::default())])
        }
        Route::ByModel => explicit_providers(hints)
            .into_iter()
            .map(|provider| {
                (
                    provider,
                    Attributed {
                        explicit: true,
                        ..Attributed::default()
                    },
                )
            })
            .collect(),
    }
}

/// One provider's share of one session day.
struct ProviderDay {
    provider: &'static str,
    attributed: Attributed,
    /// The pricing keys this provider owns alone.
    pricing: BTreeMap<String, ModelTokens>,
    /// Some of this provider's tokens have no pricing entry it owns alone.
    pricing_incomplete: bool,
}

/// Split one session day's pricing keys between the providers its models
/// belong to.
///
/// A pricing key that two providers can claim belongs to neither, and both
/// report their cost as incomplete. A wrong split would be worse than a floor.
fn split_pricing(
    attributed: BTreeMap<&'static str, Attributed>,
    mut remaining_pricing: BTreeMap<String, ModelTokens>,
) -> Vec<ProviderDay> {
    let mut pricing_owners: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for (provider, attributed) in &attributed {
        for model in attributed.models.keys() {
            pricing_owners
                .entry(turn_pricing_key(model, None))
                .or_default()
                .insert(provider);
            pricing_owners
                .entry(turn_pricing_key(model, Some("fast")))
                .or_default()
                .insert(provider);
        }
    }

    let mut split = Vec::with_capacity(attributed.len());
    for (provider, attributed) in attributed {
        let mut pricing = BTreeMap::new();
        let mut pricing_incomplete = false;
        let mut keys = BTreeSet::new();
        for model in attributed.models.keys() {
            keys.insert(turn_pricing_key(model, None));
            keys.insert(turn_pricing_key(model, Some("fast")));
        }
        for key in keys {
            if pricing_owners
                .get(&key)
                .is_some_and(|owners| owners.len() > 1)
            {
                pricing_incomplete |= remaining_pricing.contains_key(&key);
                continue;
            }
            if let Some(tokens) = remaining_pricing.remove(&key) {
                pricing.insert(key, tokens);
            }
        }
        pricing_incomplete |=
            token_totals(attributed.models.values()) != token_totals(pricing.values());
        split.push(ProviderDay {
            provider,
            attributed,
            pricing,
            pricing_incomplete,
        });
    }
    split
}

/// Aggregate local session evidence into per-provider usage.
///
/// `rows` may hold sessions outside every window. The caller's query uses
/// [`lookback_start`] as a lower bound, so this function skips other rows.
///
/// Each session contributes its tokens to the days its turns ran, through
/// [`day_slices`]. A window counts a session once when any of its days
/// falls in the window.
pub fn summarize(
    rows: &[UsageEvidenceRecord],
    now: i64,
    utc_offset_minutes: i32,
) -> ProviderUsageSummary {
    let bounds = window_bounds(now, utc_offset_minutes);
    let offset = local_offset(utc_offset_minutes);
    let offset_seconds = i64::from(offset.whole_seconds());
    let mut accumulators: BTreeMap<(&'static str, Option<String>), Accumulator> = BTreeMap::new();
    let mut session_counts: BTreeMap<String, [u32; 4]> = BTreeMap::new();
    let mut days: Vec<DayBucket> = (0..SERIES_DAYS).map(|_| DayBucket::default()).collect();
    let mut previous_days: Vec<DayBucket> =
        (0..SERIES_DAYS).map(|_| DayBucket::default()).collect();

    for record in rows {
        let hints = provider_hints_of(record);
        // The windows that any day of this session reaches, in total and per
        // provider group. The loop below counts the session once per window,
        // after it adds all of the session's days.
        let mut session_windows = Membership::default();
        let mut group_windows: BTreeMap<(&'static str, Option<String>), Membership> =
            BTreeMap::new();

        for (day_start, slice) in day_slices(record, offset_seconds) {
            let membership = Membership::of(day_start, &bounds);
            if membership.is_empty() {
                continue;
            }
            session_windows.include(membership);
            // The day this slice lands on, in whichever series holds it. A
            // slice is in at most one of the two.
            let day = if membership.last_30_days {
                day_index(day_start, bounds.last_30_days_start).map(|index| (&mut days, index))
            } else if membership.previous_30_days {
                day_index(day_start, bounds.previous_30_days_start)
                    .map(|index| (&mut previous_days, index))
            } else {
                None
            };
            let mut day = day.map(|(series, index)| &mut series[index]);
            if let Some(day) = day.as_deref_mut() {
                day.total.session_count = day.total.session_count.saturating_add(1);
                let agent = day.agents.entry(record.agent.clone()).or_default();
                agent.session_count = agent.session_count.saturating_add(1);
            }

            let attributed = attribute_day(&record.agent, slice.models, &hints);
            for share in split_pricing(attributed, slice.pricing) {
                let models = &share.attributed.models;
                if let Some(day) = day.as_deref_mut() {
                    let agent = day.agents.entry(record.agent.clone()).or_default();
                    for bucket in [&mut day.total, agent] {
                        bucket.add_usage(models, &share.pricing, share.pricing_incomplete);
                    }
                }
                // The comparison period feeds the daily series and nothing else:
                // no provider row, no total, no state.
                if !membership.in_a_window() {
                    continue;
                }
                let key = (share.provider, account_for(record, share.provider));
                group_windows
                    .entry(key.clone())
                    .or_default()
                    .include(membership);
                let accumulator = accumulators.entry(key).or_default();
                accumulator.explicit_provider_detected |= share.attributed.explicit;
                accumulator.last_activity = Some(
                    accumulator
                        .last_activity
                        .map_or(record.updated_at_epoch, |at| {
                            at.max(record.updated_at_epoch)
                        }),
                );
                accumulator
                    .all
                    .add_usage(models, &share.pricing, share.pricing_incomplete);
                let agent = accumulator.agents.entry(record.agent.clone()).or_default();
                for buckets in [&mut accumulator.windows, agent] {
                    for (bucket, included) in
                        buckets.each_mut().into_iter().zip(membership.windows())
                    {
                        if included {
                            bucket.add_usage(models, &share.pricing, share.pricing_incomplete);
                        }
                    }
                }
            }
        }

        if session_windows.in_a_window() {
            let counts = session_counts.entry(record.agent.clone()).or_default();
            for (count, included) in counts.iter_mut().zip(session_windows.windows()) {
                *count = count.saturating_add(u32::from(included));
            }
        }
        for (key, membership) in group_windows {
            let accumulator = accumulators.entry(key).or_default();
            accumulator.all.session_count = accumulator.all.session_count.saturating_add(1);
            let agent = accumulator.agents.entry(record.agent.clone()).or_default();
            for buckets in [&mut accumulator.windows, agent] {
                for (bucket, included) in buckets.each_mut().into_iter().zip(membership.windows()) {
                    bucket.session_count = bucket.session_count.saturating_add(u32::from(included));
                }
            }
        }
    }

    let mut providers: Vec<ProviderUsage> = accumulators
        .into_iter()
        .map(|((provider, account_key), accumulator)| ProviderUsage {
            provider: provider.to_string(),
            account_key,
            display_name: providers::display_name(provider).to_string(),
            state: state_of(&accumulator),
            staleness: staleness_of(accumulator.last_activity, now),
            windows: accumulator.windows.to_wire(),
            agents: accumulator
                .agents
                .iter()
                .map(|(agent, buckets)| ProviderAgentUsage {
                    agent: agent.clone(),
                    windows: buckets.to_wire(),
                })
                .collect(),
            last_activity_at: accumulator.last_activity.map(|at| iso_from_epoch(Some(at))),
        })
        .collect();

    // Most recently used first, with the id as the tie-break so the order is
    // total and a re-render never reshuffles equal rows. Ranking by size
    // belongs to the views, which know which window the reader is looking at.
    providers.sort_by(|a, b| {
        b.last_activity_at
            .cmp(&a.last_activity_at)
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.account_key.cmp(&b.account_key))
    });

    let mut totals =
        providers
            .iter()
            .fold(ProviderUsageWindows::default(), |mut totals, provider| {
                add_window(&mut totals.today, &provider.windows.today);
                add_window(&mut totals.week, &provider.windows.week);
                add_window(&mut totals.month_to_date, &provider.windows.month_to_date);
                add_window(&mut totals.last_30_days, &provider.windows.last_30_days);
                totals
            });
    let mut agents: BTreeMap<String, ProviderUsageWindows> = BTreeMap::new();
    for provider in &providers {
        for entry in &provider.agents {
            let windows = agents.entry(entry.agent.clone()).or_default();
            add_window(&mut windows.today, &entry.windows.today);
            add_window(&mut windows.week, &entry.windows.week);
            add_window(&mut windows.month_to_date, &entry.windows.month_to_date);
            add_window(&mut windows.last_30_days, &entry.windows.last_30_days);
        }
    }

    // A session can use more than one provider. Count it once in combined totals.
    let mut total_counts = [0_u32; 4];
    for (agent, counts) in session_counts {
        if let Some(windows) = agents.get_mut(&agent) {
            set_session_counts(windows, counts);
        }
        for (total, count) in total_counts.iter_mut().zip(counts) {
            *total = total.saturating_add(count);
        }
    }
    set_session_counts(&mut totals, total_counts);

    tracing::debug!(
        providers = providers.len(),
        agents = agents.len(),
        today_tokens = totals.today.tokens_in + totals.today.tokens_out + totals.today.cache_read,
        last_30_day_tokens = totals.last_30_days.tokens_in
            + totals.last_30_days.tokens_out
            + totals.last_30_days.cache_read,
        "computed local usage totals"
    );

    ProviderUsageSummary {
        providers,
        totals,
        agents: agents
            .into_iter()
            .map(|(agent, windows)| ProviderAgentUsage { agent, windows })
            .collect(),
        days: days_of(&days, bounds.last_30_days_start, offset),
        previous_days: days_of(&previous_days, bounds.previous_30_days_start, offset),
        generated_at: iso_from_epoch(Some(now)),
    }
}

fn set_session_counts(windows: &mut ProviderUsageWindows, counts: [u32; 4]) {
    for (window, count) in [
        &mut windows.today,
        &mut windows.week,
        &mut windows.month_to_date,
        &mut windows.last_30_days,
    ]
    .into_iter()
    .zip(counts)
    {
        window.session_count = count;
    }
}

fn add_window(target: &mut ProviderUsageWindow, source: &ProviderUsageWindow) {
    target.tokens_in = target.tokens_in.saturating_add(source.tokens_in);
    target.tokens_out = target.tokens_out.saturating_add(source.tokens_out);
    target.cache_read = target.cache_read.saturating_add(source.cache_read);
    target.session_count = target.session_count.saturating_add(source.session_count);
    target.cost_complete &= source.cost_complete;
    target.estimated_usd = match (target.estimated_usd, source.estimated_usd) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0.0) + right.unwrap_or(0.0)),
    };
}
