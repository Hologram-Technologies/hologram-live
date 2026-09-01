//! Bounded execution for Hologram Component Model v1 contract profiles.

use crate::application_plan::ProviderContext;
use crate::error::{LiveError, Result};
use crate::holo_channel::{ChannelBroker, ChannelError};
use crate::holo_graph::{resolve_storage_graph, StorageGraphLimits};
use crate::holo_provider::{
    LayerCompletion, LayerInvocation, LayerPrepareContext, LayerProvider, LayerRuntimeStatus,
    PreparedLayer, ProviderTarget,
};
use crate::store::ObjectStore;
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

mod store_read_bindings {
    wasmtime::component::bindgen!({
        path: "specs/wit/store-read",
        world: "application",
    });
}

mod store_write_bindings {
    wasmtime::component::bindgen!({
        path: "specs/wit/store-write",
        world: "application",
    });
}

mod channel_publish_bindings {
    wasmtime::component::bindgen!({
        path: "specs/wit/channel-publish",
        world: "application",
    });
}

mod channel_subscribe_bindings {
    wasmtime::component::bindgen!({
        path: "specs/wit/channel-subscribe",
        world: "application",
    });
}

pub const COMPONENT_MEMORY_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const COMPONENT_INSTANCE_MAX: usize = 128;
pub const COMPONENT_TABLE_MAX: usize = 32;
pub const COMPONENT_MEMORY_COUNT_MAX: usize = 32;
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
    profile: ComponentProfile,
    object_store: Option<Arc<ObjectStore>>,
    channel_broker: Option<Arc<ChannelBroker>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComponentProfile {
    ImportFree,
    StoreRead,
    StoreGraphRead,
    StoreWrite,
    ChannelPublish,
    ChannelSubscribe,
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
            profile: ComponentProfile::ImportFree,
            object_store: None,
            channel_broker: None,
        }
    }

    pub fn store_read_direct(object_store: Option<Arc<ObjectStore>>) -> Self {
        Self::store_read(ProviderTarget::Direct, object_store)
    }

    pub fn store_read_resident(object_store: Option<Arc<ObjectStore>>) -> Self {
        Self::store_read(ProviderTarget::Resident, object_store)
    }

    fn store_read(target: ProviderTarget, object_store: Option<Arc<ObjectStore>>) -> Self {
        Self {
            target,
            limits: ComponentLimits::default(),
            profile: ComponentProfile::StoreRead,
            object_store,
            channel_broker: None,
        }
    }

    pub fn store_graph_read_direct(object_store: Option<Arc<ObjectStore>>) -> Self {
        Self::store_graph_read(ProviderTarget::Direct, object_store)
    }

    pub fn store_graph_read_resident(object_store: Option<Arc<ObjectStore>>) -> Self {
        Self::store_graph_read(ProviderTarget::Resident, object_store)
    }

    fn store_graph_read(target: ProviderTarget, object_store: Option<Arc<ObjectStore>>) -> Self {
        Self {
            target,
            limits: ComponentLimits::default(),
            profile: ComponentProfile::StoreGraphRead,
            object_store,
            channel_broker: None,
        }
    }

    pub fn store_write_direct(object_store: Option<Arc<ObjectStore>>) -> Self {
        Self::store_write(ProviderTarget::Direct, object_store)
    }

    pub fn store_write_resident(object_store: Option<Arc<ObjectStore>>) -> Self {
        Self::store_write(ProviderTarget::Resident, object_store)
    }

    fn store_write(target: ProviderTarget, object_store: Option<Arc<ObjectStore>>) -> Self {
        Self {
            target,
            limits: ComponentLimits::default(),
            profile: ComponentProfile::StoreWrite,
            object_store,
            channel_broker: None,
        }
    }

    pub fn channel_publish_direct(channel_broker: Arc<ChannelBroker>) -> Self {
        Self::channel(
            ProviderTarget::Direct,
            ComponentProfile::ChannelPublish,
            channel_broker,
        )
    }

    pub fn channel_publish_resident(channel_broker: Arc<ChannelBroker>) -> Self {
        Self::channel(
            ProviderTarget::Resident,
            ComponentProfile::ChannelPublish,
            channel_broker,
        )
    }

    pub fn channel_subscribe_direct(channel_broker: Arc<ChannelBroker>) -> Self {
        Self::channel(
            ProviderTarget::Direct,
            ComponentProfile::ChannelSubscribe,
            channel_broker,
        )
    }

    pub fn channel_subscribe_resident(channel_broker: Arc<ChannelBroker>) -> Self {
        Self::channel(
            ProviderTarget::Resident,
            ComponentProfile::ChannelSubscribe,
            channel_broker,
        )
    }

    fn channel(
        target: ProviderTarget,
        profile: ComponentProfile,
        channel_broker: Arc<ChannelBroker>,
    ) -> Self {
        Self {
            target,
            limits: ComponentLimits::default(),
            profile,
            object_store: None,
            channel_broker: Some(channel_broker),
        }
    }

    #[cfg(test)]
    fn with_limits(target: ProviderTarget, limits: ComponentLimits) -> Self {
        Self {
            target,
            limits,
            profile: ComponentProfile::ImportFree,
            object_store: None,
            channel_broker: None,
        }
    }
}

