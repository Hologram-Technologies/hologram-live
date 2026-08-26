//! Bounded execution for the import-free Hologram Component Model v1 world.

use crate::application_plan::ProviderContext;
use crate::error::{LiveError, Result};
use crate::holo_provider::{
    LayerCompletion, LayerInvocation, LayerPrepareContext, LayerProvider, LayerRuntimeStatus,
    PreparedLayer, ProviderTarget,
};
use hologram::space::LayerKind;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

mod bindings {
    wasmtime::component::bindgen!({
        path: "specs/wit",
        world: "application",
    });
}

pub const COMPONENT_MEMORY_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const COMPONENT_FUEL_PER_INPUT: u64 = 100_000_000;
pub const COMPONENT_INPUT_MAX_BYTES: usize = 1024 * 1024;
pub const COMPONENT_OUTPUT_MAX_BYTES: usize = 1024 * 1024;
pub const COMPONENT_DEADLINE: Duration = Duration::from_secs(2);
pub use crate::holo_contract::COMPONENT_V1_ENTRY;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentLimits {
    pub memory_max_bytes: usize,
    pub fuel_per_input: u64,
    pub input_max_bytes: usize,
    pub output_max_bytes: usize,
    pub deadline: Duration,
}

impl Default for ComponentLimits {
    fn default() -> Self {
        Self {
            memory_max_bytes: COMPONENT_MEMORY_MAX_BYTES,
            fuel_per_input: COMPONENT_FUEL_PER_INPUT,
            input_max_bytes: COMPONENT_INPUT_MAX_BYTES,
            output_max_bytes: COMPONENT_OUTPUT_MAX_BYTES,
            deadline: COMPONENT_DEADLINE,
        }
    }
}

impl ComponentLimits {
    fn tighten(mut self, context: &LayerPrepareContext) -> Self {
        let request = &context.requested_capabilities.capabilities;
        let grant = &context.effective_grant.capabilities;
        self.memory_max_bytes = tighten_usize(
            self.memory_max_bytes,
            request.memory_max_bytes,
            grant.memory_max_bytes,
        );
        let cpu_ms = tighten_u64(
            self.deadline.as_millis().try_into().unwrap_or(u64::MAX),
            request.cpu_time_per_event_ms,
            grant.cpu_time_per_event_ms,
        );
        self.deadline = Duration::from_millis(cpu_ms.max(1));
        self
    }
}

fn tighten_usize(host: usize, requested: u64, granted: u64) -> usize {
    [requested, granted]
        .into_iter()
        .filter(|value| *value > 0)
        .fold(host, |limit, value| {
            limit.min(usize::try_from(value).unwrap_or(usize::MAX))
        })
}

fn tighten_u64(host: u64, requested: u64, granted: u64) -> u64 {
    [requested, granted]
        .into_iter()
        .filter(|value| *value > 0)
        .fold(host, u64::min)
}

pub struct ComponentProvider {
    target: ProviderTarget,
    limits: ComponentLimits,
}

impl ComponentProvider {
    pub fn direct() -> Self {
        Self::new(ProviderTarget::Direct)
    }

    pub fn resident() -> Self {
        Self::new(ProviderTarget::Resident)
    }

    fn new(target: ProviderTarget) -> Self {
        Self {
            target,
            limits: ComponentLimits::default(),
        }
    }

    #[cfg(test)]
    fn with_limits(target: ProviderTarget, limits: ComponentLimits) -> Self {
        Self { target, limits }
    }
}

#[tonic::async_trait]
impl LayerProvider for ComponentProvider {
    fn kind(&self) -> LayerKind {
        LayerKind::WasmCodemodule
    }

