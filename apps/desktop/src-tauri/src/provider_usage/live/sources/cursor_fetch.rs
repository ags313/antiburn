//! Fetch bounded Cursor usage pages and commit complete runs only.

use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::time::Duration;

use super::{
    cooldown::Cooldown,
    cursor_login::{self, CursorLogin, CursorLoginState},
    http,
};
use crate::provider_usage::live::{
    SourceOutcome, cursor::parse_usage_page, model::ProviderUsageError,
};
use crate::store::{Store, StoredCursorUsageEvent};

const ENDPOINT: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetFilteredUsageEvents";
const DAY_MS: i64 = 86_400_000;
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 100;

#[derive(Default)]
pub struct CursorFetch {
    cooldown: Cooldown,
    // Keep only a digest in memory. A changed credential permits another request.
    expired: Mutex<Option<[u8; 32]>>,
}

impl CursorFetch {
    pub fn login_state(&self) -> CursorLoginState {
        self.state_for(&cursor_login::read())
    }

    fn state_for(&self, login: &CursorLogin) -> CursorLoginState {
        if let CursorLogin::SignedIn { access_token } = login {
            let identity: [u8; 32] = Sha256::digest(access_token.as_bytes()).into();
            if *self.expired.lock().unwrap_or_else(|p| p.into_inner()) == Some(identity) {
                return CursorLoginState::Expired;
            }
        }
        login.state()
    }

    pub fn poll(
        &self,
        store: &Store,
        now: time::OffsetDateTime,
        max_age: Duration,
        observed: impl FnOnce(crate::analytics::event::Facts),
    ) -> SourceOutcome {
        self.poll_observed(store, now, max_age, cursor_login::read, ENDPOINT, observed)
    }

    #[cfg(test)]
    fn poll_authorized(
        &self,
        store: &Store,
        now: time::OffsetDateTime,
        max_age: Duration,
        login: impl FnOnce() -> CursorLogin,
        endpoint: &str,
    ) -> SourceOutcome {
        self.poll_observed(store, now, max_age, login, endpoint, |_| {})
    }

    fn poll_observed(
        &self,
        store: &Store,
        now: time::OffsetDateTime,
        max_age: Duration,
        login: impl FnOnce() -> CursorLogin,
        endpoint: &str,
        observed: impl FnOnce(crate::analytics::event::Facts),
    ) -> SourceOutcome {
        if !store.settings_snapshot().cursor_account_usage_enabled {
            return SourceOutcome::absent();
        }
        self.cooldown.poll(now, max_age, || {
            let login = login();
            let state = self.state_for(&login);
            let result = self.fetch_login(store, now.unix_timestamp(), login, endpoint);
            if store.settings_snapshot().cursor_account_usage_enabled {
                observed(cursor_fetch_facts(state, &result));
            }
            result?;
            Ok(None)
        })
    }

    #[cfg(test)]
    fn poll_with_login(
        &self,
        store: &Store,
        now: time::OffsetDateTime,
        max_age: Duration,
        login: impl FnOnce() -> CursorLogin,
        endpoint: &str,
    ) -> SourceOutcome {
        self.poll_observed(store, now, max_age, login, endpoint, |_| {})
    }

    fn fetch_login(
        &self,
        store: &Store,
        now: i64,
        login: CursorLogin,
        endpoint: &str,
    ) -> Result<usize, ProviderUsageError> {
        let token = match login {
            CursorLogin::SignedIn { access_token } => access_token,
            CursorLogin::SignedOut | CursorLogin::Expired => {
                return Err(ProviderUsageError::Authentication);
            }
            CursorLogin::Unreadable => return Err(ProviderUsageError::Unavailable),
        };
        let identity: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if *self.expired.lock().unwrap_or_else(|p| p.into_inner()) == Some(identity) {
            return Err(ProviderUsageError::Authentication);
        }
        let result = fetch_pages(store, now, &token, endpoint);
        if result == Err(ProviderUsageError::Authentication) {
            *self.expired.lock().unwrap_or_else(|p| p.into_inner()) = Some(identity);
        }
        result
    }
}

fn cursor_fetch_facts(
    state: CursorLoginState,
    result: &Result<usize, ProviderUsageError>,
) -> crate::analytics::event::Facts {
    use crate::analytics::event::{Facts, bucket};
    let label = match (state, result) {
        (_, Ok(_)) => "ok",
        (CursorLoginState::SignedOut, Err(_)) => "signed_out",
        (CursorLoginState::Expired, Err(_)) | (_, Err(ProviderUsageError::Authentication)) => {
            "expired"
        }
        _ => "failed",
    };
    Facts {
        label: Some(label),
        bucket: result.as_ref().ok().map(|count| bucket(*count as u64)),
        ..Facts::default()
    }
}

