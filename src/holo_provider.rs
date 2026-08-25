//! Provider selection and transactional lifecycle for resolved `.holo` applications.

use crate::application_plan::{
    layer_kind_name, ApplicationPlan, ApplicationPlanReport, HoloIdentity, ProviderAvailability,
    ProviderContext, ResolvedLayer,
};
use crate::error::{LiveError, Result};
use hologram::space::LayerKind;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

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
    pub required_capabilities: Arc<[u8]>,
    pub layer: ResolvedLayer,
    pub target: ProviderTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerInvocation {
    pub outputs: Vec<Vec<u8>>,
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
            if providers[..index]
                .iter()
                .any(|existing| existing.kind() == provider.kind())
            {
                return Err(LiveError::Config(format!(
                    "duplicate {} provider for {} layers",
                    target.name(),
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
        let Some(provider) = self.select(context.kind) else {
            let reason = if context.kind == LayerKind::InferenceModel {
                format!(
                    "inference provider for service {} ({}) is not connected to {} execution",
                    context.entry,
                    context.aux,
                    self.target.name()
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
        };
        match provider.availability(context, self.target) {
            Ok(()) => ProviderAvailability::Available {
                provider: provider.name().to_owned(),
            },
            Err(reason) => ProviderAvailability::Unavailable { reason },
        }
    }

    fn select(&self, kind: LayerKind) -> Option<&Arc<dyn LayerProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.kind() == kind)
    }
}

pub struct RunningApplication {
    identity: HoloIdentity,
    primary_layer: u32,
    layers: Vec<Arc<dyn PreparedLayer>>,
    state: AtomicU8,
}

impl RunningApplication {
    pub fn state(&self) -> LifecycleState {
        LifecycleState::from_value(self.state.load(Ordering::Acquire))
    }

    pub fn identity(&self) -> &HoloIdentity {
        &self.identity
    }

    pub fn status(&self) -> LayerRuntimeStatus {
        self.layers.iter().map(|layer| layer.status()).fold(
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
            .find(|layer| layer.position() == self.primary_layer)
            .ok_or_else(|| {
                LiveError::Conflict(format!(
                    "running application {} lost primary layer {}",
                    self.identity.application_kappa, self.primary_layer
                ))
            })?;
        layer.invoke(inputs).await
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
    let primary_layer = plan.primary_layer.ok_or_else(|| {
        LiveError::Capability(format!(
            "application {} has no primary exit-bearing layer",
            plan.identity.application_kappa
        ))
    })?;
    let state = AtomicU8::new(LifecycleState::Preparing.value());
    let mut prepared = Vec::with_capacity(plan.layers.len());
    for layer in &plan.layers {
        let provider = registry.select(layer.kind).ok_or_else(|| {
            LiveError::Capability(format!(
                "application {} layer {} has no {} provider for {}",
                plan.identity.application_kappa,
                layer.position,
                registry.target.name(),
                layer_kind_name(layer.kind)
            ))
        })?;
        if layer.provider != provider.name() {
            return Err(LiveError::Conflict(format!(
                "application {} layer {} selected provider {}, but registry resolved {}",
                plan.identity.application_kappa,
                layer.position,
                layer.provider,
                provider.name()
            )));
        }
        tracing::info!(
            application_kappa = %plan.identity.application_kappa,
            archive_kappa = %plan.identity.archive_kappa,
            layer_position = layer.position,
            provider = provider.name(),
            lifecycle_phase = "prepare",
            "preparing holo layer"
        );
        match provider
            .prepare(LayerPrepareContext {
                identity: plan.identity.clone(),
                required_capabilities: plan.required_capabilities.clone(),
                layer: layer.clone(),
                target: registry.target,
            })
            .await
        {
            Ok(instance) => prepared.push(instance),
            Err(error) => {
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
            application_kappa = %plan.identity.application_kappa,
            archive_kappa = %plan.identity.archive_kappa,
            layer_position = layer.position(),
            lifecycle_phase = "start",
            "starting holo layer"
        );
        if let Err(error) = layer.start().await {
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
    })
}

async fn stop_reverse(
    identity: &HoloIdentity,
    layers: &[Arc<dyn PreparedLayer>],
    phase: &'static str,
) -> Vec<String> {
    let mut failures = Vec::new();
    for layer in layers.iter().rev() {
        tracing::info!(
            application_kappa = %identity.application_kappa,
            archive_kappa = %identity.archive_kappa,
            layer_position = layer.position(),
            lifecycle_phase = phase,
            "stopping holo layer"
        );
        if let Err(error) = layer.stop().await {
            failures.push(format!("layer {}: {error}", layer.position()));
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
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    struct SyntheticProvider {
        events: Arc<Mutex<Vec<String>>>,
        fail_prepare: Option<u32>,
        fail_start: Option<u32>,
        fail_stop: Option<u32>,
    }

    #[tonic::async_trait]
    impl LayerProvider for SyntheticProvider {
        fn kind(&self) -> LayerKind {
            LayerKind::WasmCodemodule
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

    fn plan(registry: &ProviderRegistry) -> ApplicationPlan {
        let capabilities = b"capabilities";
        let first = b"first wasm";
        let second = b"second wasm";
        let manifest = AppManifest {
            primary: Some(1),
            requires: address_bytes(capabilities),
            layers: vec![
                Layer::wasm(address_bytes(first), "first"),
                Layer::wasm(address_bytes(second), "second"),
            ],
            children: Vec::new(),
        };
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(capabilities).as_bytes(), capabilities);
        writer.add_content_blob(address_bytes(first).as_bytes(), first);
        writer.add_content_blob(address_bytes(second).as_bytes(), second);
        let bytes = writer.finish().expect("archive");
        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("plan report");
        registry.evaluate(&mut report);
        report.into_application_plan().expect("strict plan")
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

    fn record(events: &Mutex<Vec<String>>, event: String) {
        events.lock().expect("events").push(event);
    }

    fn take(events: &Mutex<Vec<String>>) -> Vec<String> {
        std::mem::take(&mut *events.lock().expect("events"))
    }
}