    fn contract(&self) -> Option<&'static str> {
        Some(crate::holo_contract::WASM_CONTRACT_COMPONENT_V1)
    }

    fn name(&self) -> &'static str {
        match self.target {
            ProviderTarget::Direct => "wasmtime-component-direct",
            ProviderTarget::Resident => "wasmtime-component-resident",
        }
    }

    fn availability(
        &self,
        context: &ProviderContext<'_>,
        target: ProviderTarget,
    ) -> std::result::Result<(), String> {
        if target != self.target {
            return Err(format!(
                "{} provider is configured for {}, not {}",
                self.name(),
                self.target.name(),
                target.name()
            ));
        }
        if context.entry != COMPONENT_V1_ENTRY {
            return Err(format!(
                "Component v1 entry must be {COMPONENT_V1_ENTRY:?}, got {:?}",
                context.entry
            ));
        }
        Ok(())
    }

    async fn prepare(&self, context: LayerPrepareContext) -> Result<Arc<dyn PreparedLayer>> {
        if context.target != self.target {
            return Err(LiveError::Conflict(format!(
                "provider {} cannot prepare a {} layer",
                self.name(),
                context.target.name()
            )));
        }
        if context.layer.entry != COMPONENT_V1_ENTRY {
            return Err(LiveError::Protocol(format!(
                "Component v1 entry must be {COMPONENT_V1_ENTRY:?}, got {:?}",
                context.layer.entry
            )));
        }
        let limits = self.limits.tighten(&context);
        let kappa = context.identity.archive_kappa;
        let position = context.layer.position;
        let payload = context.layer.content;
        let resident_bytes = payload.len();
        let compiled = tokio::task::spawn_blocking(move || {
            PreparedComponent::compile(&kappa, &payload, limits)
        })
        .await
        .map_err(|error| LiveError::Conflict(format!("join component prepare: {error}")))??;
        Ok(Arc::new(ComponentLayer {
            position,
            target: self.target,
            compiled: Arc::new(compiled),
            resident_bytes,
            running: AtomicBool::new(false),
            queued: AtomicUsize::new(0),
            processed: AtomicUsize::new(0),
        }))
    }
}

struct ComponentStore {
    limits: StoreLimits,
}

struct PreparedComponent {
    engine: Engine,
    component: Component,
    limits: ComponentLimits,
    serial: Mutex<()>,
}

impl PreparedComponent {
    fn compile(kappa: &str, bytes: &[u8], limits: ComponentLimits) -> Result<Self> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|error| {
            LiveError::Conflict(format!("configure Component v1 engine: {error}"))
        })?;
        let component = Component::new(&engine, bytes).map_err(|error| {
            LiveError::InvalidHolo(format!("compile Component v1 layer of {kappa}: {error}"))
        })?;

        // Instantiate once during preparation so a component with the wrong
        // imports or exported world fails before any application layer starts.
        instantiate(&engine, &component, limits)?;
        Ok(Self {
            engine,
            component,
            limits,
            serial: Mutex::new(()),
        })
    }

    fn run_inputs(&self, inputs: Vec<Vec<u8>>, cancelled: &AtomicBool) -> Result<Vec<Vec<u8>>> {
        let _serial = self
            .serial
            .lock()
            .map_err(|_| LiveError::Conflict("Component v1 execution lock poisoned".to_owned()))?;
        let mut outputs = Vec::with_capacity(inputs.len());
        for input in inputs {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            outputs.push(run_once(
                &self.engine,
                &self.component,
                self.limits,
                &input,
            )?);
        }
        Ok(outputs)
    }
}

fn new_store(engine: &Engine, limits: ComponentLimits) -> Result<Store<ComponentStore>> {
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.memory_max_bytes)
        .instances(32)
        .tables(32)
        .memories(32)
        .build();
    let mut store = Store::new(
        engine,
        ComponentStore {
            limits: store_limits,
        },
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(limits.fuel_per_input)
        .map_err(|error| LiveError::Conflict(format!("configure Component v1 fuel: {error}")))?;
    store.set_epoch_deadline(1);
    store.epoch_deadline_trap();
    Ok(store)
}

