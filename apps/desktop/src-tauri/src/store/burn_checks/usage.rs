use antiburn_checks::decision::{DecisionAttempt, attempt_id};
use antiburn_local::analysis::jev::JevUsage;
use anyhow::ensure;
use rusqlite::OptionalExtension;

use super::super::{
    Store,
    usage_accounting::{
        USAGE_LEDGER_KEY, UsageIdentity, load_ledger, record_confirmed_usage, write_json_setting,
    },
};

impl Store {
    /// Records installed-check usage once per candidate, decision, and attempt.
    /// Known tokens can replace unknown usage. Session validity does not gate incurred usage.
    pub(crate) fn record_burn_check_decision_usage(
        &self,
        candidate_id: &str,
        attempt: &DecisionAttempt,
        check_id: &str,
        (provider, model): (&str, &str),
        usage: Option<JevUsage>,
        now_epoch: i64,
    ) -> anyhow::Result<()> {
        ensure!(
            candidate_id.len() == 64
                && candidate_id
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                && !check_id.is_empty()
                && matches!(
                    provider,
                    "typesafe-systemone"
                        | "ollama-systemone"
                        | "cloudflare-systemone"
                        | "custom-systemone"
                )
                && !model.is_empty()
                && model.len() <= 128
                && model
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte)),
            "invalid installed-check usage identity"
        );
        attempt.validate()?;
        let fingerprint = super::runtime::parse_candidate_id(candidate_id)?;
        let attempt_id = attempt_id(&fingerprint, attempt);
        let mut connection = self.lock();
        let transaction = connection.transaction()?;
        let previous: Option<bool> = transaction
            .query_row(
                "SELECT usage_known FROM burn_check_decision_usage WHERE attempt_id = ?1",
                [&attempt_id],
                |row| row.get(0),
            )
            .optional()?;
        if previous == Some(true) || (previous.is_some() && usage.is_none()) {
            return Ok(());
        }

        let mut ledger = load_ledger(&transaction)?;
        if let Some(usage) = usage {
            if previous.is_some() {
                ledger.summary.unknown_outcomes = ledger.summary.unknown_outcomes.saturating_sub(1);
            }
            record_confirmed_usage(
                &mut ledger.summary,
                &UsageIdentity {
                    provider,
                    check_id,
                    model,
                },
                usage.input_tokens,
                usage.output_tokens,
                now_epoch,
            );
        } else {
            ledger.summary.unknown_outcomes = ledger.summary.unknown_outcomes.saturating_add(1);
            ledger.summary.last_unknown_at_epoch = Some(now_epoch);
        }
        transaction.execute(
            "INSERT INTO burn_check_decision_usage (attempt_id, usage_known) VALUES (?1, ?2)
             ON CONFLICT(attempt_id) DO UPDATE SET usage_known = excluded.usage_known",
            rusqlite::params![attempt_id, usage.is_some()],
        )?;
        write_json_setting(&transaction, USAGE_LEDGER_KEY, &ledger)?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SessionKey;

    const CANDIDATE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const USAGE: JevUsage = JevUsage {
        input_tokens: 1_000_000,
        output_tokens: 12,
    };
    const IDENTITY: (&str, &str) = (
        "typesafe-systemone",
        antiburn_local::analysis::jev::PINNED_MODEL,
    );

    fn store() -> Store {
        Store::open_in_memory(std::path::Path::new("installed-check-usage")).unwrap()
    }

    fn attempt(number: u8) -> DecisionAttempt {
        DecisionAttempt::new("review", number).unwrap()
    }

    #[test]
    fn authored_check_usage_settles_once_without_native_admission() {
        let store = store();
        let check_id = "custom-check-".repeat(12);
        for _ in 0..2 {
            store
                .record_burn_check_decision_usage(
                    CANDIDATE,
                    &attempt(1),
                    &check_id,
                    IDENTITY,
                    Some(USAGE),
                    100,
                )
                .unwrap();
        }
        let summary = store.burn_check_usage_summary().unwrap();
        assert_eq!(summary.input_tokens, USAGE.input_tokens);
        assert_eq!(summary.output_tokens, USAGE.output_tokens);
        assert_eq!(summary.confirmed_calls, 1);
        assert_eq!(summary.unknown_outcomes, 0);
        assert_eq!(summary.estimated_usd.as_deref(), Some("$0.042"));
        let native_tables: i64 = store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
                    ('burn_check_assessment', 'burn_check_usage_reservation', 'burn_check_request_outcome')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(native_tables, 0);
    }

    #[test]
    fn known_usage_replaces_unknown_once_and_attempts_remain_separate() {
        let store = store();
        for _ in 0..2 {
            store
                .record_burn_check_decision_usage(
                    CANDIDATE,
                    &attempt(1),
                    "custom-check",
                    IDENTITY,
                    None,
                    100,
                )
                .unwrap();
        }
        let summary = store.burn_check_usage_summary().unwrap();
        assert_eq!(summary.unknown_outcomes, 1);
        assert_eq!(summary.confirmed_calls, 0);
        for usage in [Some(USAGE), Some(USAGE), None] {
            store
                .record_burn_check_decision_usage(
                    CANDIDATE,
                    &attempt(1),
                    "custom-check",
                    IDENTITY,
                    usage,
                    101,
                )
                .unwrap();
        }
        store
            .record_burn_check_decision_usage(
                CANDIDATE,
                &attempt(2),
                "custom-check",
                IDENTITY,
                Some(USAGE),
                102,
            )
            .unwrap();
        let summary = store.burn_check_usage_summary().unwrap();
        assert_eq!(summary.unknown_outcomes, 0);
        assert_eq!(summary.confirmed_calls, 2);
        assert_eq!(summary.input_tokens, USAGE.input_tokens * 2);
        assert_eq!(summary.output_tokens, USAGE.output_tokens * 2);
    }

    #[test]
    fn original_provider_and_model_control_pricing_and_survive_late_usage() {
        for provider in [
            "custom-systemone",
            "cloudflare-systemone",
            "ollama-systemone",
        ] {
            let store = store();
            let identity = (provider, antiburn_local::analysis::jev::PINNED_MODEL);
            store
                .record_burn_check_decision_usage(
                    CANDIDATE,
                    &attempt(1),
                    "custom-check",
                    identity,
                    None,
                    100,
                )
                .unwrap();
            store
                .cancel_unfinished_burn_check_assessments_for_connection_change()
                .unwrap();
            for usage in [Some(USAGE), None, Some(USAGE)] {
                store
                    .record_burn_check_decision_usage(
                        CANDIDATE,
                        &attempt(1),
                        "custom-check",
                        identity,
                        usage,
                        101,
                    )
                    .unwrap();
            }
            let summary = store.burn_check_usage_summary().unwrap();
            assert_eq!(summary.confirmed_calls, 1);
            assert_eq!(summary.unknown_outcomes, 0);
            assert_eq!(summary.input_tokens, USAGE.input_tokens);
            assert_eq!(
                summary.estimated_usd.as_deref(),
                if provider == "ollama-systemone" {
                    Some("$0.00")
                } else {
                    None
                }
            );
            let ledger = load_ledger(&store.lock()).unwrap();
            assert_eq!(ledger.summary.aggregates[0].provider, provider);
            assert_eq!(ledger.summary.aggregates[0].model, identity.1);
            assert_eq!(ledger.summary.aggregates[0].price_version, None);
        }
    }

    #[test]
    fn yaml_accounting_does_not_need_retired_check_tables() {
        let store = store();
        store
            .record_burn_check_decision_usage(
                CANDIDATE,
                &attempt(1),
                "custom-check",
                IDENTITY,
                None,
                100,
            )
            .unwrap();
        assert_eq!(
            store.burn_check_usage_summary().unwrap().unknown_outcomes,
            1
        );
        for usage in [Some(USAGE), Some(USAGE), None] {
            store
                .record_burn_check_decision_usage(
                    CANDIDATE,
                    &attempt(1),
                    "custom-check",
                    IDENTITY,
                    usage,
                    101,
                )
                .unwrap();
        }
        let summary = store.burn_check_usage_summary().unwrap();
        assert_eq!(summary.unknown_outcomes, 0);
        assert_eq!(summary.confirmed_calls, 1);
        assert_eq!(summary.input_tokens, USAGE.input_tokens);
        assert_eq!(summary.output_tokens, USAGE.output_tokens);
        assert_eq!(summary.estimated_usd.as_deref(), Some("$0.042"));
    }

    #[test]
    fn marker_and_usage_total_roll_back_together() {
        let store = store();
        store.lock().execute_batch("CREATE TRIGGER reject_usage BEFORE INSERT ON setting WHEN NEW.key = 'internal:burnCheckUsageLedgerV1' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
        let result = store.record_burn_check_decision_usage(
            CANDIDATE,
            &attempt(1),
            "custom-check",
            IDENTITY,
            Some(USAGE),
            100,
        );
        assert!(result.is_err());
        assert_eq!(store.burn_check_usage_summary().unwrap().confirmed_calls, 0);
        let markers: i64 = store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_decision_usage",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(markers, 0);
        store
            .lock()
            .execute_batch("DROP TRIGGER reject_usage")
            .unwrap();
        store
            .record_burn_check_decision_usage(
                CANDIDATE,
                &attempt(1),
                "custom-check",
                IDENTITY,
                Some(USAGE),
                100,
            )
            .unwrap();
        assert_eq!(store.burn_check_usage_summary().unwrap().confirmed_calls, 1);
    }

    #[test]
    fn accounting_survives_reopen_and_session_deletion_but_clears_with_local_data() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store
            .record_burn_check_decision_usage(
                CANDIDATE,
                &attempt(1),
                "custom-check",
                IDENTITY,
                Some(USAGE),
                100,
            )
            .unwrap();
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        store
            .record_burn_check_decision_usage(
                CANDIDATE,
                &attempt(1),
                "custom-check",
                IDENTITY,
                Some(USAGE),
                101,
            )
            .unwrap();
        store.lock().execute("INSERT INTO session (environment_key, agent, session_id, source_kind, source_label, first_seen_at, last_seen_at) VALUES ('native', 'fixture', 'session', 'file', 'fixture', 'now', 'now')", []).unwrap();
        store
            .delete_session(&SessionKey::new("native", "fixture", "session"))
            .unwrap();
        assert_eq!(store.burn_check_usage_summary().unwrap().confirmed_calls, 1);
        store.clear_local_session_data().unwrap();
        assert_eq!(store.burn_check_usage_summary().unwrap().confirmed_calls, 0);
        let markers: i64 = store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_decision_usage",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(markers, 0);
    }

    #[test]
    fn named_decisions_have_distinct_usage_keys_and_keep_legacy_markers() {
        let store = store();
        let legacy = format!("{CANDIDATE}:1");
        store
            .lock()
            .execute(
                "INSERT INTO burn_check_decision_usage VALUES (?1, 1)",
                [&legacy],
            )
            .unwrap();
        for name in ["review", "confirm"] {
            let attempt = DecisionAttempt::new(name, 1).unwrap();
            for usage in [None, Some(USAGE), Some(USAGE), None] {
                store
                    .record_burn_check_decision_usage(
                        CANDIDATE,
                        &attempt,
                        "custom-check",
                        IDENTITY,
                        usage,
                        100,
                    )
                    .unwrap();
            }
            let expected = attempt_id(
                &super::super::runtime::parse_candidate_id(CANDIDATE).unwrap(),
                &attempt,
            );
            assert!(
                store
                    .lock()
                    .query_row(
                        "SELECT usage_known FROM burn_check_decision_usage WHERE attempt_id = ?1",
                        [expected],
                        |row| row.get::<_, bool>(0)
                    )
                    .unwrap()
            );
        }
        let summary = store.burn_check_usage_summary().unwrap();
        assert_eq!((summary.confirmed_calls, summary.unknown_outcomes), (2, 0));
        assert_eq!(
            store
                .lock()
                .query_row(
                    "SELECT COUNT(*) FROM burn_check_decision_usage",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            3
        );
        assert!(
            store
                .lock()
                .query_row(
                    "SELECT usage_known FROM burn_check_decision_usage WHERE attempt_id = ?1",
                    [legacy],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap()
        );
    }

    #[test]
    fn invalid_literal_attempts_cannot_write_usage() {
        let store = store();
        for attempt in [
            DecisionAttempt {
                decision: "review".into(),
                number: 0,
            },
            DecisionAttempt {
                decision: "unknown:name".into(),
                number: 1,
            },
        ] {
            assert!(
                store
                    .record_burn_check_decision_usage(
                        CANDIDATE,
                        &attempt,
                        "custom-check",
                        IDENTITY,
                        Some(USAGE),
                        100
                    )
                    .is_err()
            );
        }
        assert_eq!(store.burn_check_usage_summary().unwrap().confirmed_calls, 0);
    }
}
