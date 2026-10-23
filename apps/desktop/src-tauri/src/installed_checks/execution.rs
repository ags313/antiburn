use super::*;
use crate::jev::worker::{ExecutionSnapshot, WorkerHandle};
use antiburn_checks::domain::ActivationId;
use std::future::Future;
use std::pin::Pin;

pub(super) type CapabilityFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ModelCapabilities>> + Send + 'a>>;

pub(super) struct CapabilityState {
    generation: u64,
    result: std::result::Result<ModelCapabilities, String>,
    retry_at: Instant,
}

pub(super) trait SelectedJobRuntime: Send + Sync {
    fn store(&self) -> &Store;
    fn worker(&self) -> &WorkerHandle;
    fn installed(&self) -> &InstalledChecks;
    fn now_unix_ms(&self) -> Result<u64> {
        Ok(u64::try_from(
            time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000,
        )?)
    }
    fn report_changed(&self) {}
    fn settings_changed(&self) {}
    fn preparation_recorded(&self, _: &antiburn_checks::fit::PreparationRecord) {}
    fn resolve<'a>(&'a self, snapshot: &'a ExecutionSnapshot) -> CapabilityFuture<'a> {
        Box::pin(resolve_capabilities(snapshot))
    }
    #[cfg(test)]
    fn stage(&self, _: Stage) {}
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    Preparation,
    ResponseReceipt,
    Authentication,
}

pub(super) struct AppRuntime(pub(super) tauri::AppHandle);

impl SelectedJobRuntime for AppRuntime {
    fn store(&self) -> &Store {
        self.0.state::<Store>().inner()
    }
    fn worker(&self) -> &WorkerHandle {
        self.0.state::<WorkerHandle>().inner()
    }
    fn installed(&self) -> &InstalledChecks {
        self.0.state::<InstalledChecks>().inner()
    }
    fn report_changed(&self) {
        let _ = self
            .0
            .emit(crate::commands::CHECKS_REPORT_CHANGED_EVENT, ());
    }
    fn settings_changed(&self) {
        crate::jev::settings::changed(&self.0);
    }
    fn preparation_recorded(&self, preparation: &antiburn_checks::fit::PreparationRecord) {
        record_preparation(preparation, |outcome| {
            crate::analytics::record_burn_check_input_prepared(&self.0, outcome)
        });
    }
}

pub(super) fn coordinator_capabilities(
    state: &InstalledChecks,
    generation: Option<u64>,
    admission: &Admission,
) -> (Option<ModelCapabilities>, Admission) {
    let saved = state
        .capabilities
        .read()
        .expect("installed capability lock");
    let Some(saved) = saved
        .as_ref()
        .filter(|saved| Some(saved.generation) == generation)
    else {
        return (None, admission.clone());
    };
    match &saved.result {
        Ok(capabilities) => (Some(capabilities.clone()), admission.clone()),
        Err(reason) if *admission == Admission::Ready => {
            (None, Admission::Unsupported(reason.clone()))
        }
        Err(_) => (None, admission.clone()),
    }
}

pub(super) struct InstalledDecisionEvaluator {
    runtime: Arc<dyn SelectedJobRuntime>,
    selected: Option<TypeSafeDecisionEvaluator>,
    generation: Option<u64>,
}

impl InstalledDecisionEvaluator {
    pub(super) fn factory(runtime: Arc<dyn SelectedJobRuntime>) -> Self {
        Self {
            runtime,
            selected: None,
            generation: None,
        }
    }

    pub(super) async fn for_assessment(
        runtime: Arc<dyn SelectedJobRuntime>,
        activation: &EffectiveActivation,
    ) -> Result<Option<Arc<Self>>> {
        let Some(snapshot) = runtime.worker().execution_snapshot() else {
            return Ok(None);
        };
        {
            let state = runtime
                .installed()
                .capabilities
                .read()
                .expect("installed capability lock");
            if let Some(saved) = state.as_ref().filter(|saved| {
                saved.generation == snapshot.generation && Instant::now() < saved.retry_at
            }) {
                let blocked = match &saved.result {
                    Ok(capabilities) => {
                        choice_admission(&activation.compiled.choice_requirements(), capabilities)
                            .is_err()
                    }
                    Err(_) => true,
                };
                if blocked {
                    return Ok(None);
                }
            }
        }
        let resolved = runtime.resolve(&snapshot).await;
        if !runtime.worker().key_is_current(snapshot.generation)
            || !crate::jev::settings::yaml_discovery_enabled(runtime.store())
        {
            return Ok(None);
        }
        let status = resolved.as_ref().map(Clone::clone).map_err(|error| {
            format!("The selected connection is unavailable: {error}. Check Settings → Checks.")
        });
        *runtime
            .installed()
            .capabilities
            .write()
            .expect("installed capability lock") = Some(CapabilityState {
            generation: snapshot.generation,
            result: status,
            retry_at: Instant::now() + DISCOVERY_INTERVAL,
        });
        runtime.installed().wake.notify_one();
        let capabilities = resolved?;
        if choice_admission(&activation.compiled.choice_requirements(), &capabilities).is_err() {
            return Ok(None);
        }
        let generation = snapshot.generation;
        let provider = usage_provider(snapshot.connection.provider);
        let model = snapshot.connection.model.clone();
        let cancellation_runtime = Arc::clone(&runtime);
        let activation_id = activation.id.clone();
        let rejection_runtime = Arc::clone(&runtime);
        let usage_runtime = Arc::clone(&runtime);
        let check_id = activation.id.definition_id.clone();
        let selected = TypeSafeDecisionEvaluator::from_selected_snapshot(
            snapshot.connection,
            snapshot.credential,
            capabilities,
        )
        .map_err(|_| anyhow::anyhow!("The selected connection could not prepare this assessment."))?
        .with_cancellation(move || {
            !current(&*cancellation_runtime, generation)
                || !activation_current(&*cancellation_runtime, &activation_id)
        })
        .with_authentication_rejection(move || {
            #[cfg(test)]
            rejection_runtime.stage(Stage::Authentication);
            if rejection_runtime
                .worker()
                .reject_authentication(rejection_runtime.store(), generation)
                .map_err(|error| error.to_string())?
            {
                rejection_runtime.settings_changed();
            }
            Ok(())
        })
        .with_usage(Arc::new(move |candidate, attempt, usage| {
            usage_runtime
                .store()
                .record_burn_check_decision_usage(
                    candidate,
                    attempt,
                    &check_id,
                    (provider, &model),
                    usage,
                    time::OffsetDateTime::now_utc().unix_timestamp(),
                )
                .map_err(|_| "Could not record Burn Check provider usage.".to_owned())?;
            #[cfg(test)]
            if usage.is_some() {
                usage_runtime.stage(Stage::ResponseReceipt);
            }
            Ok(())
        }));
        Ok(Some(Arc::new(Self {
            runtime,
            selected: Some(selected),
            generation: Some(generation),
        })))
    }