fn instantiate(engine: &Engine, component: &Component, limits: ComponentLimits) -> Result<()> {
    let linker = Linker::new(engine);
    let mut store = new_store(engine, limits)?;
    bindings::Application::instantiate(&mut store, component, &linker).map_err(|error| {
        LiveError::Protocol(format!(
            "Component v1 must export the import-free hologram:application/application@1.0.0 world: {error}"
        ))
    })?;
    Ok(())
}

fn run_once(
    engine: &Engine,
    component: &Component,
    limits: ComponentLimits,
    input: &[u8],
) -> Result<Vec<u8>> {
    if input.len() > limits.input_max_bytes {
        return Err(LiveError::Capability(format!(
            "Component v1 input is {} bytes; limit is {} bytes",
            input.len(),
            limits.input_max_bytes
        )));
    }
    let linker = Linker::new(engine);
    let mut store = new_store(engine, limits)?;
    let bindings = bindings::Application::instantiate(&mut store, component, &linker)
        .map_err(|error| LiveError::Protocol(format!("instantiate Component v1: {error}")))?;
    let result = bindings
        .hologram_application_guest()
        .call_run(&mut store, input)
        .map_err(|error| LiveError::Protocol(format!("execute Component v1 run: {error}")))?;
    let output = result.map_err(|error| {
        LiveError::Protocol(format!(
            "Component v1 guest returned {:?}: {}",
            error.code, error.message
        ))
    })?;
    if output.len() > limits.output_max_bytes {
        return Err(LiveError::Capability(format!(
            "Component v1 output is {} bytes; limit is {} bytes",
            output.len(),
            limits.output_max_bytes
        )));
    }
    Ok(output)
}

struct CancellationGuard {
    engine: Engine,
    cancelled: Arc<AtomicBool>,
    armed: bool,
}

impl CancellationGuard {
    fn new(engine: Engine, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            engine,
            cancelled,
            armed: true,
        }
    }

    fn cancel(&mut self) {
        if self.armed {
            self.cancelled.store(true, Ordering::Release);
            self.engine.increment_epoch();
            self.armed = false;
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct ComponentLayer {
    position: u32,
    target: ProviderTarget,
    compiled: Arc<PreparedComponent>,
    resident_bytes: usize,
    running: AtomicBool,
    queued: AtomicUsize,
    processed: AtomicUsize,
}

struct QueueGuard<'a>(&'a AtomicUsize);

impl Drop for QueueGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[tonic::async_trait]
impl PreparedLayer for ComponentLayer {
    fn position(&self) -> u32 {
        self.position
    }

    async fn start(&self) -> Result<()> {
        self.running.store(true, Ordering::Release);
        Ok(())
    }

    async fn invoke(&self, inputs: Vec<Vec<u8>>) -> Result<LayerInvocation> {
        if !self.running.load(Ordering::Acquire) {
            return Err(LiveError::Conflict(format!(
                "{} Component v1 layer {} is not running",
                self.target.name(),
                self.position
            )));
        }
        for input in &inputs {
            if input.len() > self.compiled.limits.input_max_bytes {
                return Err(LiveError::Capability(format!(
                    "Component v1 input is {} bytes; limit is {} bytes",
                    input.len(),
                    self.compiled.limits.input_max_bytes
                )));
            }
        }

        let started = Instant::now();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut guard = CancellationGuard::new(self.compiled.engine.clone(), cancelled.clone());
        let compiled = self.compiled.clone();
        self.queued.fetch_add(1, Ordering::Relaxed);
        let _queued = QueueGuard(&self.queued);
        let mut task = tokio::task::spawn_blocking(move || compiled.run_inputs(inputs, &cancelled));
        let result = if let Ok(joined) =
            tokio::time::timeout(self.compiled.limits.deadline, &mut task).await
        {
            guard.disarm();
            joined.map_err(|error| {
                LiveError::Conflict(format!("join Component v1 invocation: {error}"))
            })?
        } else {
            guard.cancel();
            let _ = task.await;
            Err(LiveError::Capability(format!(
                "Component v1 invocation exceeded the {} ms deadline",
                self.compiled.limits.deadline.as_millis()
            )))
        };
        let outputs = result?;
        self.processed.fetch_add(1, Ordering::Relaxed);
        Ok(LayerInvocation {
            outputs,
            completion: LayerCompletion::Returned,
            elapsed_micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        })
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Release);
        self.compiled.engine.increment_epoch();
        Ok(())
    }

    fn status(&self) -> LayerRuntimeStatus {
        LayerRuntimeStatus {
            resident_bytes: self.resident_bytes,
            queued: self.queued.load(Ordering::Relaxed),
            processed: self.processed.load(Ordering::Relaxed),
        }
    }
}

