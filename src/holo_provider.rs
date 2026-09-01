//! Provider selection and transactional lifecycle for resolved `.holo` applications.

use crate::application_plan::{
    layer_kind_name, ApplicationPlan, ApplicationPlanReport, HoloIdentity, ProviderAvailability,
    ProviderContext, ResolvedLayer,
};
use crate::error::{LiveError, Result};
use crate::holo_capability::{EffectiveGrant, RequestedCapabilities};
use hologram::space::LayerKind;
use hologram_view_surface::{
    PortableViewIntentHandler, ViewAttachmentId, ViewIntentRequest, ViewIntentResponse,
    MAX_INTENT_OUTPUTS, MAX_INTENT_OUTPUT_BYTES, VIEW_INTENT_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Planned,
    Preparing,
    Running,
    Stopping,
    Stopped,
    Failed,
}

impl LifecycleState {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    const fn value(self) -> u8 {
        match self {
            Self::Planned => 0,
            Self::Preparing => 1,
            Self::Running => 2,
            Self::Stopping => 3,
            Self::Stopped => 4,
            Self::Failed => 5,
        }
    }

    const fn from_value(value: u8) -> Self {
        match value {
            0 => Self::Planned,
            1 => Self::Preparing,
            2 => Self::Running,
            3 => Self::Stopping,
            4 => Self::Stopped,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTarget {
    Direct,
    Resident,
}

impl ProviderTarget {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Resident => "resident",
        }
    }
}

#[derive(Clone)]
pub struct LayerPrepareContext {
    pub identity: HoloIdentity,
    pub effective_grant: EffectiveGrant,
    pub requested_capabilities: RequestedCapabilities,
    pub layer: ResolvedLayer,
    pub target: ProviderTarget,
    pub view_intents: Arc<dyn PortableViewIntentHandler>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerCompletion {
    Returned,
    Exited { code: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerCompletionRole {
    ExitBearing,
    NonExitBearing,
}

pub const fn layer_completion_role(kind: LayerKind) -> LayerCompletionRole {
    match kind {
        LayerKind::WasmCodemodule | LayerKind::RootfsImage => LayerCompletionRole::ExitBearing,
        LayerKind::TensorPlan | LayerKind::View | LayerKind::InferenceModel => {
            LayerCompletionRole::NonExitBearing
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerInvocation {
    pub outputs: Vec<Vec<u8>>,
    pub completion: LayerCompletion,
    pub elapsed_micros: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayerRuntimeStatus {
    pub resident_bytes: usize,
    pub queued: usize,
    pub processed: usize,
}

#[tonic::async_trait]
pub trait LayerProvider: Send + Sync {
    fn kind(&self) -> LayerKind;
    /// Canonical Wasm guest contract served by this provider. Non-Wasm
    /// providers have no contract selector.
    fn contract(&self) -> Option<&'static str>;
    fn name(&self) -> &'static str;
    fn availability(
        &self,
        context: &ProviderContext<'_>,
        target: ProviderTarget,
    ) -> Result<(), String>;
    async fn prepare(&self, context: LayerPrepareContext) -> Result<Arc<dyn PreparedLayer>>;
}

#[tonic::async_trait]
pub trait PreparedLayer: Send + Sync {
    fn position(&self) -> u32;
    async fn start(&self) -> Result<()>;
    async fn invoke(&self, inputs: Vec<Vec<u8>>) -> Result<LayerInvocation>;
    async fn stop(&self) -> Result<()>;
    fn status(&self) -> LayerRuntimeStatus;
}

pub struct ProviderRegistry {
    target: ProviderTarget,
    providers: Vec<Arc<dyn LayerProvider>>,
}

impl ProviderRegistry {
    pub fn new(target: ProviderTarget, providers: Vec<Arc<dyn LayerProvider>>) -> Result<Self> {
        for (index, provider) in providers.iter().enumerate() {
            if provider.kind() == LayerKind::WasmCodemodule {
                let contract = provider.contract().ok_or_else(|| {
                    LiveError::Config(format!(
                        "{} Wasm provider {} has no guest contract selector",
                        target.name(),
                        provider.name()
                    ))
                })?;
                let normalized = crate::holo_contract::normalize_wasm_contract(contract)
                    .map_err(LiveError::Config)?;
                if normalized != contract {
                    return Err(LiveError::Config(format!(
                        "{} Wasm provider {} must declare the explicit canonical contract {normalized}",
                        target.name(),
                        provider.name()
                    )));
                }
            }
            if providers[..index].iter().any(|existing| {
                existing.kind() == provider.kind() && existing.contract() == provider.contract()
            }) {
                return Err(LiveError::Config(format!(
                    "duplicate {} provider for {} layers{}",
                    target.name(),
                    layer_kind_name(provider.kind()),
                    provider
                        .contract()
                        .map(|contract| format!(" using contract {contract}"))
                        .unwrap_or_default()
                )));
            }
            if provider.kind() != LayerKind::WasmCodemodule && provider.contract().is_some() {
                return Err(LiveError::Config(format!(
                    "{} provider {} declares a Wasm contract for {} layers",
                    target.name(),
                    provider.name(),
                    layer_kind_name(provider.kind())
                )));
            }
        }
        Ok(Self { target, providers })
    }

    pub const fn target(&self) -> ProviderTarget {
        self.target
    }

    pub fn evaluate(&self, report: &mut ApplicationPlanReport) {
        report.evaluate_providers(|context| self.availability(&context));
    }

    fn availability(&self, context: &ProviderContext<'_>) -> ProviderAvailability {
        let provider = match self.select(context.kind, context.aux) {
            Ok(Some(provider)) => provider,
            Err(reason) => return ProviderAvailability::Unavailable { reason },
            Ok(None) => {
                let reason = if context.kind == LayerKind::InferenceModel {
                    format!(
                        "inference provider for service {} ({}) is not connected to {} execution",
                        context.entry,
                        context.aux,
                        self.target.name()
                    )
                } else if context.kind == LayerKind::WasmCodemodule {
                    let contract = crate::holo_contract::normalize_wasm_contract(context.aux)
                        .unwrap_or(context.aux);
                    format!(
                        "{} execution has no provider for Wasm guest contract {contract} (entry {})",
                        self.target.name(),
                        context.entry
                    )
                } else {
                    format!(
                        "{} execution has no provider for {} layer entry {}",
                        self.target.name(),
                        layer_kind_name(context.kind),
                        context.entry
                    )
                };
                return ProviderAvailability::Unavailable { reason };
            }
        };
        match provider.availability(context, self.target) {
            Ok(()) => ProviderAvailability::Available {
                provider: provider.name().to_owned(),
            },
            Err(reason) => ProviderAvailability::Unavailable { reason },
        }
    }

    fn select(
        &self,
        kind: LayerKind,
        aux: &str,
    ) -> std::result::Result<Option<&Arc<dyn LayerProvider>>, String> {
        let contract = if kind == LayerKind::WasmCodemodule {
            Some(crate::holo_contract::normalize_wasm_contract(aux)?)
        } else {
            None
        };
        Ok(self
            .providers
            .iter()
            .find(|provider| provider.kind() == kind && provider.contract() == contract))
    }
}

pub struct RunningApplication {
    identity: HoloIdentity,
    primary_layer: u32,
    layers: Vec<PreparedApplicationLayer>,
    state: AtomicU8,
    invocations: Vec<Arc<Mutex<()>>>,
}

struct PreparedApplicationLayer {
    application_index: usize,
    application_kappa: String,
    layer: Arc<dyn PreparedLayer>,
}

struct ApplicationIntentBroker {
    application_kappa: String,
    primary: OnceLock<Arc<dyn PreparedLayer>>,
    invocation: Arc<Mutex<()>>,
}

impl ApplicationIntentBroker {
    fn new(application_kappa: &str, invocation: Arc<Mutex<()>>) -> Self {
        Self {
            application_kappa: application_kappa.to_owned(),
            primary: OnceLock::new(),
            invocation,
        }
    }

    fn bind(&self, primary: Arc<dyn PreparedLayer>) -> Result<()> {
        self.primary.set(primary).map_err(|_| {
            LiveError::Conflict(format!(
                "application {} intent broker was bound more than once",
                self.application_kappa
            ))
        })
    }
}

impl PortableViewIntentHandler for ApplicationIntentBroker {
    fn handle<'a>(
        &'a self,
        id: &'a ViewAttachmentId,
        request: ViewIntentRequest,
    ) -> hologram_view_surface::IntentFuture<'a> {
        Box::pin(async move {
            request.validate()?;
            if id.application_kappa != self.application_kappa {
                return Err("portable View intent does not belong to this application".to_owned());
            }
            let primary = self
                .primary
                .get()
                .ok_or_else(|| "portable View application primary is not ready".to_owned())?;
            let _invocation = self.invocation.lock().await;
            let outcome = primary
                .invoke(vec![request.payload.into_bytes()])
                .await
                .map_err(|error| error.to_string())?;
            if outcome.outputs.len() > MAX_INTENT_OUTPUTS {
                return Err(format!(
                    "portable View intent returned {} outputs; maximum is {MAX_INTENT_OUTPUTS}",
                    outcome.outputs.len()
                ));
            }
            let mut output_bytes = 0usize;
            let outputs = outcome
                .outputs
                .into_iter()
                .enumerate()
                .map(|(index, output)| {
                    output_bytes = output_bytes.saturating_add(output.len());
                    String::from_utf8(output).map_err(|error| {
                        format!("portable View intent output {index} is not UTF-8: {error}")
                    })
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if output_bytes > MAX_INTENT_OUTPUT_BYTES {
                return Err(format!(
                    "portable View intent returned {output_bytes} bytes; maximum is {MAX_INTENT_OUTPUT_BYTES}"
                ));
            }
            Ok(ViewIntentResponse {
                version: VIEW_INTENT_VERSION,
                outputs,
            })
        })
    }
}

impl RunningApplication {
    pub fn state(&self) -> LifecycleState {
        LifecycleState::from_value(self.state.load(Ordering::Acquire))
    }

    pub fn identity(&self) -> &HoloIdentity {
        &self.identity
    }

    pub fn application_kappas(&self) -> Vec<&str> {
        let mut kappas = Vec::new();
        for layer in &self.layers {
            if kappas.last().copied() != Some(layer.application_kappa.as_str()) {
                kappas.push(layer.application_kappa.as_str());
            }
        }
        kappas
    }

    pub fn status(&self) -> LayerRuntimeStatus {
        self.layers.iter().map(|layer| layer.layer.status()).fold(
            LayerRuntimeStatus::default(),
            |mut total, layer| {
                total.resident_bytes = total.resident_bytes.saturating_add(layer.resident_bytes);
                total.queued = total.queued.saturating_add(layer.queued);
                total.processed = total.processed.saturating_add(layer.processed);
                total
            },
        )
    }

    pub async fn invoke(&self, inputs: Vec<Vec<u8>>) -> Result<LayerInvocation> {
        let invocation = self.invocations.first().ok_or_else(|| {
            LiveError::Conflict(format!(
                "running application {} lost its invocation gate",
                self.identity.application_kappa
            ))
        })?;
        let _invocation = invocation.lock().await;
        if self.state() != LifecycleState::Running {
            return Err(LiveError::Conflict(format!(
                "application {} is not running (state {:?})",
                self.identity.application_kappa,
                self.state()
            )));
        }
        let layer = self
            .layers
            .iter()
            .find(|layer| {
                layer.application_index == 0 && layer.layer.position() == self.primary_layer
            })
            .ok_or_else(|| {
                LiveError::Conflict(format!(
                    "running application {} lost primary layer {}",
                    self.identity.application_kappa, self.primary_layer
                ))
            })?;
        layer.layer.invoke(inputs).await
    }

    pub async fn invoke_then_stop(&self, inputs: Vec<Vec<u8>>) -> Result<LayerInvocation> {
        let invocation = self.invoke(inputs).await;
        let stopped = self.stop().await;
        match (invocation, stopped) {
            (Ok(outcome), Ok(())) => Ok(outcome),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(stop_error)) => {
                Err(with_rollback(error, vec![stop_error.to_string()]))
            }
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let mut invocation_guards = Vec::with_capacity(self.invocations.len());
        for invocation in &self.invocations {
            invocation_guards.push(invocation.lock().await);
        }
        loop {
            let state = self.state();
            match state {
                LifecycleState::Stopped => return Ok(()),
                LifecycleState::Stopping => {
                    return Err(LiveError::Conflict(format!(
                        "application {} is already stopping",
                        self.identity.application_kappa
                    )));
                }
                LifecycleState::Running | LifecycleState::Failed => {
                    if self
                        .state
                        .compare_exchange(
                            state.value(),
                            LifecycleState::Stopping.value(),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        break;
                    }
                }
                LifecycleState::Planned | LifecycleState::Preparing => {
                    return Err(LiveError::Conflict(format!(
                        "application {} cannot stop from state {state:?}",
                        self.identity.application_kappa
                    )));
                }
            }
        }

        let failures = stop_reverse(&self.identity, &self.layers, "stop").await;
        if failures.is_empty() {
            self.state
                .store(LifecycleState::Stopped.value(), Ordering::Release);
            Ok(())
        } else {
            self.state
                .store(LifecycleState::Failed.value(), Ordering::Release);
            Err(LiveError::Conflict(format!(
                "stop application {} failed: {}",
                self.identity.application_kappa,
                failures.join("; ")
            )))
        }
    }
}

pub async fn prepare_and_start(
    plan: &ApplicationPlan,
    registry: &ProviderRegistry,
) -> Result<RunningApplication> {
    let grant = EffectiveGrant::local_baseline();
    prepare_and_start_with_grant(plan, registry, &grant).await
}

pub async fn prepare_and_start_with_grant(
    plan: &ApplicationPlan,
    registry: &ProviderRegistry,
    effective_grant: &EffectiveGrant,
) -> Result<RunningApplication> {
    let admitted_grants = plan.admitted_grants(effective_grant)?;
    prepare_and_start_with_admitted_grants(plan, registry, &admitted_grants).await
}

pub(crate) async fn prepare_and_start_with_admitted_grants(
    plan: &ApplicationPlan,
    registry: &ProviderRegistry,
    admitted_grants: &HashMap<usize, EffectiveGrant>,
) -> Result<RunningApplication> {
    let primary_layer = plan.primary_layer.ok_or_else(|| {
        LiveError::Capability(format!(
            "application {} has no primary exit-bearing layer",
            plan.identity.application_kappa
        ))
    })?;
    let primary = plan
        .layers
        .iter()
        .find(|layer| layer.position == primary_layer)
        .ok_or_else(|| {
            LiveError::InvalidHolo(format!(
                "application {} primary layer {primary_layer} is missing",
                plan.identity.application_kappa
            ))
        })?;
    if layer_completion_role(primary.kind) == LayerCompletionRole::NonExitBearing {
        return Err(LiveError::Capability(format!(
            "application {} cannot use non-exit-bearing {} layer {primary_layer} as its primary",
            plan.identity.application_kappa,
            layer_kind_name(primary.kind)
        )));
    }
    let state = AtomicU8::new(LifecycleState::Preparing.value());
    let mut invocations = Vec::new();
    let application_order = lifecycle_application_order(plan)?;
    let layer_count = plan.layers.len()
        + plan
            .children
            .iter()
            .map(|child| child.layers.len())
            .sum::<usize>();
    let mut prepared = Vec::with_capacity(layer_count);
    for application_index in application_order {
        let (application_kappa, layers) = application_layers(plan, application_index)?;
        let primary_layer = if layers.iter().any(|layer| layer.kind == LayerKind::View) {
            match application_primary_layer(plan, application_index) {
                Ok(primary) => Some(primary),
                Err(error) => {
                    state.store(LifecycleState::Failed.value(), Ordering::Release);
                    return Err(with_rollback(
                        error,
                        stop_reverse(&plan.identity, &prepared, "rollback").await,
                    ));
                }
            }
        } else {
            None
        };
        let invocation = Arc::new(Mutex::new(()));
        invocations.push(invocation.clone());
        let view_intents = Arc::new(ApplicationIntentBroker::new(application_kappa, invocation));
        let prepared_start = prepared.len();
        let requested_capabilities = application_requested_capabilities(plan, application_index)?;
        let grant = admitted_grants.get(&application_index).ok_or_else(|| {
            LiveError::Conflict(format!(
                "runtime lost admitted grant for application {application_kappa}"
            ))
        })?;
        for layer in layers {
            let provider = registry
                .select(layer.kind, &layer.aux)
                .map_err(LiveError::Capability)?
                .ok_or_else(|| {
                    LiveError::Capability(format!(
                        "application {application_kappa} layer {} has no {} provider for {}{}",
                        layer.position,
                        registry.target.name(),
                        layer_kind_name(layer.kind),
                        if layer.kind == LayerKind::WasmCodemodule {
                            format!(
                                " contract {}",
                                crate::holo_contract::normalize_wasm_contract(&layer.aux)
                                    .unwrap_or(&layer.aux)
                            )
                        } else {
                            String::new()
                        }
                    ))
                })?;
            if layer.provider != provider.name() {
                return Err(LiveError::Conflict(format!(
                    "application {application_kappa} layer {} selected provider {}, but registry resolved {}",
                    layer.position,
                    layer.provider,
                    provider.name()
                )));
            }
            tracing::info!(
                application_kappa,
                archive_kappa = %plan.identity.archive_kappa,
                layer_position = layer.position,
                provider = provider.name(),
                lifecycle_phase = "prepare",
                "preparing holo layer"
            );
            let mut identity = plan.identity.clone();
            application_kappa.clone_into(&mut identity.application_kappa);
            match provider
                .prepare(LayerPrepareContext {
                    identity,
                    effective_grant: grant.clone(),
                    requested_capabilities: requested_capabilities.clone(),
                    layer: layer.clone(),
                    target: registry.target,
                    view_intents: view_intents.clone(),
                })
                .await
            {
                Ok(instance) => prepared.push(PreparedApplicationLayer {
                    application_index,
                    application_kappa: application_kappa.to_owned(),
                    layer: instance,
                }),
                Err(error) => {
                    state.store(LifecycleState::Failed.value(), Ordering::Release);
                    return Err(with_rollback(
                        error,
                        stop_reverse(&plan.identity, &prepared, "rollback").await,
                    ));
                }
            }
        }
        if let Some(primary_layer) = primary_layer {
            let binding = prepared[prepared_start..]
                .iter()
                .find(|layer| layer.layer.position() == primary_layer)
                .ok_or_else(|| {
                    LiveError::Conflict(format!(
                        "application {application_kappa} lost primary layer {primary_layer} while binding View intents"
                    ))
                })
                .and_then(|primary| view_intents.bind(primary.layer.clone()));
            if let Err(error) = binding {
                state.store(LifecycleState::Failed.value(), Ordering::Release);
                return Err(with_rollback(
                    error,
                    stop_reverse(&plan.identity, &prepared, "rollback").await,
                ));
            }
        }
    }

    for layer in &prepared {
        tracing::info!(
            application_kappa = %layer.application_kappa,
            archive_kappa = %plan.identity.archive_kappa,
            layer_position = layer.layer.position(),
            lifecycle_phase = "start",
            "starting holo layer"
        );
        if let Err(error) = layer.layer.start().await {
            state.store(LifecycleState::Failed.value(), Ordering::Release);
            return Err(with_rollback(
                error,
                stop_reverse(&plan.identity, &prepared, "rollback").await,
            ));
        }
    }
    state.store(LifecycleState::Running.value(), Ordering::Release);
    Ok(RunningApplication {
        identity: plan.identity.clone(),
        primary_layer,
        layers: prepared,
        state,
        invocations,
    })
}

fn application_layers(
    plan: &ApplicationPlan,
    application_index: usize,
) -> Result<(&str, &[ResolvedLayer])> {
    if application_index == 0 {
        return Ok((&plan.identity.application_kappa, &plan.layers));
    }
    plan.children
        .iter()
        .find(|child| child.application_index == application_index)
        .map(|child| (child.application_kappa.as_str(), child.layers.as_slice()))
        .ok_or_else(|| {
            LiveError::Conflict(format!(
                "runtime lost planned child application index {application_index}"
            ))
        })
}

fn application_requested_capabilities(
    plan: &ApplicationPlan,
    application_index: usize,
) -> Result<&RequestedCapabilities> {
    if application_index == 0 {
        return Ok(&plan.requested_capabilities);
    }
    plan.children
        .iter()
        .find(|child| child.application_index == application_index)
        .map(|child| &child.requested_capabilities)
        .ok_or_else(|| {
            LiveError::Conflict(format!(
                "runtime lost requested capabilities for application index {application_index}"
            ))
        })
}

fn application_primary_layer(plan: &ApplicationPlan, application_index: usize) -> Result<u32> {
    if application_index == 0 {
        return plan.primary_layer.ok_or_else(|| {
            LiveError::Capability(format!(
                "application {} has no primary exit-bearing layer",
                plan.identity.application_kappa
            ))
        });
    }
    plan.children
        .iter()
        .find(|child| child.application_index == application_index)
        .ok_or_else(|| {
            LiveError::Conflict(format!(
                "runtime lost planned child application index {application_index}"
            ))
        })?
        .primary_layer
        .ok_or_else(|| {
            LiveError::Capability(format!(
                "child application index {application_index} has no primary exit-bearing layer"
            ))
        })
}

fn lifecycle_application_order(plan: &ApplicationPlan) -> Result<Vec<usize>> {
    let mut children_by_parent: HashMap<usize, Vec<_>> = HashMap::new();
    for child in &plan.children {
        children_by_parent
            .entry(child.parent_application_index)
            .or_default()
            .push(child);
    }
    for children in children_by_parent.values_mut() {
        children.sort_by_key(|child| (child.position, child.application_index));
    }

    let mut order = Vec::with_capacity(plan.children.len() + 1);
    let mut stack = vec![0usize];
    while let Some(application_index) = stack.pop() {
        order.push(application_index);
        if let Some(children) = children_by_parent.get(&application_index) {
            stack.extend(children.iter().rev().map(|child| child.application_index));
        }
    }
    if order.len() != plan.children.len() + 1 {
        return Err(LiveError::Conflict(format!(
            "runtime lifecycle traversal reached {} of {} planned applications",
            order.len(),
            plan.children.len() + 1
        )));
    }
    Ok(order)
}

async fn stop_reverse(
    identity: &HoloIdentity,
    layers: &[PreparedApplicationLayer],
    phase: &'static str,
) -> Vec<String> {
    let mut failures = Vec::new();
    for layer in layers.iter().rev() {
        tracing::info!(
            application_kappa = %layer.application_kappa,
            archive_kappa = %identity.archive_kappa,
            layer_position = layer.layer.position(),
            lifecycle_phase = phase,
            "stopping holo layer"
        );
        if let Err(error) = layer.layer.stop().await {
            failures.push(format!(
                "application {} layer {}: {error}",
                layer.application_kappa,
                layer.layer.position()
            ));
        }
    }
    failures
}

fn with_rollback(error: LiveError, failures: Vec<String>) -> LiveError {
    if failures.is_empty() {
        return error;
    }
    let message = format!("{error}; rollback failures: {}", failures.join("; "));
    match error {
        LiveError::Io(_) => LiveError::Io(message),
        LiveError::Config(_) => LiveError::Config(message),
        LiveError::Protocol(_) => LiveError::Protocol(message),
        LiveError::Transport(_) => LiveError::Transport(message),
        LiveError::Authentication(_) => LiveError::Authentication(message),
        LiveError::Authorization(_) => LiveError::Authorization(message),
        LiveError::Capability(_) => LiveError::Capability(message),
        LiveError::NotFound(_) => LiveError::NotFound(message),
        LiveError::Conflict(_) => LiveError::Conflict(message),
        LiveError::InvalidHolo(_) => LiveError::InvalidHolo(message),
        LiveError::UnknownCommitState(_) => LiveError::UnknownCommitState(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application_plan::{explain_application, PlanLimits};
    use hologram::archive::HoloWriter;
    use hologram::space::{address_bytes, AppManifest, Layer, Realization};
    use hologram_view_surface::{
        PortableViewAttachment, PortableViewSurface, SurfaceFuture, ViewSurfaceRegistry,
        APPLICATION_INVOKE_INTENT,
    };
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    fn wasm_layer(content: hologram::space::KappaLabel71, entry: &str) -> Layer {
        Layer::wasm_with_contract(content, entry, crate::holo_contract::WASM_CONTRACT_CORE_V1)
    }

    fn test_capabilities() -> &'static [u8] {
        static CAPABILITIES: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        CAPABILITIES.get_or_init(crate::holo_capability::empty_canonical)
    }

    fn finish_archive(manifest: &AppManifest, blobs: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        let directory =
            crate::holo_directory::derive(manifest, blobs.iter().copied()).expect("directory");
        writer.add_extension(
            crate::holo_directory::DIRECTORY_EXTENSION_KEY,
            crate::holo_directory::encode(&directory).expect("encode directory"),
        );
        for (kappa, bytes) in blobs {
            writer.add_content_blob(*kappa, *bytes);
        }
        writer.finish().expect("archive")
    }

    struct SyntheticProvider {
        events: Arc<Mutex<Vec<String>>>,
        fail_prepare: Option<u32>,
        fail_start: Option<u32>,
        fail_stop: Option<u32>,
    }

    struct RecordingViewSurface {
        events: Arc<Mutex<Vec<String>>>,
        attachment: Mutex<Option<PortableViewAttachment>>,
    }

    impl PortableViewSurface for RecordingViewSurface {
        fn attach(&self, view: PortableViewAttachment) -> SurfaceFuture<'_> {
            Box::pin(async move {
                record(&self.events, "attach:view".to_owned());
                *self.attachment.lock().expect("attachment") = Some(view);
                Ok(())
            })
        }

        fn detach<'a>(&'a self, _id: &'a ViewAttachmentId) -> SurfaceFuture<'a> {
            Box::pin(async move {
                record(&self.events, "detach:view".to_owned());
                Ok(())
            })
        }
    }

    #[tonic::async_trait]
    impl LayerProvider for SyntheticProvider {
        fn kind(&self) -> LayerKind {
            LayerKind::WasmCodemodule
        }

        fn contract(&self) -> Option<&'static str> {
            Some(crate::holo_contract::WASM_CONTRACT_CORE_V1)
        }

        fn name(&self) -> &'static str {
            "synthetic-wasm"
        }

        fn availability(
            &self,
            _context: &ProviderContext<'_>,
            _target: ProviderTarget,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn prepare(&self, context: LayerPrepareContext) -> Result<Arc<dyn PreparedLayer>> {
            record(&self.events, format!("prepare:{}", context.layer.position));
            if self.fail_prepare == Some(context.layer.position) {
                return Err(LiveError::Conflict(format!(
                    "prepare failed at {}",
                    context.layer.position
                )));
            }
            Ok(Arc::new(SyntheticLayer {
                position: context.layer.position,
                events: self.events.clone(),
                fail_start: self.fail_start == Some(context.layer.position),
                fail_stop: self.fail_stop == Some(context.layer.position),
                stopped: AtomicBool::new(false),
            }))
        }
    }

    struct SyntheticLayer {
        position: u32,
        events: Arc<Mutex<Vec<String>>>,
        fail_start: bool,
        fail_stop: bool,
        stopped: AtomicBool,
    }

    struct NamedProvider {
        events: Arc<Mutex<Vec<String>>>,
        fail_prepare: Option<&'static str>,
        fail_start: Option<&'static str>,
    }

    #[tonic::async_trait]
    impl LayerProvider for NamedProvider {
        fn kind(&self) -> LayerKind {
            LayerKind::WasmCodemodule
        }

        fn contract(&self) -> Option<&'static str> {
            Some(crate::holo_contract::WASM_CONTRACT_CORE_V1)
        }

        fn name(&self) -> &'static str {
            "synthetic-wasm"
        }

        fn availability(
            &self,
            _context: &ProviderContext<'_>,
            _target: ProviderTarget,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn prepare(&self, context: LayerPrepareContext) -> Result<Arc<dyn PreparedLayer>> {
            record(
                &self.events,
                format!(
                    "prepare:{}:{}",
                    context.layer.entry,
                    context.effective_grant.source.name()
                ),
            );
            if self.fail_prepare == Some(context.layer.entry.as_str()) {
                return Err(LiveError::Conflict(format!(
                    "prepare failed at {}",
                    context.layer.entry
                )));
            }
            let fail_start = self.fail_start == Some(context.layer.entry.as_str());
            Ok(Arc::new(NamedLayer {
                position: context.layer.position,
                entry: context.layer.entry,
                events: self.events.clone(),
                fail_start,
            }))
        }
    }

    struct NamedLayer {
        position: u32,
        entry: String,
        events: Arc<Mutex<Vec<String>>>,
        fail_start: bool,
    }

    #[tonic::async_trait]
    impl PreparedLayer for NamedLayer {
        fn position(&self) -> u32 {
            self.position
        }

        async fn start(&self) -> Result<()> {
            record(&self.events, format!("start:{}", self.entry));
            if self.fail_start {
                Err(LiveError::Conflict(format!(
                    "start failed at {}",
                    self.entry
                )))
            } else {
                Ok(())
            }
        }

        async fn invoke(&self, inputs: Vec<Vec<u8>>) -> Result<LayerInvocation> {
            record(&self.events, format!("invoke:{}", self.entry));
            Ok(LayerInvocation {
                outputs: inputs,
                completion: LayerCompletion::Returned,
                elapsed_micros: 1,
            })
        }

        async fn stop(&self) -> Result<()> {
            record(&self.events, format!("stop:{}", self.entry));
            Ok(())
        }

        fn status(&self) -> LayerRuntimeStatus {
            LayerRuntimeStatus {
                resident_bytes: 10,
                ..LayerRuntimeStatus::default()
            }
        }
    }

    #[tonic::async_trait]
    impl PreparedLayer for SyntheticLayer {
        fn position(&self) -> u32 {
            self.position
        }

        async fn start(&self) -> Result<()> {
            record(&self.events, format!("start:{}", self.position));
            if self.fail_start {
                Err(LiveError::Conflict(format!(
                    "start failed at {}",
                    self.position
                )))
            } else {
                Ok(())
            }
        }

        async fn invoke(&self, inputs: Vec<Vec<u8>>) -> Result<LayerInvocation> {
            record(&self.events, format!("invoke:{}", self.position));
            Ok(LayerInvocation {
                outputs: inputs,
                completion: LayerCompletion::Returned,
                elapsed_micros: 1,
            })
        }

        async fn stop(&self) -> Result<()> {
            if self.stopped.swap(true, Ordering::AcqRel) {
                return Ok(());
            }
            record(&self.events, format!("stop:{}", self.position));
            if self.fail_stop {
                self.stopped.store(false, Ordering::Release);
                Err(LiveError::Conflict(format!(
                    "stop failed at {}",
                    self.position
                )))
            } else {
                Ok(())
            }
        }

        fn status(&self) -> LayerRuntimeStatus {
            LayerRuntimeStatus {
                resident_bytes: 10,
                ..LayerRuntimeStatus::default()
            }
        }
    }

    fn registry(
        events: Arc<Mutex<Vec<String>>>,
        fail_prepare: Option<u32>,
        fail_start: Option<u32>,
        fail_stop: Option<u32>,
    ) -> ProviderRegistry {
        ProviderRegistry::new(
            ProviderTarget::Direct,
            vec![Arc::new(SyntheticProvider {
                events,
                fail_prepare,
                fail_start,
                fail_stop,
            })],
        )
        .expect("registry")
    }

    fn named_registry(events: Arc<Mutex<Vec<String>>>) -> ProviderRegistry {
        fallible_named_registry(events, None, None)
    }

    fn fallible_named_registry(
        events: Arc<Mutex<Vec<String>>>,
        fail_prepare: Option<&'static str>,
        fail_start: Option<&'static str>,
    ) -> ProviderRegistry {
        ProviderRegistry::new(
            ProviderTarget::Direct,
            vec![Arc::new(NamedProvider {
                events,
                fail_prepare,
                fail_start,
            })],
        )
        .expect("named registry")
    }

    fn plan(registry: &ProviderRegistry) -> ApplicationPlan {
        plan_with_capabilities(registry, test_capabilities())
    }

    fn plan_with_capabilities(registry: &ProviderRegistry, capabilities: &[u8]) -> ApplicationPlan {
        let first = b"first wasm";
        let second = b"second wasm";
        let manifest = AppManifest {
            primary: Some(1),
            requires: address_bytes(capabilities),
            layers: vec![
                wasm_layer(address_bytes(first), "first"),
                wasm_layer(address_bytes(second), "second"),
            ],
            children: Vec::new(),
        };
        let capabilities_kappa = address_bytes(capabilities);
        let first_kappa = address_bytes(first);
        let second_kappa = address_bytes(second);
        let bytes = finish_archive(
            &manifest,
            &[
                (capabilities_kappa.as_bytes(), capabilities),
                (first_kappa.as_bytes(), first),
                (second_kappa.as_bytes(), second),
            ],
        );
        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("plan report");
        registry.evaluate(&mut report);
        report.into_application_plan().expect("strict plan")
    }

    fn composed_view_plan(registry: &ProviderRegistry) -> ApplicationPlan {
        let capabilities = test_capabilities();
        let wasm = b"synthetic primary";
        let view = crate::holo_view::encode(&crate::holo_view::ViewBundle {
            version: crate::holo_view::VIEW_BUNDLE_VERSION,
            entry: crate::holo_view::PORTABLE_ENTRY.to_owned(),
            files: vec![crate::holo_view::ViewFile {
                path: crate::holo_view::PORTABLE_ENTRY.to_owned(),
                bytes: b"<button>invoke</button>".to_vec(),
            }],
        })
        .expect("view bundle");
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![
                wasm_layer(address_bytes(wasm), "root"),
                Layer::view(address_bytes(&view), crate::holo_view::PORTABLE_SURFACE),
            ],
            children: Vec::new(),
        };
        let bytes = finish_archive(
            &manifest,
            &[
                (address_bytes(capabilities).as_bytes(), capabilities),
                (address_bytes(wasm).as_bytes(), wasm),
                (address_bytes(&view).as_bytes(), &view),
            ],
        );
        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("plan report");
        registry.evaluate(&mut report);
        report.into_application_plan().expect("strict plan")
    }

    #[tokio::test]
    async fn composed_view_intent_invokes_its_primary_and_stops_in_reverse() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let surface = Arc::new(RecordingViewSurface {
            events: events.clone(),
            attachment: Mutex::new(None),
        });
        let surfaces = Arc::new(ViewSurfaceRegistry::new());
        surfaces
            .register_portable(surface.clone())
            .expect("register surface");
        let registry = ProviderRegistry::new(
            ProviderTarget::Direct,
            vec![
                Arc::new(NamedProvider {
                    events: events.clone(),
                    fail_prepare: None,
                    fail_start: None,
                }),
                Arc::new(crate::holo_view_provider::ViewProvider::new(surfaces)),
            ],
        )
        .expect("registry");
        let plan = composed_view_plan(&registry);
        let application = prepare_and_start(&plan, &registry).await.expect("start");
        let attachment = surface
            .attachment
            .lock()
            .expect("attachment")
            .clone()
            .expect("attached view");

        let response = attachment
            .intents
            .handle(
                &attachment.id,
                ViewIntentRequest {
                    version: VIEW_INTENT_VERSION,
                    name: APPLICATION_INVOKE_INTENT.to_owned(),
                    payload: "hello intent".to_owned(),
                },
            )
            .await
            .expect("invoke primary");
        assert_eq!(response.outputs, vec!["hello intent"]);
        application.stop().await.expect("stop");
        assert_eq!(
            events.lock().expect("events").as_slice(),
            [
                "prepare:root:local_baseline",
                "start:root",
                "attach:view",
                "invoke:root",
                "detach:view",
                "stop:root"
            ]
        );
    }

    fn plan_with_child(
        registry: &ProviderRegistry,
        root_request: &[u8],
        delegated: &[u8],
        child_request: &[u8],
    ) -> ApplicationPlan {
        let root_layer = b"root wasm";
        let child_layer = b"child wasm";
        let child_manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(child_request),
            layers: vec![wasm_layer(address_bytes(child_layer), "child")],
            children: Vec::new(),
        };
        let child_manifest_bytes = child_manifest.canonicalize();
        let child_kappa = address_bytes(&child_manifest_bytes);
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(root_request),
            layers: vec![wasm_layer(address_bytes(root_layer), "root")],
            children: vec![(child_kappa, address_bytes(delegated))],
        };
        let mut blobs = std::collections::BTreeMap::new();
        for bytes in [
            root_request,
            delegated,
            child_request,
            root_layer,
            child_layer,
            child_manifest_bytes.as_slice(),
        ] {
            blobs.insert(address_bytes(bytes).to_string(), bytes);
        }
        let blob_refs = blobs
            .iter()
            .map(|(kappa, bytes)| (kappa.as_bytes(), *bytes))
            .collect::<Vec<_>>();
        let bytes = finish_archive(&manifest, &blob_refs);
        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("child plan");
        registry.evaluate(&mut report);
        report.into_application_plan().expect("strict child plan")
    }

    fn plan_with_nested_siblings(registry: &ProviderRegistry) -> ApplicationPlan {
        let capabilities = test_capabilities();
        let capabilities_kappa = address_bytes(capabilities);
        let root_layer = b"root layer";
        let first_layer = b"first child layer";
        let grandchild_layer = b"grandchild layer";
        let second_layer = b"second child layer";
        let grandchild = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![wasm_layer(address_bytes(grandchild_layer), "grandchild")],
            children: Vec::new(),
        };
        let grandchild_bytes = grandchild.canonicalize();
        let first = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![wasm_layer(address_bytes(first_layer), "first-child")],
            children: vec![(address_bytes(&grandchild_bytes), capabilities_kappa)],
        };
        let first_bytes = first.canonicalize();
        let second = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![wasm_layer(address_bytes(second_layer), "second-child")],
            children: Vec::new(),
        };
        let second_bytes = second.canonicalize();
        let root = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![wasm_layer(address_bytes(root_layer), "root")],
            children: vec![
                (address_bytes(&first_bytes), capabilities_kappa),
                (address_bytes(&second_bytes), capabilities_kappa),
            ],
        };
        let mut blobs = std::collections::BTreeMap::new();
        for bytes in [
            capabilities,
            root_layer,
            first_layer,
            grandchild_layer,
            second_layer,
            first_bytes.as_slice(),
            grandchild_bytes.as_slice(),
            second_bytes.as_slice(),
        ] {
            blobs.insert(address_bytes(bytes).to_string(), bytes);
        }
        let blob_refs = blobs
            .iter()
            .map(|(kappa, bytes)| (kappa.as_bytes(), *bytes))
            .collect::<Vec<_>>();
        let bytes = finish_archive(&root, &blob_refs);
        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("nested plan");
        registry.evaluate(&mut report);
        report.into_application_plan().expect("strict nested plan")
    }

    fn network_capabilities() -> Vec<u8> {
        crate::holo_capability::compile_source(
            std::path::Path::new("network.json"),
            br#"{"schema_version":2,"network_fetch_endpoints":["https://api.example.com:443/v1"]}"#,
        )
        .expect("network capabilities")
    }

    fn network_grant() -> EffectiveGrant {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("grant.json");
        std::fs::write(
            &path,
            br#"{"schema_version":2,"network_fetch_endpoints":["https://api.example.com:443/v1"]}"#,
        )
        .expect("grant");
        EffectiveGrant::from_development_file(
            &path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("development grant")
    }

    #[test]
    fn component_contract_is_not_routed_to_the_core_wasm_provider() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events, None, None, None);
        let capabilities = test_capabilities();
        let payload = b"component payload";
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(payload),
                "hologram:application/run",
                crate::holo_contract::WASM_CONTRACT_COMPONENT_V1,
            )],
            children: Vec::new(),
        };
        let capabilities_kappa = address_bytes(capabilities);
        let payload_kappa = address_bytes(payload);
        let bytes = finish_archive(
            &manifest,
            &[
                (capabilities_kappa.as_bytes(), capabilities),
                (payload_kappa.as_bytes(), payload),
            ],
        );
        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("plan");

        registry.evaluate(&mut report);

        assert!(!report.runnable());
        assert!(matches!(
            &report.layers[0].provider,
            ProviderAvailability::Unavailable { reason }
                if reason.contains(crate::holo_contract::WASM_CONTRACT_COMPONENT_V1)
        ));
        let blocker = report
            .blockers
            .iter()
            .find(|blocker| {
                matches!(
                    blocker,
                    crate::application_plan::PlanBlocker::ProviderUnavailable { .. }
                )
            })
            .expect("provider blocker");
        assert_eq!(blocker.error_code(), "LIVE_CAPABILITY_MISSING");
    }

    #[tokio::test]
    async fn lifecycle_follows_manifest_order_and_stops_in_reverse() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events.clone(), None, None, None);
        let application = prepare_and_start(&plan(&registry), &registry)
            .await
            .expect("start");
        assert_eq!(application.state(), LifecycleState::Running);
        assert_eq!(application.status().resident_bytes, 20);
        application
            .invoke(vec![b"hello".to_vec()])
            .await
            .expect("invoke");
        application.stop().await.expect("stop");
        application.stop().await.expect("idempotent stop");
        assert_eq!(
            take(&events),
            [
                "prepare:0",
                "prepare:1",
                "start:0",
                "start:1",
                "invoke:1",
                "stop:1",
                "stop:0"
            ]
        );
    }

    #[tokio::test]
    async fn prepare_failure_rolls_back_prepared_layers() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events.clone(), Some(1), None, None);
        let error = prepare_and_start(&plan(&registry), &registry)
            .await
            .err()
            .expect("prepare failure");
        assert_eq!(error.code(), "LIVE_CONFLICT");
        assert_eq!(take(&events), ["prepare:0", "prepare:1", "stop:0"]);
    }

    #[tokio::test]
    async fn insufficient_grant_denies_before_provider_preparation() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events.clone(), None, None, None);
        let requested = crate::holo_capability::compile_source(
            std::path::Path::new("request.json"),
            br#"{"schema_version":2,"network_fetch_endpoints":["https://api.example.com:443/v1"]}"#,
        )
        .expect("network request");

        let error = prepare_and_start(&plan_with_capabilities(&registry, &requested), &registry)
            .await
            .err()
            .expect("baseline must deny network authority");

        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(take(&events).is_empty(), "provider prepare must not run");
    }

    #[tokio::test]
    async fn child_amplification_is_denied_before_provider_preparation() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events.clone(), None, None, None);
        let network = network_capabilities();
        let plan = plan_with_child(
            &registry,
            test_capabilities(),
            &network,
            test_capabilities(),
        );

        let mut decisions = Vec::new();
        let admission_error = plan
            .admitted_grants_with(&EffectiveGrant::local_baseline(), |decision| {
                decisions.push(decision);
            })
            .expect_err("empty parent grant cannot delegate network");
        assert_eq!(admission_error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].relation, "application_request");
        assert_eq!(decisions[0].outcome, "allowed");
        assert_eq!(decisions[1].relation, "child_delegation");
        assert_eq!(decisions[1].outcome, "denied");

        let error = prepare_and_start(&plan, &registry)
            .await
            .err()
            .expect("empty parent grant cannot delegate network");

        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error.to_string().contains("not admitted by parent grant"));
        assert!(take(&events).is_empty(), "provider prepare must not run");
    }

    #[tokio::test]
    async fn under_granted_child_request_is_denied_before_provider_preparation() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events.clone(), None, None, None);
        let network = network_capabilities();
        let plan = plan_with_child(
            &registry,
            test_capabilities(),
            test_capabilities(),
            &network,
        );

        let error = prepare_and_start_with_grant(&plan, &registry, &network_grant())
            .await
            .err()
            .expect("empty delegation cannot admit child network request");

        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error
            .to_string()
            .contains("not admitted by delegated grant"));
        assert!(take(&events).is_empty(), "provider prepare must not run");
    }

    #[tokio::test]
    async fn admitted_child_runs_under_attenuated_grant_and_root_remains_the_only_exit() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = named_registry(events.clone());
        let network = network_capabilities();
        let plan = plan_with_child(&registry, test_capabilities(), &network, &network);

        let application = prepare_and_start_with_grant(&plan, &registry, &network_grant())
            .await
            .expect("admitted child starts");
        assert_eq!(application.status().resident_bytes, 20);
        application
            .invoke(vec![b"hello".to_vec()])
            .await
            .expect("invoke root primary");
        application.stop().await.expect("stop tree");

        assert_eq!(
            take(&events),
            [
                "prepare:root:direct_development_file",
                "prepare:child:child_delegation",
                "start:root",
                "start:child",
                "invoke:root",
                "stop:child",
                "stop:root",
            ]
        );
    }

    #[tokio::test]
    async fn nested_children_follow_depth_first_manifest_order_and_reverse_stop_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = named_registry(events.clone());
        let plan = plan_with_nested_siblings(&registry);
        let expected_kappas = lifecycle_application_order(&plan)
            .expect("lifecycle order")
            .into_iter()
            .map(|application_index| {
                application_layers(&plan, application_index)
                    .expect("planned application")
                    .0
            })
            .collect::<Vec<_>>();
        let application = prepare_and_start(&plan, &registry)
            .await
            .expect("start nested tree");
        assert_eq!(application.status().resident_bytes, 40);
        assert_eq!(application.application_kappas(), expected_kappas);
        application.stop().await.expect("stop nested tree");

        assert_eq!(
            take(&events),
            [
                "prepare:root:local_baseline",
                "prepare:first-child:child_delegation",
                "prepare:grandchild:child_delegation",
                "prepare:second-child:child_delegation",
                "start:root",
                "start:first-child",
                "start:grandchild",
                "start:second-child",
                "stop:second-child",
                "stop:grandchild",
                "stop:first-child",
                "stop:root",
            ]
        );
    }

    #[tokio::test]
    async fn child_prepare_failure_rolls_back_the_prepared_parent() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = fallible_named_registry(events.clone(), Some("child"), None);
        let plan = plan_with_child(
            &registry,
            test_capabilities(),
            test_capabilities(),
            test_capabilities(),
        );

        let error = prepare_and_start(&plan, &registry)
            .await
            .err()
            .expect("child prepare failure");
        assert_eq!(error.code(), "LIVE_CONFLICT");
        assert_eq!(
            take(&events),
            [
                "prepare:root:local_baseline",
                "prepare:child:child_delegation",
                "stop:root",
            ]
        );
    }

    #[tokio::test]
    async fn child_start_failure_rolls_back_the_complete_prepared_tree_in_reverse() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = fallible_named_registry(events.clone(), None, Some("child"));
        let plan = plan_with_child(
            &registry,
            test_capabilities(),
            test_capabilities(),
            test_capabilities(),
        );

        let error = prepare_and_start(&plan, &registry)
            .await
            .err()
            .expect("child start failure");
        assert_eq!(error.code(), "LIVE_CONFLICT");
        assert_eq!(
            take(&events),
            [
                "prepare:root:local_baseline",
                "prepare:child:child_delegation",
                "start:root",
                "start:child",
                "stop:child",
                "stop:root",
            ]
        );
    }

    #[tokio::test]
    async fn sufficient_development_grant_reaches_provider_preparation() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events.clone(), Some(0), None, None);
        let requested = crate::holo_capability::compile_source(
            std::path::Path::new("request.json"),
            br#"{"schema_version":2,"network_fetch_endpoints":["https://api.example.com:443/v1"]}"#,
        )
        .expect("network request");
        let directory = tempfile::tempdir().expect("tempdir");
        let grant_path = directory.path().join("grant.json");
        std::fs::write(
            &grant_path,
            br#"{"schema_version":2,"network_fetch_endpoints":["https://api.example.com:443/v1"]}"#,
        )
        .expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("development grant");

        let error = prepare_and_start_with_grant(
            &plan_with_capabilities(&registry, &requested),
            &registry,
            &grant,
        )
        .await
        .err()
        .expect("synthetic provider fails after authorization");

        assert_eq!(error.code(), "LIVE_CONFLICT");
        assert_eq!(take(&events), ["prepare:0"]);
    }

    #[tokio::test]
    async fn start_failure_rolls_back_every_prepared_layer_in_reverse() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events.clone(), None, Some(1), None);
        let error = prepare_and_start(&plan(&registry), &registry)
            .await
            .err()
            .expect("start failure");
        assert_eq!(error.code(), "LIVE_CONFLICT");
        assert_eq!(
            take(&events),
            [
                "prepare:0",
                "prepare:1",
                "start:0",
                "start:1",
                "stop:1",
                "stop:0"
            ]
        );
    }

    #[tokio::test]
    async fn rollback_failures_are_diagnostics_on_the_original_error() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events, None, Some(1), Some(0));
        let error = prepare_and_start(&plan(&registry), &registry)
            .await
            .err()
            .expect("start failure");
        assert_eq!(error.code(), "LIVE_CONFLICT");
        assert!(error.to_string().contains("start failed at 1"), "{error}");
        assert!(error.to_string().contains("rollback failures"), "{error}");
        assert!(error.to_string().contains("stop failed at 0"), "{error}");
    }

    #[tokio::test]
    async fn normal_stop_continues_in_reverse_after_a_provider_failure() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = registry(events.clone(), None, None, Some(1));
        let application = prepare_and_start(&plan(&registry), &registry)
            .await
            .expect("start");

        let error = application.stop().await.expect_err("stop failure");

        assert_eq!(error.code(), "LIVE_CONFLICT");
        assert_eq!(application.state(), LifecycleState::Failed);
        assert_eq!(
            take(&events),
            [
                "prepare:0",
                "prepare:1",
                "start:0",
                "start:1",
                "stop:1",
                "stop:0"
            ]
        );
    }

    #[test]
    fn completion_roles_do_not_invent_exit_semantics_for_service_layers() {
        assert_eq!(
            layer_completion_role(LayerKind::WasmCodemodule),
            LayerCompletionRole::ExitBearing
        );
        assert_eq!(
            layer_completion_role(LayerKind::RootfsImage),
            LayerCompletionRole::ExitBearing
        );
        for kind in [
            LayerKind::TensorPlan,
            LayerKind::View,
            LayerKind::InferenceModel,
        ] {
            assert_eq!(
                layer_completion_role(kind),
                LayerCompletionRole::NonExitBearing
            );
        }
    }

    fn record(events: &Mutex<Vec<String>>, event: String) {
        events.lock().expect("events").push(event);
    }

    fn take(events: &Mutex<Vec<String>>) -> Vec<String> {
        std::mem::take(&mut *events.lock().expect("events"))
    }
}
