use super::*;

impl StoreCheckStore {
    pub(super) fn claim(
        &self,
        candidate: &FrozenCandidate,
        now_ms: u64,
        bypass: bool,
    ) -> Result<CandidateClaim> {
        ensure!(
            candidate.activation.scope == self.scope,
            "candidate belongs to another scope"
        );
        let id = hex(&candidate.fingerprint);
        let mut connection = self.store.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved: Option<(String, String, u64)> = tx.query_row(
            "SELECT c.state, c.frozen_input_json, c.claim_fence FROM burn_check_candidate c
             JOIN burn_check_activation a USING(check_id, project_key)
             WHERE c.candidate_id = ?1 AND c.project_key = ?2 AND c.generation = a.generation AND a.enabled = 1",
            params![id, self.project_key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        let Some((state, frozen, fence)) = saved else {
            return Ok(CandidateClaim::Stale);
        };
        if state != "pending" {
            return Ok(CandidateClaim::Stale);
        }
        ensure!(
            serde_json::from_str::<FrozenCandidate>(&frozen)? == *candidate,
            "claim does not match frozen candidate"
        );
        let busy: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM burn_check_candidate WHERE check_id = ?1 AND project_key = ?2
              AND generation = ?3 AND branch_key = ?4 AND state IN ('claimed', 'dispatching', 'answered'))",
            params![candidate.activation.definition_id, self.project_key, integer(candidate.activation.generation.0)?, candidate.branch.0], |row| row.get(0),
        )?;
        let until: Option<u64> = tx.query_row(
            "SELECT cooldown_until_epoch FROM burn_check_cooldown WHERE check_id = ?1 AND project_key = ?2 AND generation = ?3 AND branch_key = ?4",
            params![candidate.activation.definition_id, self.project_key, integer(candidate.activation.generation.0)?, candidate.branch.0], |row| row.get(0),
        ).optional()?;
        if busy || (!bypass && until.is_some_and(|until| now_ms < until.saturating_mul(1000))) {
            return Ok(CandidateClaim::NotDue {
                retry_at_unix_ms: until.map_or(now_ms.saturating_add(1000), |until| {
                    until.saturating_mul(1000).max(now_ms.saturating_add(1))
                }),
            });
        }
        let last: Option<(u8, String)> = tx.query_row(
            "SELECT attempt_number, outcome FROM burn_check_evaluation_attempt WHERE candidate_id = ?1 ORDER BY attempt_number DESC LIMIT 1",
            [&id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let next_attempt = match last {
            None => 1,
            Some((1, outcome)) if outcome == "safe_transient" => 2,
            _ => return Ok(CandidateClaim::Stale),
        };
        let prior: Option<String> = tx.query_row(
            "SELECT status FROM burn_check_current_status WHERE check_id = ?1 AND project_key = ?2 AND branch_key = ?3 AND generation = ?4",
            params![candidate.activation.definition_id, self.project_key, candidate.branch.0, integer(candidate.activation.generation.0)?], |row| row.get(0),
        ).optional()?;
        let fence = fence.checked_add(1).context("claim fence is exhausted")?;
        tx.execute("UPDATE burn_check_candidate SET state = 'claimed', claim_owner = ?2, claim_fence = ?3,
            prior_status = ?4, claim_epoch = ?5, updated_at_epoch = ?5 WHERE candidate_id = ?1 AND state = 'pending'",
            params![id, self.owner, integer(fence)?, prior, integer(now_ms / 1000)?])?;
        tx.commit()?;
        Ok(CandidateClaim::Claimed(Box::new(ClaimedCandidate {
            id: CandidateId::from(candidate),
            candidate: candidate.clone(),
            prior_status: parse_status(prior)?,
            fence,
            next_attempt,
        })))
    }

    pub(super) fn dispatch(
        &self,
        claimed: &ClaimedCandidate,
        now_ms: u64,
        cooldown: Duration,
        number: u8,
    ) -> Result<()> {
        let claim = self.claim_record(claimed)?;
        ensure!(
            (1..=2).contains(&number),
            "attempt number must be one or two"
        );
        let mut connection = self.store.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if number == 2 {
            let safe: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM burn_check_evaluation_attempt WHERE candidate_id = ?1 AND attempt_number = 1 AND state = 'settled' AND outcome = 'safe_transient')", [&claim.candidate_id], |row| row.get(0))?;
            ensure!(safe, "retry requires a settled safe failure");
        }
        let changed = tx.execute("UPDATE burn_check_candidate SET state = 'dispatching', updated_at_epoch = ?4
            WHERE candidate_id = ?1 AND claim_owner = ?2 AND claim_fence = ?3 AND state = 'claimed'
            AND EXISTS(SELECT 1 FROM burn_check_activation a WHERE a.check_id = burn_check_candidate.check_id
              AND a.project_key = burn_check_candidate.project_key AND a.generation = burn_check_candidate.generation AND a.enabled = 1)",
            params![claim.candidate_id, claim.owner, claim.fence, integer(now_ms / 1000)?])?;
        ensure!(changed == 1, "dispatch claim is stale");
        tx.execute("INSERT INTO burn_check_evaluation_attempt (attempt_id, candidate_id, attempt_number, state, updated_at_epoch)
            VALUES (?1, ?2, ?3, 'dispatch_intent', ?4)",
            params![format!("{}:{number}", claim.candidate_id), claim.candidate_id, number, integer(now_ms / 1000)?])?;
        if number == 1 {
            let cooldown_ms = u64::try_from(cooldown.as_millis())
                .context("cooldown exceeds supported duration")?;
            let until = now_ms.saturating_add(cooldown_ms).saturating_add(999) / 1000;
            tx.execute("INSERT INTO burn_check_cooldown SELECT check_id, project_key, generation, branch_key, ?2 FROM burn_check_candidate WHERE candidate_id = ?1
                ON CONFLICT(check_id, project_key, generation, branch_key) DO UPDATE SET cooldown_until_epoch = MAX(cooldown_until_epoch, excluded.cooldown_until_epoch)",
                params![claim.candidate_id, integer(until)?])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn settle_failure(
        &self,
        claimed: &ClaimedCandidate,
        number: u8,
        outcome: AttemptFailure,
    ) -> Result<()> {
        let claim = self.claim_record(claimed)?;
        let mut connection = self.store.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome_name = match outcome {
            AttemptFailure::SafeTransient => "safe_transient",
            AttemptFailure::Unknown => "unknown",
            AttemptFailure::Permanent => "permanent",
        };
        let changed = tx.execute("UPDATE burn_check_evaluation_attempt SET state = 'settled', outcome = ?3, updated_at_epoch = ?4
            WHERE candidate_id = ?1 AND attempt_number = ?2 AND state = 'dispatch_intent'
              AND EXISTS(SELECT 1 FROM burn_check_candidate c WHERE c.candidate_id = ?1 AND c.claim_owner = ?5 AND c.claim_fence = ?6)",
            params![claim.candidate_id, number, outcome_name, now_epoch(), claim.owner, claim.fence])?;
        ensure!(changed == 1, "attempt claim is stale or already settled");
        if outcome == AttemptFailure::SafeTransient && number == 1 {
            tx.execute("UPDATE burn_check_candidate SET state = 'claimed' WHERE candidate_id = ?1 AND state = 'dispatching'", [&claim.candidate_id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn save_answer(
        &self,
        claimed: &ClaimedCandidate,
        evaluated: &EvaluatedCandidate,
    ) -> Result<()> {
        let claim = self.claim_record(claimed)?;
        let answer = serde_json::to_string(evaluated)?;
        let mut connection = self.store.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute("UPDATE burn_check_evaluation_attempt SET state = 'settled', outcome = 'answer', response_json = ?3, updated_at_epoch = ?4
            WHERE candidate_id = ?1 AND attempt_number = ?2 AND state = 'dispatch_intent'
              AND EXISTS(SELECT 1 FROM burn_check_candidate c WHERE c.candidate_id = ?1 AND c.claim_owner = ?5 AND c.claim_fence = ?6)",
            params![claim.candidate_id, evaluated.attempts, answer, now_epoch(), claim.owner, claim.fence])?;
        ensure!(changed == 1, "answer has no exact dispatch intent");
        let changed = tx.execute("UPDATE burn_check_candidate SET state = CASE WHEN state = 'dispatching' THEN 'answered' ELSE state END, answer_json = ?4, updated_at_epoch = ?5
            WHERE candidate_id = ?1 AND claim_owner = ?2 AND claim_fence = ?3 AND state IN ('dispatching', 'stale', 'cancelled')",
            params![claim.candidate_id, claim.owner, claim.fence, answer, now_epoch()])?;
        ensure!(changed == 1, "answer claim is stale");
        tx.commit()?;
        Ok(())
    }

    pub(super) fn terminal(
        &self,
        claimed: &ClaimedCandidate,
        detail: &str,
        not_assessed: bool,
    ) -> Result<()> {
        let claim = self.claim_record(claimed)?;
        let state = if not_assessed {
            "not_assessed"
        } else {
            "execution_failed"
        };
        let mut connection = self.store.lock();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute("INSERT INTO burn_check_assessment_history
            SELECT ?4, candidate_id, check_id, project_key, generation, branch_key, trigger_position, ?5, answer_json, ?6, 0, ?7
              FROM burn_check_candidate WHERE candidate_id = ?1 AND claim_owner = ?2 AND claim_fence = ?3 AND state IN ('claimed', 'dispatching', 'answered')",
            params![claim.candidate_id, claim.owner, claim.fence, format!("assessment:{}", claim.candidate_id), state, serde_json::to_string(&serde_json::json!({"detail": detail}))?, now_epoch()])?;
        ensure!(changed == 1, "terminal claim is stale");
        tx.execute("UPDATE burn_check_candidate SET state = ?4, updated_at_epoch = ?5 WHERE candidate_id = ?1 AND claim_owner = ?2 AND claim_fence = ?3", params![claim.candidate_id, claim.owner, claim.fence, state, now_epoch()])?;
        tx.commit()?;
        Ok(())
    }
}