fn cancelled_error() -> LiveError {
    LiveError::Capability("Component v1 invocation was cancelled".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_wat() -> &'static str {
        include_str!("../tests/fixtures/component-echo/echo.wat")
    }

    fn spinning_wat() -> String {
        let marker = "    (func $hologram:application/guest@1.0.0#run (;3;)";
        let source = echo_wat();
        let start = source.find(marker).expect("fixture run export");
        let end = source[start + marker.len()..]
            .find("\n    (func ")
            .map(|offset| start + marker.len() + offset)
            .expect("function after fixture run export");
        let spin = concat!(
            "    (func $hologram:application/guest@1.0.0#run (;3;) ",
            "(type 2) (param i32 i32) (result i32)\n",
            "      (loop $spin\n",
            "        br $spin\n",
            "      )\n",
            "      unreachable\n",
            "    )"
        );
        format!("{}{}{}", &source[..start], spin, &source[end..])
    }

    #[test]
    fn zero_capability_scalars_do_not_remove_host_ceilings() {
        assert_eq!(tighten_usize(64, 0, 0), 64);
        assert_eq!(tighten_u64(2_000, 0, 0), 2_000);
    }

    #[test]
    fn nonzero_capability_scalars_only_tighten_host_ceilings() {
        assert_eq!(tighten_usize(64, 32, 48), 32);
        assert_eq!(tighten_usize(64, 128, 96), 64);
        assert_eq!(tighten_u64(2_000, 1_500, 1_800), 1_500);
    }

    #[test]
    fn providers_have_exact_target_and_contract() {
        let direct =
            ComponentProvider::with_limits(ProviderTarget::Direct, ComponentLimits::default());
        assert_eq!(direct.target, ProviderTarget::Direct);
        assert_eq!(
            direct.contract(),
            Some(crate::holo_contract::WASM_CONTRACT_COMPONENT_V1)
        );
    }

    #[test]
    fn echo_component_runs_with_a_fresh_store_per_input() {
        let component = PreparedComponent::compile(
            "fixture",
            echo_wat().as_bytes(),
            ComponentLimits::default(),
        )
        .expect("compile echo component");
        let cancelled = AtomicBool::new(false);
        let outputs = component
            .run_inputs(vec![b"one".to_vec(), b"two".to_vec()], &cancelled)
            .expect("run echo component");
        assert_eq!(outputs, vec![b"one".to_vec(), b"two".to_vec()]);
    }

    #[test]
    fn guest_declared_error_is_typed_and_redacted_to_its_public_fields() {
        let component = PreparedComponent::compile(
            "fixture",
            echo_wat().as_bytes(),
            ComponentLimits::default(),
        )
        .expect("compile echo component");
        let error = component
            .run_inputs(vec![b"guest-error".to_vec()], &AtomicBool::new(false))
            .expect_err("guest error");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR", "{error}");
        assert!(error.to_string().contains("Failed"));
        assert!(error.to_string().contains("fixture failure"));
        assert!(!error.to_string().contains("guest-error"));
    }

    #[test]
    fn undeclared_component_import_is_rejected_during_preparation() {
        let imported = br#"(component
          (type $forbidden-type (func))
          (import "forbidden" (func $forbidden (type $forbidden-type)))
        )"#;
        let error = PreparedComponent::compile("fixture", imported, ComponentLimits::default())
            .err()
            .expect("undeclared import");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR", "{error}");
        assert!(error.to_string().contains("forbidden"));
    }

    #[test]
    fn input_and_output_limits_fail_closed() {
        let mut limits = ComponentLimits {
            input_max_bytes: 2,
            ..ComponentLimits::default()
        };
        let component = PreparedComponent::compile("fixture", echo_wat().as_bytes(), limits)
            .expect("compile echo component");
        let error = component
            .run_inputs(vec![b"three".to_vec()], &AtomicBool::new(false))
            .expect_err("oversized input");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");

        limits.input_max_bytes = 16;
        limits.output_max_bytes = 2;
        let component = PreparedComponent::compile("fixture", echo_wat().as_bytes(), limits)
            .expect("compile echo component");
        let error = component
            .run_inputs(vec![b"three".to_vec()], &AtomicBool::new(false))
            .expect_err("oversized output");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
    }

    #[test]
    fn memory_and_fuel_limits_fail_closed() {
        let mut limits = ComponentLimits {
            memory_max_bytes: 512 * 1024,
            ..ComponentLimits::default()
        };
        let error = PreparedComponent::compile("fixture", echo_wat().as_bytes(), limits)
            .err()
            .expect("fixture requires more memory");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");

        limits = ComponentLimits::default();
        let mut component = PreparedComponent::compile("fixture", echo_wat().as_bytes(), limits)
            .expect("compile echo component");
        component.limits.fuel_per_input = 10;
        let error = component
            .run_inputs(vec![b"fuel".to_vec()], &AtomicBool::new(false))
            .expect_err("fuel exhaustion");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
    }

    #[tokio::test]
    async fn deadline_interrupts_only_its_component_engine() {
        let limits = ComponentLimits {
            fuel_per_input: u64::MAX,
            deadline: Duration::from_millis(10),
            ..ComponentLimits::default()
        };
        let compiled = Arc::new(
            PreparedComponent::compile("fixture", spinning_wat().as_bytes(), limits)
                .expect("compile spinning component"),
        );
        let layer = ComponentLayer {
            position: 0,
            target: ProviderTarget::Direct,
            compiled,
            resident_bytes: 1,
            running: AtomicBool::new(true),
            queued: AtomicUsize::new(0),
            processed: AtomicUsize::new(0),
        };
        let error = layer
            .invoke(vec![b"deadline".to_vec()])
            .await
            .expect_err("deadline");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("deadline"));

        let isolated = PreparedComponent::compile(
            "isolated",
            echo_wat().as_bytes(),
            ComponentLimits::default(),
        )
        .expect("compile isolated component");
        assert_eq!(
            isolated
                .run_inputs(vec![b"still running".to_vec()], &AtomicBool::new(false))
                .expect("separate component engine remains live"),
            vec![b"still running".to_vec()]
        );
    }

    #[tokio::test]
    async fn dropping_invocation_interrupts_its_guest() {
        let limits = ComponentLimits {
            fuel_per_input: u64::MAX,
            deadline: Duration::from_secs(10),
            ..ComponentLimits::default()
        };
        let compiled = Arc::new(
            PreparedComponent::compile("fixture", spinning_wat().as_bytes(), limits)
                .expect("compile spinning component"),
        );
        let layer = Arc::new(ComponentLayer {
            position: 0,
            target: ProviderTarget::Resident,
            compiled: compiled.clone(),
            resident_bytes: 1,
            running: AtomicBool::new(true),
            queued: AtomicUsize::new(0),
            processed: AtomicUsize::new(0),
        });
        let task_layer = layer.clone();
        let task = tokio::spawn(async move { task_layer.invoke(vec![b"cancel".to_vec()]).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        task.abort();
        let _ = task.await;

        assert_eq!(layer.queued.load(Ordering::Relaxed), 0);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if compiled.serial.try_lock().is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled synchronous guest releases serialization boundary");
    }
}