fn fetch_pages(
    store: &Store,
    now: i64,
    token: &str,
    endpoint: &str,
) -> Result<usize, ProviderUsageError> {
    // Production credentials go only to the fixed HTTPS endpoint.
    if endpoint != ENDPOINT && !cfg!(test) {
        return Err(ProviderUsageError::Unavailable);
    }
    let unavailable = |_| ProviderUsageError::Unavailable;
    let end = now.saturating_mul(1000);
    let cutoff = store
        .provider_usage_retention_cutoff_epoch(now)
        .map_err(unavailable)?
        .unwrap_or(0)
        .saturating_mul(1000)
        .max(0);
    let start = store
        .newest_cursor_usage_timestamp_ms()
        .map_err(unavailable)?
        .map(|ts| ts.saturating_sub(DAY_MS))
        .unwrap_or(cutoff)
        .max(cutoff)
        .min(end);
    let mut events = Vec::new();
    for page in 1..=MAX_PAGES {
        if !store.settings_snapshot().cursor_account_usage_enabled {
            return Ok(0);
        }
        let response = http::client().post(endpoint)
            .bearer_auth(token)
            .header("Connect-Protocol-Version", "1")
            .json(&serde_json::json!({"startDate": start.to_string(), "endDate": end.to_string(), "page": page, "pageSize": PAGE_SIZE}))
            .send().map_err(|_| ProviderUsageError::Unavailable)?;
        if let Some(error) = http::status_error(response.status()) {
            return Err(error);
        }
        let parsed = parse_usage_page(&http::read_capped_body(response)?)?;
        let count = parsed.events.len();
        if count > PAGE_SIZE {
            return Err(ProviderUsageError::Unavailable);
        }
        for event in parsed.events {
            let ts_ms =
                i64::try_from(event.timestamp_ms).map_err(|_| ProviderUsageError::Unavailable)?;
            if ts_ms < start || ts_ms > end {
                return Err(ProviderUsageError::Unavailable);
            }
            events.push(StoredCursorUsageEvent {
                usage_event_id: event.usage_event_id,
                ts_ms,
                model: event.model,
                kind: event.kind,
                input_tokens: event.input_tokens,
                output_tokens: event.output_tokens,
                cache_read_tokens: event.cache_read_tokens,
                recorded_cost_micros: event.recorded_cost_micros,
                conversation_id: (!event.conversation_id.is_empty())
                    .then_some(event.conversation_id),
                headless: event.is_headless,
                fetched_at: now,
            });
        }
        if (parsed.total_usage_events_count > 0
            && events.len() as u64 >= parsed.total_usage_events_count)
            || (parsed.total_usage_events_count == 0 && count < PAGE_SIZE)
        {
            return store
                .insert_cursor_usage_events(&events)
                .map_err(unavailable);
        }
        if count == 0 {
            return Err(ProviderUsageError::Unavailable);
        }
    }
    Err(ProviderUsageError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const NOW: i64 = 1_800_000_000;

    fn enabled_store(path: &std::path::Path) -> Store {
        let store = Store::open_in_memory(path).unwrap();
        store
            .update_settings(|settings| settings.cursor_account_usage_enabled = true)
            .unwrap();
        store
    }

    fn server(
        replies: Vec<(u16, String)>,
    ) -> (String, std::thread::JoinHandle<Vec<serde_json::Value>>) {
        server_with_hook(replies, || {})
    }

    fn server_with_hook(
        replies: Vec<(u16, String)>,
        mut before_response: impl FnMut() + Send + 'static,
    ) -> (String, std::thread::JoinHandle<Vec<serde_json::Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/usage", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break bytes.len();
                    }
                };
                let headers = String::from_utf8_lossy(&bytes).to_lowercase();
                assert!(headers.contains("connect-protocol-version: 1\r\n"));
                assert!(headers.contains("content-type: application/json\r\n"));
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                bytes.resize(header_end + length, 0);
                stream.read_exact(&mut bytes[header_end..]).unwrap();
                requests.push(serde_json::from_slice(&bytes[header_end..]).unwrap());
                before_response();
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (url, thread)
    }

    fn page(first: usize, count: usize, total: usize) -> String {
        serde_json::json!({"totalUsageEventsCount": total, "usageEventsDisplay": (first..first+count).map(|i| serde_json::json!({"usageEventId":format!("synthetic-{i}"),"timestamp":(NOW*1000 - 1000 + i as i64).to_string()})).collect::<Vec<_>>()}).to_string()
    }

    fn login() -> CursorLogin {
        CursorLogin::SignedIn {
            access_token: "synthetic-test-credential".into(),
        }
    }

    #[test]
    fn analytics_reports_completed_attempts_only_with_closed_coarse_facts() {
        let dir = tempfile::tempdir().unwrap();
        let store = enabled_store(dir.path());
        let fetch = CursorFetch::default();
        let now = time::OffsetDateTime::from_unix_timestamp(NOW).unwrap();
        let age = Duration::from_secs(1800);
        let (url, server) = server(vec![
            (200, page(0, 12, 12)),
            (401, "{}".into()),
            (500, "{}".into()),
        ]);
        for (login, expected, count) in [
            (login(), "ok", Some("10-49")),
            (login(), "expired", None),
            (CursorLogin::SignedOut, "signed_out", None),
            (CursorLogin::Expired, "expired", None),
            (
                CursorLogin::SignedIn {
                    access_token: "synthetic-other".into(),
                },
                "failed",
                None,
            ),
        ] {
            fetch.cooldown.open_for_test();
            let mut recorded = false;
            fetch.poll_observed(
                &store,
                now,
                age,
                || login,
                &url,
                |facts| {
                    recorded = true;
                    assert_eq!(facts.label, Some(expected));
                    assert_eq!(facts.bucket, count);
                    assert_eq!(facts.detail, None);
                    #[cfg(feature = "analytics")]
                    assert!(
                        !crate::analytics::event::EventName::CursorAccountUsageFetched
                            .is_user_oriented(&facts)
                    );
                },
            );
            assert!(recorded);
            fetch.poll_observed(
                &store,
                now,
                age,
                || panic!("cached"),
                &url,
                |_| panic!("cached analytics"),
            );
        }
        store
            .update_settings(|settings| settings.cursor_account_usage_enabled = false)
            .unwrap();
        fetch.cooldown.open_for_test();
        fetch.poll_observed(
            &store,
            now,
            age,
            || panic!("disabled"),
            &url,
            |_| panic!("disabled analytics"),
        );
        assert_eq!(server.join().unwrap().len(), 3);
    }

    #[test]
    fn opting_out_during_a_page_stops_pagination_and_discards_partial_events() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(enabled_store(dir.path()));
        let server_store = store.clone();
        let (url, server) = server_with_hook(vec![(200, page(0, 100, 101))], move || {
            server_store
                .update_settings(|settings| settings.cursor_account_usage_enabled = false)
                .unwrap();
        });
        let outcome = CursorFetch::default().poll_observed(
            &store,
            time::OffsetDateTime::from_unix_timestamp(NOW).unwrap(),
            Duration::from_secs(1800),
            login,
            &url,
            |_| panic!("canceled import must not report success"),
        );
        assert!(outcome.error.is_none());
        assert_eq!(server.join().unwrap().len(), 1);
        assert!(store.cursor_usage_events(0, i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn consent_gates_login_and_requests_and_enable_fetches_without_a_view() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory(dir.path()).unwrap();
        let fetch = CursorFetch::default();
        let now = time::OffsetDateTime::from_unix_timestamp(NOW).unwrap();
        let age = Duration::from_secs(1800);
        assert!(
            fetch
                .poll_authorized(
                    &store,
                    now,
                    age,
                    || panic!("off must not read login"),
                    "invalid"
                )
                .error
                .is_none()
        );
        store
            .update_settings(|settings| settings.cursor_account_usage_enabled = true)
            .unwrap();
        let (url, server) = server(vec![(200, page(0, 1, 1))]);
        assert!(
            fetch
                .poll_authorized(&store, now, age, login, &url)
                .error
                .is_none()
        );
        assert_eq!(server.join().unwrap().len(), 1);
        assert_eq!(store.cursor_usage_events(0, i64::MAX).unwrap().len(), 1);
        store
            .update_settings(|settings| settings.cursor_account_usage_enabled = false)
            .unwrap();
        fetch.cooldown.open_for_test();
        assert!(
            fetch
                .poll_authorized(
                    &store,
                    now,
                    age,
                    || panic!("off must not read login"),
                    "invalid"
                )
                .error
                .is_none()
        );
    }

    #[test]
    fn cooldown_skips_credential_reads_and_changed_login_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let store = enabled_store(dir.path());
        let fetch = CursorFetch::default();
        assert_eq!(fetch.state_for(&login()), CursorLoginState::SignedIn);
        let now = time::OffsetDateTime::from_unix_timestamp(NOW).unwrap();
        let max_age = Duration::from_secs(300);
        let (url, server) = server(vec![(401, "{}".into()), (200, "{}".into())]);
        assert_eq!(
            fetch
                .poll_with_login(&store, now, max_age, login, &url)
                .error,
            Some(ProviderUsageError::Authentication)
        );
        assert_eq!(fetch.state_for(&login()), CursorLoginState::Expired);
        assert_eq!(
            fetch.state_for(&CursorLogin::SignedOut),
            CursorLoginState::SignedOut
        );
        assert_eq!(
            fetch.state_for(&CursorLogin::Expired),
            CursorLoginState::Expired
        );
        assert_eq!(
            fetch.state_for(&CursorLogin::Unreadable),
            CursorLoginState::Unreadable
        );
        assert_eq!(
            fetch.state_for(&CursorLogin::SignedIn {
                access_token: "synthetic-new-login".into()
            }),
            CursorLoginState::SignedIn
        );
        assert_eq!(
            fetch
                .poll_with_login(
                    &store,
                    now,
                    max_age,
                    || panic!("cooldown must skip login reads"),
                    &url
                )
                .error,
            Some(ProviderUsageError::Authentication)
        );
        fetch.cooldown.open_for_test();
        assert!(
            fetch
                .poll_with_login(
                    &store,
                    now,
                    max_age,
                    || CursorLogin::SignedIn {
                        access_token: "synthetic-replacement-credential".into()
                    },
                    &url
                )
                .error
                .is_none()
        );
        assert_eq!(server.join().unwrap().len(), 2);
        assert!(store.cursor_usage_events(0, i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn paging_overlap_and_repeated_runs_insert_only_new_events() {
        let dir = tempfile::tempdir().unwrap();
        let store = enabled_store(dir.path());
        store
            .update_settings(|settings| settings.session_data_retention_days = 30)
            .unwrap();
        let fetch = CursorFetch::default();
        let (url, server) = server(vec![
            (200, page(0, 100, 101)),
            (200, page(100, 1, 101)),
            (200, page(100, 2, 2)),
        ]);
        assert_eq!(fetch.fetch_login(&store, NOW, login(), &url), Ok(101));
        assert_eq!(fetch.fetch_login(&store, NOW, login(), &url), Ok(1));
        let requests = server.join().unwrap();
        assert_eq!(
            requests[0]["startDate"],
            ((NOW - 30 * 86400) * 1000).to_string()
        );
        assert_eq!(requests[0]["endDate"], (NOW * 1000).to_string());
        assert_eq!(requests[0]["pageSize"], 100);
        assert_eq!(requests[1]["page"], 2);
        assert_eq!(
            requests[2]["startDate"],
            (NOW * 1000 - 900 - DAY_MS).to_string()
        );
        assert_eq!(store.cursor_usage_events(0, i64::MAX).unwrap().len(), 102);
    }

    #[test]
    fn later_page_failures_preserve_all_stored_events() {
        for (status, body) in [(200, "{malformed"), (500, "failure"), (429, "limited")] {
            let dir = tempfile::tempdir().unwrap();
            let store = enabled_store(dir.path());
            let (url, server) = server(vec![
                (200, page(0, 1, 1)),
                (200, page(1, 100, 101)),
                (status, body.into()),
            ]);
            let fetch = CursorFetch::default();
            assert_eq!(fetch.fetch_login(&store, NOW, login(), &url), Ok(1));
            let before = store.cursor_usage_events(0, i64::MAX).unwrap();
            assert!(fetch.fetch_login(&store, NOW, login(), &url).is_err());
            assert_eq!(store.cursor_usage_events(0, i64::MAX).unwrap(), before);
            assert_eq!(server.join().unwrap().len(), 3);
        }
    }

    #[test]
    fn rejected_login_stops_without_retrying_and_preserves_events() {
        for status in [401, 403] {
            let dir = tempfile::tempdir().unwrap();
            let store = enabled_store(dir.path());
            let fetch = CursorFetch::default();
            let (url, server) = server(vec![(200, page(0, 1, 1)), (status, "{}".into())]);
            assert_eq!(fetch.fetch_login(&store, NOW, login(), &url), Ok(1));
            let before = store.cursor_usage_events(0, i64::MAX).unwrap();
            for _ in 0..3 {
                assert_eq!(
                    fetch.fetch_login(&store, NOW, login(), &url),
                    Err(ProviderUsageError::Authentication)
                );
            }
            assert_eq!(store.cursor_usage_events(0, i64::MAX).unwrap(), before);
            assert_eq!(server.join().unwrap().len(), 2);
        }
    }

    #[test]
    fn page_cap_discards_incomplete_run() {
        let dir = tempfile::tempdir().unwrap();
        let store = enabled_store(dir.path());
        let (url, server) = server(
            (0..MAX_PAGES)
                .map(|_| (200, page(0, 1, MAX_PAGES + 1)))
                .collect(),
        );
        assert_eq!(
            CursorFetch::default().fetch_login(&store, NOW, login(), &url),
            Err(ProviderUsageError::Unavailable)
        );
        assert!(store.cursor_usage_events(0, i64::MAX).unwrap().is_empty());
        assert_eq!(server.join().unwrap().len(), MAX_PAGES);
    }
}