    fn selected(
        &self,
    ) -> std::result::Result<&TypeSafeDecisionEvaluator, antiburn_checks::fit::PreparationFailure>
    {
        self.selected
            .as_ref()
            .filter(|_| {
                self.generation
                    .is_some_and(|generation| current(&*self.runtime, generation))
            })
            .ok_or_else(|| antiburn_checks::fit::PreparationFailure {
                code: antiburn_checks::fit::PreparationFailureCode::ProviderUnavailable,
                detail: "The selected connection is unavailable.".into(),
                report: None,
                context: Default::default(),
            })
    }
}

fn current(runtime: &dyn SelectedJobRuntime, generation: u64) -> bool {
    runtime.worker().key_is_current(generation)
        && crate::jev::settings::yaml_discovery_enabled(runtime.store())
        && !runtime.installed().stopping.load(Ordering::Acquire)
}

fn activation_current(runtime: &dyn SelectedJobRuntime, id: &ActivationId) -> bool {
    let scope = StoreCheckStore::activation_scope_key(&id.scope);
    runtime
        .installed()
        .metadata
        .read()
        .expect("installed metadata lock")
        .iter()
        .any(|definition| {
            definition.check_id == id.definition_id
                && definition.activation_scope_key == scope
                && definition.generation == i64::try_from(id.generation.0).ok()
        })
}

impl DecisionEvaluator for InstalledDecisionEvaluator {
    fn preparation_recorded(&self, preparation: &antiburn_checks::fit::PreparationRecord) {
        self.runtime.preparation_recorded(preparation);
    }
    fn prepare(
        &self,
        activation: &EffectiveActivation,
        input: serde_json::Value,
    ) -> std::result::Result<
        antiburn_checks::fit::PreparedDecision,
        antiburn_checks::fit::PreparationFailure,
    > {
        #[cfg(test)]
        self.runtime.stage(Stage::Preparation);
        self.selected()?.prepare(activation, input)
    }
    fn validate_prepared(
        &self,
        activation: &EffectiveActivation,
        prepared: &antiburn_checks::fit::PreparedDecision,
    ) -> std::result::Result<(), antiburn_checks::fit::PreparationFailure> {
        self.selected()?.validate_prepared(activation, prepared)
    }
    fn evaluate<'a>(
        &'a self,
        activation: &'a EffectiveActivation,
        candidate: &'a antiburn_checks::domain::FrozenCandidate,
        input: &'a serde_json::Value,
        attempt: u8,
    ) -> DecisionFuture<'a> {
        Box::pin(async move {
            self.selected()
                .map_err(unavailable_evaluation)?
                .evaluate(activation, candidate, input, attempt)
                .await
        })
    }
    fn evaluate_prepared<'a>(
        &'a self,
        activation: &'a EffectiveActivation,
        candidate: &'a antiburn_checks::domain::FrozenCandidate,
        prepared: &'a antiburn_checks::fit::PreparedDecision,
        attempt: u8,
    ) -> DecisionFuture<'a> {
        Box::pin(async move {
            self.selected()
                .map_err(unavailable_evaluation)?
                .evaluate_prepared(activation, candidate, prepared, attempt)
                .await
        })
    }
}

fn unavailable_evaluation(error: antiburn_checks::fit::PreparationFailure) -> EvaluationFailure {
    EvaluationFailure {
        code: EvaluationFailureCode::Cancelled,
        kind: AttemptFailure::Permanent,
        detail: error.detail,
        attempts: 0,
    }
}