#[tonic::async_trait]
impl LayerProvider for ComponentProvider {
    fn kind(&self) -> LayerKind {
        LayerKind::WasmCodemodule
    }

    fn contract(&self) -> Option<&'static str> {
        Some(match self.profile {
            ComponentProfile::ImportFree => crate::holo_contract::WASM_CONTRACT_COMPONENT_V1,
            ComponentProfile::StoreRead => {
                crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_READ_V1
            }
            ComponentProfile::StoreGraphRead => {
                crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1
            }
            ComponentProfile::StoreWrite => {
                crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_WRITE_V1
            }
            ComponentProfile::ChannelPublish => {
                crate::holo_contract::WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1
            }
            ComponentProfile::ChannelSubscribe => {
                crate::holo_contract::WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1
            }
        })
    }

    fn name(&self) -> &'static str {
        match (self.target, self.profile) {
            (ProviderTarget::Direct, ComponentProfile::ImportFree) => "wasmtime-component-direct",
            (ProviderTarget::Resident, ComponentProfile::ImportFree) => {
                "wasmtime-component-resident"
            }
            (ProviderTarget::Direct, ComponentProfile::StoreRead) => {
                "wasmtime-component-store-read-direct"
            }
            (ProviderTarget::Resident, ComponentProfile::StoreRead) => {
                "wasmtime-component-store-read-resident"
            }
            (ProviderTarget::Direct, ComponentProfile::StoreGraphRead) => {
                "wasmtime-component-store-graph-read-direct"
            }
            (ProviderTarget::Resident, ComponentProfile::StoreGraphRead) => {
                "wasmtime-component-store-graph-read-resident"
            }
            (ProviderTarget::Direct, ComponentProfile::StoreWrite) => {
                "wasmtime-component-store-write-direct"
            }
            (ProviderTarget::Resident, ComponentProfile::StoreWrite) => {
                "wasmtime-component-store-write-resident"
            }
            (ProviderTarget::Direct, ComponentProfile::ChannelPublish) => {
                "wasmtime-component-channel-publish-direct"
            }
            (ProviderTarget::Resident, ComponentProfile::ChannelPublish) => {
                "wasmtime-component-channel-publish-resident"
            }
            (ProviderTarget::Direct, ComponentProfile::ChannelSubscribe) => {
                "wasmtime-component-channel-subscribe-direct"
            }
            (ProviderTarget::Resident, ComponentProfile::ChannelSubscribe) => {
                "wasmtime-component-channel-subscribe-resident"
            }
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
        let host = match self.profile {
            ComponentProfile::ImportFree => ComponentHost::ImportFree,
            ComponentProfile::StoreRead => {
                let roots = context
                    .requested_capabilities
                    .capabilities
                    .storage_roots
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                if roots.is_empty() {
                    return Err(LiveError::Authorization(format!(
                        "Component interface hologram:host/store@1.0.0 requires at least one admitted storage_roots entry for application {}",
                        context.identity.application_kappa
                    )));
                }
                let object_store = self.object_store.clone().ok_or_else(|| {
                    LiveError::Capability(
                        "Component interface hologram:host/store@1.0.0 has no object-store backend"
                            .to_owned(),
                    )
                })?;
                ComponentHost::StoreRead(StoreReadHost {
                    object_store,
                    roots: Arc::from(roots),
                })
            }
            ComponentProfile::StoreGraphRead => {
                let roots = context
                    .requested_capabilities
                    .capabilities
                    .storage_roots
                    .clone();
                if roots.is_empty() {
                    return Err(LiveError::Authorization(format!(
                        "Component graph-read interface hologram:host/store@1.0.0 requires at least one admitted storage_roots entry for application {}",
                        context.identity.application_kappa
                    )));
                }
                let object_store = self.object_store.clone().ok_or_else(|| {
                    LiveError::Capability(
                        "Component graph-read interface has no object-store backend".to_owned(),
                    )
                })?;
                let resolver_store = object_store.clone();
                let closure = tokio::task::spawn_blocking(move || {
                    resolve_storage_graph(&resolver_store, &roots, StorageGraphLimits::default())
                })
                .await
                .map_err(|error| {
                    LiveError::Conflict(format!("join typed storage graph resolution: {error}"))
                })??;
                ComponentHost::StoreRead(StoreReadHost {
                    object_store,
                    roots: Arc::from(closure.readable),
                })
            }
            ComponentProfile::StoreWrite => {
                let request = &context.requested_capabilities.capabilities;
                let roots = request
                    .storage_roots
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                if roots.is_empty() {
                    return Err(LiveError::Authorization(format!(
                        "Component interface hologram:host/store-write@1.0.0 requires at least one admitted storage_roots entry for application {}",
                        context.identity.application_kappa
                    )));
                }
                if request.storage_quota_bytes == 0 {
                    return Err(LiveError::Authorization(format!(
                        "Component interface hologram:host/store-write@1.0.0 requires a nonzero admitted storage_quota_bytes value for application {}",
                        context.identity.application_kappa
                    )));
                }
                let object_store = self.object_store.clone().ok_or_else(|| {
                    LiveError::Capability(
                        "Component interface hologram:host/store-write@1.0.0 has no object-store backend"
                            .to_owned(),
                    )
                })?;
                ComponentHost::StoreWrite(StoreWriteHost {
                    object_store,
                    roots: Arc::from(roots),
                    quota_remaining: Arc::new(Mutex::new(request.storage_quota_bytes)),
                })
            }
            ComponentProfile::ChannelPublish => {
                let channels = context
                    .requested_capabilities
                    .capabilities
                    .publish_channels
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                if channels.is_empty() {
                    return Err(LiveError::Authorization(format!(
                        "Component interface hologram:host/channel-publish@1.0.0 requires at least one admitted publish_channels entry for application {}",
                        context.identity.application_kappa
                    )));
                }
                ComponentHost::ChannelPublish(ChannelHost {
                    broker: self.channel_broker.clone().ok_or_else(|| {
                        LiveError::Capability(
                            "Component channel-publish interface has no broker backend".to_owned(),
                        )
                    })?,
                    channels: Arc::from(channels),
                })
            }
            ComponentProfile::ChannelSubscribe => {
                let channels = context
                    .requested_capabilities
                    .capabilities
                    .subscribe_channels
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                if channels.is_empty() {
                    return Err(LiveError::Authorization(format!(
                        "Component interface hologram:host/channel-subscribe@1.0.0 requires at least one admitted subscribe_channels entry for application {}",
                        context.identity.application_kappa
                    )));
                }
                ComponentHost::ChannelSubscribe(ChannelHost {
                    broker: self.channel_broker.clone().ok_or_else(|| {
                        LiveError::Capability(
                            "Component channel-subscribe interface has no broker backend"
                                .to_owned(),
                        )
                    })?,
                    channels: Arc::from(channels),
                })
            }
        };
        let kappa = context.identity.archive_kappa;
        let position = context.layer.position;
        let payload = context.layer.content;
        let resident_bytes = payload.len();
        let compiled = tokio::task::spawn_blocking(move || {
            PreparedComponent::compile(&kappa, &payload, limits, host)
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
    host: ComponentHost,
}

#[derive(Clone)]
enum ComponentHost {
    ImportFree,
    StoreRead(StoreReadHost),
    StoreWrite(StoreWriteHost),
    ChannelPublish(ChannelHost),
    ChannelSubscribe(ChannelHost),
}

#[derive(Clone)]
struct StoreReadHost {
    object_store: Arc<ObjectStore>,
    roots: Arc<[String]>,
}

#[derive(Clone)]
struct StoreWriteHost {
    object_store: Arc<ObjectStore>,
    roots: Arc<[String]>,
    quota_remaining: Arc<Mutex<u64>>,
}

#[derive(Clone)]
struct ChannelHost {
    broker: Arc<ChannelBroker>,
    channels: Arc<[String]>,
}

impl store_read_bindings::hologram::host::store::Host for ComponentStore {
    fn read(&mut self, kappa: String) -> std::result::Result<Vec<u8>, String> {
        let ComponentHost::StoreRead(host) = &self.host else {
            return Err("store.read is not linked for this contract".to_owned());
        };
        if !host.roots.iter().any(|root| root == &kappa) {
            return Err("object is outside the application's admitted storage roots".to_owned());
        }
        host.object_store.get(&kappa).map_err(|error| match error {
            LiveError::NotFound(_) => "object was not found".to_owned(),
            _ => "object-store read failed".to_owned(),
        })
    }
}

impl store_write_bindings::hologram::host::store_write::Host for ComponentStore {
    fn write(&mut self, kappa: String, bytes: Vec<u8>) -> std::result::Result<(), String> {
        let ComponentHost::StoreWrite(host) = &mut self.host else {
            return Err("store.write is not linked for this contract".to_owned());
        };
        if !host.roots.iter().any(|root| root == &kappa) {
            return Err("object is outside the application's admitted storage roots".to_owned());
        }
        let mut quota_remaining = host
            .quota_remaining
            .lock()
            .map_err(|_| "store.write quota state is unavailable".to_owned())?;
        let created = host
            .object_store
            .cache_addressed_bounded(&kappa, &bytes, *quota_remaining)
            .map_err(|error| match error {
                LiveError::InvalidHolo(_) => {
                    "object bytes do not match the admitted content address".to_owned()
                }
                LiveError::Capability(_) => {
                    "write exceeds the application's admitted storage quota".to_owned()
                }
                _ => "object-store write failed".to_owned(),
            })?;
        if created {
            let byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            *quota_remaining -= byte_count;
        }
        Ok(())
    }
}

impl channel_publish_bindings::hologram::host::channel_publish::Host for ComponentStore {
    fn publish(&mut self, channel: String, message: Vec<u8>) -> std::result::Result<(), String> {
        let ComponentHost::ChannelPublish(host) = &self.host else {
            return Err("channel.publish is not linked for this contract".to_owned());
        };
        if !host.channels.iter().any(|allowed| allowed == &channel) {
            return Err("channel is outside the application's admitted publish set".to_owned());
        }
        host.broker
            .publish(&channel, message)
            .map_err(|error| match error {
                ChannelError::MessageTooLarge => {
                    "channel message exceeds the host limit".to_owned()
                }
                ChannelError::MailboxFull => "channel mailbox is full; retry later".to_owned(),
                ChannelError::StateUnavailable => "channel broker is unavailable".to_owned(),
            })
    }
}

impl channel_subscribe_bindings::hologram::host::channel_subscribe::Host for ComponentStore {
    fn try_receive(&mut self, channel: String) -> std::result::Result<Option<Vec<u8>>, String> {
        let ComponentHost::ChannelSubscribe(host) = &self.host else {
            return Err("channel.subscribe is not linked for this contract".to_owned());
        };
        if !host.channels.iter().any(|allowed| allowed == &channel) {
            return Err("channel is outside the application's admitted subscribe set".to_owned());
        }
        host.broker
            .try_receive(&channel)
            .map_err(|_| "channel broker is unavailable".to_owned())
    }
}

struct PreparedComponent {
    engine: Engine,
    component: Component,
    limits: ComponentLimits,
    host: ComponentHost,
    serial: Mutex<()>,
}

impl PreparedComponent {
    fn compile(
        kappa: &str,
        bytes: &[u8],
        limits: ComponentLimits,
        host: ComponentHost,
    ) -> Result<Self> {
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
        instantiate(&engine, &component, limits, &host)?;
        Ok(Self {
            engine,
            component,
            limits,
            host,
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
                &self.host,
            )?);
        }
        Ok(outputs)
    }
}

fn new_store(
    engine: &Engine,
    limits: ComponentLimits,
    host: &ComponentHost,
) -> Result<Store<ComponentStore>> {
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.memory_max_bytes)
        .instances(COMPONENT_INSTANCE_MAX)
        .tables(COMPONENT_TABLE_MAX)
        .memories(COMPONENT_MEMORY_COUNT_MAX)
        .build();
    let mut store = Store::new(
        engine,
        ComponentStore {
            limits: store_limits,
            host: host.clone(),
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

fn instantiate(
    engine: &Engine,
    component: &Component,
    limits: ComponentLimits,
    host: &ComponentHost,
) -> Result<()> {
    let mut linker = Linker::new(engine);
    let mut store = new_store(engine, limits, host)?;
    match host {
        ComponentHost::ImportFree => {
            bindings::Application::instantiate(&mut store, component, &linker).map_err(|error| {
                LiveError::Protocol(format!(
                    "Component v1 must export the import-free hologram:application/application@1.0.0 world: {error}"
                ))
            })?;
        }
        ComponentHost::StoreRead(_) => {
            store_read_bindings::hologram::host::store::add_to_linker::<
                _,
                wasmtime::component::HasSelf<ComponentStore>,
            >(&mut linker, |state| state)
            .map_err(|error| {
                LiveError::Conflict(format!("link Component store.read interface: {error}"))
            })?;
            store_read_bindings::Application::instantiate(&mut store, component, &linker)
                .map_err(|error| {
                    LiveError::Protocol(format!(
                        "Component store-read v1 must export hologram:application-store-read/application@1.0.0 and import only hologram:host/store@1.0.0: {error}"
                    ))
                })?;
        }
        ComponentHost::StoreWrite(_) => {
            store_write_bindings::hologram::host::store_write::add_to_linker::<
                _,
                wasmtime::component::HasSelf<ComponentStore>,
            >(&mut linker, |state| state)
            .map_err(|error| {
                LiveError::Conflict(format!("link Component store.write interface: {error}"))
            })?;
            store_write_bindings::Application::instantiate(&mut store, component, &linker)
                .map_err(|error| {
                    LiveError::Protocol(format!(
                        "Component store-write v1 must export hologram:application-store-write/application@1.0.0 and import only hologram:host/store-write@1.0.0: {error}"
                    ))
                })?;
        }
        ComponentHost::ChannelPublish(_) => {
            channel_publish_bindings::hologram::host::channel_publish::add_to_linker::<
                _,
                wasmtime::component::HasSelf<ComponentStore>,
            >(&mut linker, |state| state)
            .map_err(|error| {
                LiveError::Conflict(format!("link Component channel.publish interface: {error}"))
            })?;
            channel_publish_bindings::Application::instantiate(&mut store, component, &linker)
                .map_err(|error| {
                    LiveError::Protocol(format!(
                        "Component channel-publish v1 must export hologram:application-channel-publish/application@1.0.0 and import only hologram:host/channel-publish@1.0.0: {error}"
                    ))
                })?;
        }
        ComponentHost::ChannelSubscribe(_) => {
            channel_subscribe_bindings::hologram::host::channel_subscribe::add_to_linker::<
                _,
                wasmtime::component::HasSelf<ComponentStore>,
            >(&mut linker, |state| state)
            .map_err(|error| {
                LiveError::Conflict(format!(
                    "link Component channel.subscribe interface: {error}"
                ))
            })?;
            channel_subscribe_bindings::Application::instantiate(&mut store, component, &linker)
                .map_err(|error| {
                    LiveError::Protocol(format!(
                        "Component channel-subscribe v1 must export hologram:application-channel-subscribe/application@1.0.0 and import only hologram:host/channel-subscribe@1.0.0: {error}"
                    ))
                })?;
        }
    }
    Ok(())
}

fn run_once(
    engine: &Engine,
    component: &Component,
    limits: ComponentLimits,
    input: &[u8],
    host: &ComponentHost,
) -> Result<Vec<u8>> {
    if input.len() > limits.input_max_bytes {
        return Err(LiveError::Capability(format!(
            "Component v1 input is {} bytes; limit is {} bytes",
            input.len(),
            limits.input_max_bytes
        )));
    }
    let mut linker = Linker::new(engine);
    let mut store = new_store(engine, limits, host)?;
    let output = match host {
        ComponentHost::ImportFree => {
            let bindings = bindings::Application::instantiate(&mut store, component, &linker)
                .map_err(|error| {
                    LiveError::Protocol(format!("instantiate Component v1: {error}"))
                })?;
            let result = bindings
                .hologram_application_guest()
                .call_run(&mut store, input)
                .map_err(|error| {
                    LiveError::Protocol(format!("execute Component v1 run: {error}"))
                })?;
            result.map_err(|error| {
                LiveError::Protocol(format!(
                    "Component v1 guest returned {:?}: {}",
                    error.code, error.message
                ))
            })?
        }
        ComponentHost::StoreRead(_) => {
            store_read_bindings::hologram::host::store::add_to_linker::<
                _,
                wasmtime::component::HasSelf<ComponentStore>,
            >(&mut linker, |state| state)
            .map_err(|error| {
                LiveError::Conflict(format!("link Component store.read interface: {error}"))
            })?;
            let bindings =
                store_read_bindings::Application::instantiate(&mut store, component, &linker)
                    .map_err(|error| {
                        LiveError::Protocol(format!("instantiate Component store-read v1: {error}"))
                    })?;
            let result = bindings
                .hologram_application_store_read_guest()
                .call_run(&mut store, input)
                .map_err(|error| {
                    LiveError::Protocol(format!("execute Component store-read v1 run: {error}"))
                })?;
            result.map_err(|error| {
                LiveError::Protocol(format!(
                    "Component store-read v1 guest returned {:?}: {}",
                    error.code, error.message
                ))
            })?
        }
        ComponentHost::StoreWrite(_) => {
            store_write_bindings::hologram::host::store_write::add_to_linker::<
                _,
                wasmtime::component::HasSelf<ComponentStore>,
            >(&mut linker, |state| state)
            .map_err(|error| {
                LiveError::Conflict(format!("link Component store.write interface: {error}"))
            })?;
            let bindings =
                store_write_bindings::Application::instantiate(&mut store, component, &linker)
                    .map_err(|error| {
                        LiveError::Protocol(format!(
                            "instantiate Component store-write v1: {error}"
                        ))
                    })?;
            let result = bindings
                .hologram_application_store_write_guest()
                .call_run(&mut store, input)
                .map_err(|error| {
                    LiveError::Protocol(format!("execute Component store-write v1 run: {error}"))
                })?;
            result.map_err(|error| {
                LiveError::Protocol(format!(
                    "Component store-write v1 guest returned {:?}: {}",
                    error.code, error.message
                ))
            })?
        }
        ComponentHost::ChannelPublish(_) => {
            channel_publish_bindings::hologram::host::channel_publish::add_to_linker::<
                _,
                wasmtime::component::HasSelf<ComponentStore>,
            >(&mut linker, |state| state)
            .map_err(|error| {
                LiveError::Conflict(format!("link Component channel.publish interface: {error}"))
            })?;
            let bindings =
                channel_publish_bindings::Application::instantiate(&mut store, component, &linker)
                    .map_err(|error| {
                        LiveError::Protocol(format!(
                            "instantiate Component channel-publish v1: {error}"
                        ))
                    })?;
            let result = bindings
                .hologram_application_channel_publish_guest()
                .call_run(&mut store, input)
                .map_err(|error| {
                    LiveError::Protocol(format!(
                        "execute Component channel-publish v1 run: {error}"
                    ))
                })?;
            result.map_err(|error| {
                LiveError::Protocol(format!(
                    "Component channel-publish v1 guest returned {:?}: {}",
                    error.code, error.message
                ))
            })?
        }
        ComponentHost::ChannelSubscribe(_) => {
            channel_subscribe_bindings::hologram::host::channel_subscribe::add_to_linker::<
                _,
                wasmtime::component::HasSelf<ComponentStore>,
            >(&mut linker, |state| state)
            .map_err(|error| {
                LiveError::Conflict(format!(
                    "link Component channel.subscribe interface: {error}"
                ))
            })?;
            let bindings = channel_subscribe_bindings::Application::instantiate(
                &mut store, component, &linker,
            )
            .map_err(|error| {
                LiveError::Protocol(format!(
                    "instantiate Component channel-subscribe v1: {error}"
                ))
            })?;
            let result = bindings
                .hologram_application_channel_subscribe_guest()
                .call_run(&mut store, input)
                .map_err(|error| {
                    LiveError::Protocol(format!(
                        "execute Component channel-subscribe v1 run: {error}"
                    ))
                })?;
            result.map_err(|error| {
                LiveError::Protocol(format!(
                    "Component channel-subscribe v1 guest returned {:?}: {}",
                    error.code, error.message
                ))
            })?
        }
    };
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

    fn store_read_component() -> &'static [u8] {
        include_bytes!("../tests/fixtures/component-store-read/store-read.wasm")
    }

    fn store_write_component() -> &'static [u8] {
        include_bytes!("../tests/fixtures/component-store-write/store-write.wasm")
    }

    fn channel_publish_component() -> &'static [u8] {
        include_bytes!("../tests/fixtures/component-channel-publish/channel-publish.wasm")
    }

    fn channel_subscribe_component() -> &'static [u8] {
        include_bytes!("../tests/fixtures/component-channel-subscribe/channel-subscribe.wasm")
    }

    fn store_write_input(kappa: &str, bytes: &[u8]) -> Vec<u8> {
        let mut input = kappa.as_bytes().to_vec();
        input.push(b'\n');
        input.extend_from_slice(bytes);
        input
    }

    fn channel_publish_input(channel: &str, message: &[u8]) -> Vec<u8> {
        let mut input = channel.as_bytes().to_vec();
        input.push(b'\n');
        input.extend_from_slice(message);
        input
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
            ComponentHost::ImportFree,
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
            ComponentHost::ImportFree,
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
    fn store_read_host_only_serves_an_explicit_root() {
        let directory = tempfile::tempdir().expect("tempdir");
        let object_store = Arc::new(ObjectStore::open(directory.path()).expect("store"));
        let allowed = object_store
            .put("file", "application/octet-stream", None, b"allowed bytes")
            .expect("allowed object");
        let denied = object_store
            .put("file", "application/octet-stream", None, b"denied bytes")
            .expect("denied object");
        let component = PreparedComponent::compile(
            "fixture",
            store_read_component(),
            ComponentLimits::default(),
            ComponentHost::StoreRead(StoreReadHost {
                object_store,
                roots: Arc::from([allowed.id.clone()]),
            }),
        )
        .expect("compile store-read component");

        assert_eq!(
            component
                .run_inputs(
                    vec![allowed.id.as_bytes().to_vec()],
                    &AtomicBool::new(false),
                )
                .expect("read admitted root"),
            vec![b"allowed bytes".to_vec()]
        );
        let error = component
            .run_inputs(vec![denied.id.as_bytes().to_vec()], &AtomicBool::new(false))
            .expect_err("deny object outside roots");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
        assert!(error.to_string().contains("outside"), "{error}");
        assert!(!error.to_string().contains(&denied.id), "{error}");
    }

    #[test]
    fn store_write_host_only_materializes_an_exact_address_within_quota() {
        let directory = tempfile::tempdir().expect("tempdir");
        let object_store = Arc::new(ObjectStore::open(directory.path()).expect("store"));
        let bytes = b"capability mediated write";
        let allowed = hologram::space::address_bytes(bytes).to_string();
        let component = PreparedComponent::compile(
            "fixture",
            store_write_component(),
            ComponentLimits::default(),
            ComponentHost::StoreWrite(StoreWriteHost {
                object_store: object_store.clone(),
                roots: Arc::from([allowed.clone()]),
                quota_remaining: Arc::new(Mutex::new(
                    u64::try_from(bytes.len()).expect("fixture length"),
                )),
            }),
        )
        .expect("compile store-write component");

        assert_eq!(
            component
                .run_inputs(
                    vec![store_write_input(&allowed, bytes)],
                    &AtomicBool::new(false),
                )
                .expect("write admitted root"),
            vec![allowed.as_bytes().to_vec()]
        );
        assert_eq!(
            object_store
                .get_cached(&allowed)
                .expect("read written object"),
            Some(bytes.to_vec())
        );
    }

    #[test]
    fn store_write_quota_is_shared_across_inputs_and_charges_only_new_blobs() {
        let directory = tempfile::tempdir().expect("tempdir");
        let object_store = Arc::new(ObjectStore::open(directory.path()).expect("store"));
        let first_bytes = b"one";
        let second_bytes = b"second";
        let first = hologram::space::address_bytes(first_bytes).to_string();
        let second = hologram::space::address_bytes(second_bytes).to_string();
        let component = PreparedComponent::compile(
            "fixture",
            store_write_component(),
            ComponentLimits::default(),
            ComponentHost::StoreWrite(StoreWriteHost {
                object_store: object_store.clone(),
                roots: Arc::from([first.clone(), second.clone()]),
                quota_remaining: Arc::new(Mutex::new(
                    u64::try_from(first_bytes.len()).expect("fixture length"),
                )),
            }),
        )
        .expect("compile store-write component");

        let error = component
            .run_inputs(
                vec![
                    store_write_input(&first, first_bytes),
                    store_write_input(&second, second_bytes),
                ],
                &AtomicBool::new(false),
            )
            .expect_err("second new blob exceeds lifetime quota");
        assert!(error.to_string().contains("quota"), "{error}");
        assert_eq!(
            object_store.get_cached(&first).expect("first lookup"),
            Some(first_bytes.to_vec())
        );
        assert_eq!(
            object_store.get_cached(&second).expect("second lookup"),
            None
        );

        assert_eq!(
            component
                .run_inputs(
                    vec![store_write_input(&first, first_bytes)],
                    &AtomicBool::new(false),
                )
                .expect("existing blob consumes no additional quota"),
            vec![first.as_bytes().to_vec()]
        );
    }

    #[test]
    fn store_write_rejections_are_redacted_and_leave_no_partial_blob() {
        let directory = tempfile::tempdir().expect("tempdir");
        let object_store = Arc::new(ObjectStore::open(directory.path()).expect("store"));
        let expected = b"expected bytes";
        let allowed = hologram::space::address_bytes(expected).to_string();
        let denied_bytes = b"outside bytes";
        let denied = hologram::space::address_bytes(denied_bytes).to_string();

        let compile = |quota_remaining| {
            PreparedComponent::compile(
                "fixture",
                store_write_component(),
                ComponentLimits::default(),
                ComponentHost::StoreWrite(StoreWriteHost {
                    object_store: object_store.clone(),
                    roots: Arc::from([allowed.clone()]),
                    quota_remaining: Arc::new(Mutex::new(quota_remaining)),
                }),
            )
            .expect("compile store-write component")
        };

        let error = compile(u64::MAX)
            .run_inputs(
                vec![store_write_input(&denied, denied_bytes)],
                &AtomicBool::new(false),
            )
            .expect_err("deny object outside roots");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
        assert!(error.to_string().contains("outside"), "{error}");
        assert!(!error.to_string().contains(&denied), "{error}");
        assert_eq!(
            object_store.get_cached(&denied).expect("denied lookup"),
            None
        );

        let error = compile(u64::MAX)
            .run_inputs(
                vec![store_write_input(&allowed, b"wrong bytes")],
                &AtomicBool::new(false),
            )
            .expect_err("deny hash mismatch");
        assert!(error.to_string().contains("do not match"), "{error}");
        assert!(!error.to_string().contains(&allowed), "{error}");
        assert_eq!(
            object_store.get_cached(&allowed).expect("mismatch lookup"),
            None
        );

        let error = compile(1)
            .run_inputs(
                vec![store_write_input(&allowed, expected)],
                &AtomicBool::new(false),
            )
            .expect_err("deny quota overflow");
        assert!(error.to_string().contains("quota"), "{error}");
        assert_eq!(
            object_store.get_cached(&allowed).expect("quota lookup"),
            None
        );
    }

    #[test]
    fn channel_publish_and_subscribe_are_exact_fifo_and_at_most_once() {
        let broker = Arc::new(ChannelBroker::default());
        let channel = "blake3:0000000000000000000000000000000000000000000000000000000000000000";
        let publisher = PreparedComponent::compile(
            "fixture",
            channel_publish_component(),
            ComponentLimits::default(),
            ComponentHost::ChannelPublish(ChannelHost {
                broker: broker.clone(),
                channels: Arc::from([channel.to_owned()]),
            }),
        )
        .expect("compile channel publisher");
        let subscriber = PreparedComponent::compile(
            "fixture",
            channel_subscribe_component(),
            ComponentLimits::default(),
            ComponentHost::ChannelSubscribe(ChannelHost {
                broker,
                channels: Arc::from([channel.to_owned()]),
            }),
        )
        .expect("compile channel subscriber");

        for message in [b"one".as_slice(), b"two".as_slice()] {
            assert_eq!(
                publisher
                    .run_inputs(
                        vec![channel_publish_input(channel, message)],
                        &AtomicBool::new(false),
                    )
                    .expect("publish admitted message"),
                vec![channel.as_bytes().to_vec()]
            );
        }
        assert_eq!(
            subscriber
                .run_inputs(
                    vec![channel.as_bytes().to_vec(), channel.as_bytes().to_vec()],
                    &AtomicBool::new(false),
                )
                .expect("receive in order"),
            vec![b"one".to_vec(), b"two".to_vec()]
        );
        assert_eq!(
            subscriber
                .run_inputs(vec![channel.as_bytes().to_vec()], &AtomicBool::new(false),)
                .expect("empty mailbox is nonblocking"),
            vec![Vec::<u8>::new()]
        );
    }

    #[test]
    fn channel_authority_and_backpressure_errors_are_redacted() {
        let broker = Arc::new(ChannelBroker::new(3, 1));
        let allowed = "blake3:0000000000000000000000000000000000000000000000000000000000000000";
        let denied = "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let publisher = PreparedComponent::compile(
            "fixture",
            channel_publish_component(),
            ComponentLimits::default(),
            ComponentHost::ChannelPublish(ChannelHost {
                broker,
                channels: Arc::from([allowed.to_owned()]),
            }),
        )
        .expect("compile publisher");

        let error = publisher
            .run_inputs(
                vec![channel_publish_input(denied, b"one")],
                &AtomicBool::new(false),
            )
            .expect_err("deny channel outside publish set");
        assert!(error.to_string().contains("outside"), "{error}");
        assert!(!error.to_string().contains(denied), "{error}");

        publisher
            .run_inputs(
                vec![channel_publish_input(allowed, b"one")],
                &AtomicBool::new(false),
            )
            .expect("fill mailbox");
        let error = publisher
            .run_inputs(
                vec![channel_publish_input(allowed, b"two")],
                &AtomicBool::new(false),
            )
            .expect_err("full mailbox rejects");
        assert!(error.to_string().contains("full"), "{error}");
        assert!(!error.to_string().contains(allowed), "{error}");

        let error = publisher
            .run_inputs(
                vec![channel_publish_input(allowed, b"four")],
                &AtomicBool::new(false),
            )
            .expect_err("oversized message rejects");
        assert!(error.to_string().contains("limit"), "{error}");
    }

    #[test]
    fn component_profiles_do_not_accept_each_others_worlds() {
        let error = PreparedComponent::compile(
            "fixture",
            store_read_component(),
            ComponentLimits::default(),
            ComponentHost::ImportFree,
        )
        .err()
        .expect("base profile rejects store import");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
        assert!(error.to_string().contains("hologram:host/store"), "{error}");

        let directory = tempfile::tempdir().expect("tempdir");
        let object_store = Arc::new(ObjectStore::open(directory.path()).expect("store"));
        let error = PreparedComponent::compile(
            "fixture",
            echo_wat().as_bytes(),
            ComponentLimits::default(),
            ComponentHost::StoreRead(StoreReadHost {
                object_store,
                roots: Arc::from([
                    "blake3:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_owned(),
                ]),
            }),
        )
        .err()
        .expect("store profile rejects base world");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
        assert!(
            error
                .to_string()
                .contains("hologram:application-store-read"),
            "{error}"
        );

        let error = PreparedComponent::compile(
            "fixture",
            store_write_component(),
            ComponentLimits::default(),
            ComponentHost::ImportFree,
        )
        .err()
        .expect("base profile rejects store-write import");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
        assert!(error.to_string().contains("store-write"), "{error}");

        let error = PreparedComponent::compile(
            "fixture",
            store_read_component(),
            ComponentLimits::default(),
            ComponentHost::StoreWrite(StoreWriteHost {
                object_store: Arc::new(ObjectStore::open(directory.path()).expect("store")),
                roots: Arc::from([
                    "blake3:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_owned(),
                ]),
                quota_remaining: Arc::new(Mutex::new(1)),
            }),
        )
        .err()
        .expect("store-write profile rejects store-read world");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
        assert!(
            error
                .to_string()
                .contains("hologram:application-store-write"),
            "{error}"
        );

        let error = PreparedComponent::compile(
            "fixture",
            channel_publish_component(),
            ComponentLimits::default(),
            ComponentHost::ChannelSubscribe(ChannelHost {
                broker: Arc::new(ChannelBroker::default()),
                channels: Arc::from([
                    "blake3:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_owned(),
                ]),
            }),
        )
        .err()
        .expect("subscribe profile rejects publish world");
        assert!(error.to_string().contains("channel-subscribe"), "{error}");
    }

    #[test]
    fn undeclared_component_import_is_rejected_during_preparation() {
        let imported = br#"(component
          (type $forbidden-type (func))
          (import "forbidden" (func $forbidden (type $forbidden-type)))
        )"#;
        let error = PreparedComponent::compile(
            "fixture",
            imported,
            ComponentLimits::default(),
            ComponentHost::ImportFree,
        )
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
        let component = PreparedComponent::compile(
            "fixture",
            echo_wat().as_bytes(),
            limits,
            ComponentHost::ImportFree,
        )
        .expect("compile echo component");
        let error = component
            .run_inputs(vec![b"three".to_vec()], &AtomicBool::new(false))
            .expect_err("oversized input");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");

        limits.input_max_bytes = 16;
        limits.output_max_bytes = 2;
        let component = PreparedComponent::compile(
            "fixture",
            echo_wat().as_bytes(),
            limits,
            ComponentHost::ImportFree,
        )
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
        let error = PreparedComponent::compile(
            "fixture",
            echo_wat().as_bytes(),
            limits,
            ComponentHost::ImportFree,
        )
        .err()
        .expect("fixture requires more memory");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");

        limits = ComponentLimits::default();
        let mut component = PreparedComponent::compile(
            "fixture",
            echo_wat().as_bytes(),
            limits,
            ComponentHost::ImportFree,
        )
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
            PreparedComponent::compile(
                "fixture",
                spinning_wat().as_bytes(),
                limits,
                ComponentHost::ImportFree,
            )
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
            ComponentHost::ImportFree,
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
            PreparedComponent::compile(
                "fixture",
                spinning_wat().as_bytes(),
                limits,
                ComponentHost::ImportFree,
            )
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
