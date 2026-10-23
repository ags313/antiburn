use antiburn_checks::bounded_json::measure_json;
use antiburn_checks::results::RESULT_LIMITS;
use rusqlite::Connection;

use super::*;

impl StoreCheckStore {
    pub(super) fn read_decision_progress(
        &self,
        connection: &Connection,
        claim: &BurnCheckCandidateClaim,
        activation: &EffectiveActivation,
    ) -> Result<DecisionProgress> {
        let current: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM burn_check_candidate
              WHERE candidate_id = ?1 AND claim_owner = ?2 AND claim_fence = ?3)",
            params![claim.candidate_id, claim.owner, claim.fence],
            |row| row.get(0),
        )?;
        ensure!(current, "decision progress claim is stale");
        self.read_decision_rows(connection, &claim.candidate_id, activation)
    }

    pub(super) fn read_decision_rows(
        &self,
        connection: &Connection,
        candidate: &str,
        activation: &EffectiveActivation,
    ) -> Result<DecisionProgress> {
        let declarations = &activation.compiled.decisions;
        let requirements = activation.compiled.question_requirements();
        let mut statement = connection.prepare(
            "SELECT attempt_id, decision_name, attempt_number, state, outcome, response_json
             FROM burn_check_evaluation_attempt WHERE candidate_id = ?1 ORDER BY rowid",
        )?;
        let mut rows = statement.query([candidate])?;
        let mut attempts = Vec::new();
        let mut retained_bytes = 0_u64;
        let candidate_id = parse_candidate_id(candidate)?;
        while let Some(row) = rows.next()? {
            ensure!(
                attempts.len() < declarations.len().saturating_mul(2),
                "decision progress has too many rows"
            );
            let name = row.get_ref(1)?.as_str()?;
            let response = row.get_ref(5)?;
            let response = if matches!(response, rusqlite::types::ValueRef::Null) {
                None
            } else {
                Some(response.as_str()?)
            };
            retained_bytes = retained_bytes
                .checked_add(u64::try_from(name.len())?)
                .and_then(|bytes| bytes.checked_add(response.map_or(0, |value| value.len()) as u64))
                .and_then(|bytes| bytes.checked_add(64))
                .context("decision progress size overflow")?;
            ensure!(
                retained_bytes <= RESULT_LIMITS.max_bytes,
                "decision progress exceeds its retained byte limit"
            );
            let attempt = DecisionAttempt::new(name, row.get(2)?)?;
            let declaration = declarations
                .iter()
                .find(|block| block.name == name)
                .context("saved attempt names an unknown decision")?;
            ensure!(
                row.get_ref(0)?.as_str()? == attempt_id(&candidate_id, &attempt),
                "saved attempt identity is invalid"
            );
            let state = row.get_ref(3)?.as_str()?;
            let saved_outcome = row.get_ref(4)?;
            let saved_outcome = if matches!(saved_outcome, rusqlite::types::ValueRef::Null) {
                None
            } else {
                Some(saved_outcome.as_str()?)
            };
            let outcome = match (state, saved_outcome, response) {
                ("dispatch_intent", None, None) => DecisionAttemptOutcome::DispatchIntent,
                ("settled", Some("safe_transient"), None) => DecisionAttemptOutcome::SafeTransient,
                ("settled", Some("unknown"), None) => DecisionAttemptOutcome::Unknown,
                ("settled", Some("permanent"), None) => DecisionAttemptOutcome::Permanent,
                ("settled", Some("answer"), Some(response)) => DecisionAttemptOutcome::Answer(
                    decode_decision_result(response.as_bytes())?
                        .validate_for_declaration(declaration, &requirements)?,
                ),
                _ => anyhow::bail!("saved attempt has a contradictory state or response"),
            };
            attempts.push(SavedDecisionAttempt { attempt, outcome });
        }
        let answers = attempts
            .iter()
            .filter_map(|saved| match &saved.outcome {
                DecisionAttemptOutcome::Answer(answer) => Some(answer),
                _ => None,
            })
            .collect::<Vec<_>>();
        #[derive(Serialize)]
        struct ResultSet<'a> {
            version: u8,
            decisions: Vec<&'a DecisionResult>,
            attempts: u64,
        }
        measure_json(
            &ResultSet {
                version: 1,
                decisions: answers,
                attempts: attempts.len() as u64,
            },
            RESULT_LIMITS,
        )?;
        let progress = DecisionProgress { attempts };
        validate_decision_progress(declarations, &requirements, &progress)?;
        Ok(progress)
    }

    pub(super) fn claim_is_current_in(
        &self,
        connection: &Connection,
        claim: &BurnCheckCandidateClaim,
        activation: &EffectiveActivation,
    ) -> Result<bool> {
        let current: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM burn_check_candidate c
             JOIN burn_check_activation a USING(check_id, activation_scope_key)
             WHERE c.candidate_id = ?1 AND c.claim_owner = ?2 AND c.claim_fence = ?3
               AND c.check_id = ?5 AND c.activation_scope_key = ?6 AND c.generation = ?7
               AND c.branch_key = json_extract(c.frozen_input_json, '$.branch')
               AND c.state IN ('claimed', 'dispatching', 'answered')
               AND c.generation = a.generation AND a.enabled = 1 AND a.definition_revision = ?4
               AND NOT EXISTS(SELECT 1 FROM burn_check_acceptance_watermark w
                 WHERE w.check_id = c.check_id AND w.activation_scope_key = c.activation_scope_key
                   AND w.branch_key = c.branch_key AND w.generation = c.generation
                   AND w.position >= c.trigger_position))",
            params![
                claim.candidate_id,
                claim.owner,
                claim.fence,
                hex(&activation.revision),
                activation.id.definition_id,
                self.activation_scope_key,
                integer(activation.id.generation.0)?
            ],
            |row| row.get(0),
        )?;
        if !current {
            return Ok(false);
        }
        let mut statement = connection.prepare(
            "SELECT d.source_format, d.native_revision, d.evidence_version, d.removed,
                d.payload_sha256, o.source_format, o.native_revision, o.evidence_version,
                o.removed, o.payload_json
             FROM burn_check_candidate_dependency d
             LEFT JOIN burn_check_observation o ON o.check_id = ?2 AND o.activation_scope_key = ?3
               AND o.observation_id = d.observation_id AND o.session_key_json = d.session_key_json
             WHERE d.candidate_id = ?1",
        )?;
        let mut rows = statement.query(params![
            claim.candidate_id,
            activation.id.definition_id,
            self.activation_scope_key
        ])?;
        while let Some(row) = rows.next()? {
            let expected_hash: String = row.get(4)?;
            if row.get::<_, Option<String>>(5)?.as_deref() != Some(row.get_ref(0)?.as_str()?)
                || row.get::<_, Option<i64>>(6)? != Some(row.get(1)?)
                || row.get::<_, Option<String>>(7)?.as_deref() != Some(row.get_ref(2)?.as_str()?)
                || row.get::<_, i64>(3)? != 0
                || row.get::<_, Option<i64>>(8)? != Some(0)
                || row
                    .get::<_, Option<String>>(9)?
                    .as_deref()
                    .is_none_or(|payload| super::super::payload_sha256(payload) != expected_hash)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(in crate::store::burn_checks) fn parse_candidate_id(value: &str) -> Result<[u8; 32]> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid candidate identity"
    );
    let mut result = [0_u8; 32];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
    }
    Ok(result)
}
