//! In-process wasm execution for `.holo` archives.
//!
//! Core-Wasm guest contract v1 (`core-wasm-v1`, no WASI) requires the module
//! to export:
//!
//! - `memory` — the guest linear memory the host reads and writes through.
//! - `holo_alloc(len: i32) -> i32` — returns a guest pointer the host may
//!   write `len` bytes into (a bump allocator is sufficient; every run uses a
//!   fresh instance, so no reclamation is required).
//! - the Wasm layer's manifest-declared `entry` with signature
//!   `(ptr: i32, len: i32) -> i64` — executes one input of `len` bytes at
//!   `ptr` and returns the output location packed as
//!   `out_ptr << 32 | out_len`; the host reads `out_len` bytes at `out_ptr`
//!   from `memory`.
//!
//! `holo_run` is only the default entry used when generating a manifest.
//! Runtime providers always use the entry bound into the canonical application
//! manifest. V1 accepts no
//! imports or WASI functions and exposes no numeric process exit status: a
//! returned byte value is successful completion, while a trap is a typed
//! protocol failure.
//!
//! All pointers and lengths must be non-negative `i32` values. A module that
//! is missing any of these exports (or exports them with different types) is
//! rejected with `LIVE_PROTOCOL_ERROR` naming the offending export.
//!
//! Residency means the module stays compiled and warm; each `Run` message
//! still executes against a fresh `Store` and instance, so guests cannot
//! accumulate state between invocations.

use crate::actor::RootSupervisor;
use crate::application_plan::ProviderContext;
use crate::error::{LiveError, Result};
use crate::holo_provider::{
    LayerCompletion, LayerInvocation, LayerPrepareContext, LayerProvider, LayerRuntimeStatus,
    PreparedLayer, ProviderTarget,
};
use hologram::space::LayerKind;
use kameo::actor::{ActorRef, Spawn};
use kameo::error::SendError;
use kameo::mailbox;
use kameo::message::{Context, Message};
use kameo::Actor;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

pub const CORE_WASM_V1: &str = "core-wasm-v1";
pub const CORE_WASM_V1_DEFAULT_ENTRY: &str = "holo_run";

pub fn validate_entry_name(entry: &str) -> std::result::Result<(), &'static str> {
    if entry.is_empty() {
        return Err("must not be empty");
    }
    if entry.len() > 256 {
        return Err("must be at most 256 UTF-8 bytes");
    }
    if entry.chars().any(char::is_control) {
        return Err("must not contain control characters");
    }
    Ok(())
}

#[derive(Clone)]
struct CoreWasmV1Guest {
    module: Module,
    entry: String,
}

impl CoreWasmV1Guest {
    fn compile(engine: &Engine, kappa: &str, wasm: &[u8], entry: &str) -> Result<Self> {
        validate_entry_name(entry).map_err(|reason| {
            LiveError::Protocol(format!(
                "invalid {CORE_WASM_V1} manifest entry {entry:?}: {reason}"
            ))
        })?;
        let module = Module::new(engine, wasm).map_err(|error| {
            LiveError::InvalidHolo(format!("compile wasm layer of {kappa}: {error}"))
        })?;
        validate_contract(engine, &module, entry)?;
        Ok(Self {
            module,
            entry: entry.to_owned(),
        })
    }

    fn run_inputs(&self, kappa: &str, inputs: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
        inputs
            .iter()
            .map(|input| run_once(&self.module, kappa, &self.entry, input))
            .collect()
    }
}

/// One execution request against a resident holo: one output per input.
pub struct Run {
    pub inputs: Vec<Vec<u8>>,
}

/// Outputs plus wall-clock execution time, measured inside the actor.
#[derive(Debug)]
pub struct RunOutcome {
    pub outputs: Vec<Vec<u8>>,
    pub elapsed_micros: u64,
}

/// A resident, compiled wasm holo. Spawned and stopped by `HoloRuntime`.
#[derive(Actor)]
pub struct ResidentHoloActor {
    guest: CoreWasmV1Guest,
    kappa: String,
    processed: Arc<AtomicUsize>,
}

impl ResidentHoloActor {
    /// Compile `wasm` and verify the guest contract before residency starts.
    pub fn compile(
        kappa: impl Into<String>,
        engine: &Engine,
        wasm: &[u8],
        processed: Arc<AtomicUsize>,
    ) -> Result<Self> {
        Self::compile_entry(kappa, engine, wasm, CORE_WASM_V1_DEFAULT_ENTRY, processed)
    }

    pub fn compile_entry(
        kappa: impl Into<String>,
        engine: &Engine,
        wasm: &[u8],
        entry: &str,
        processed: Arc<AtomicUsize>,
    ) -> Result<Self> {
        let kappa = kappa.into();
        let guest = CoreWasmV1Guest::compile(engine, &kappa, wasm, entry)?;
        Ok(Self {
            guest,
            kappa,
            processed,
        })
    }
}

impl Message<Run> for ResidentHoloActor {
    type Reply = Result<RunOutcome>;

    async fn handle(
        &mut self,
        message: Run,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let started = Instant::now();
        let outputs = self.guest.run_inputs(&self.kappa, &message.inputs)?;
        self.processed.fetch_add(1, Ordering::Relaxed);
        Ok(RunOutcome {
            outputs,
            elapsed_micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        })
    }
}

/// Compile and execute a Wasm layer without making it resident. The local
/// `.holo` file executor uses this path; resident execution uses the same
/// compiler and per-input invocation below through `ResidentHoloActor`.
pub fn execute(
    engine: &Engine,
    kappa: &str,
    wasm: &[u8],
    inputs: &[Vec<u8>],
) -> Result<RunOutcome> {
    execute_entry(engine, kappa, wasm, CORE_WASM_V1_DEFAULT_ENTRY, inputs)
}

pub fn execute_entry(
    engine: &Engine,
    kappa: &str,
    wasm: &[u8],
    entry: &str,
    inputs: &[Vec<u8>],
) -> Result<RunOutcome> {
    let started = Instant::now();
    let guest = CoreWasmV1Guest::compile(engine, kappa, wasm, entry)?;
    let outputs = guest.run_inputs(kappa, inputs)?;
    Ok(RunOutcome {
        outputs,
        elapsed_micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
    })
}

pub struct WasmProvider {
    target: ProviderTarget,
    engine: Engine,
    resident_root: Option<ActorRef<RootSupervisor>>,
    mailbox_capacity: usize,
}

impl WasmProvider {
    pub fn direct(engine: Engine) -> Self {
        Self {
            target: ProviderTarget::Direct,
            engine,
            resident_root: None,
            mailbox_capacity: 1,
        }
    }

    pub fn resident(
        engine: Engine,
        root: Option<ActorRef<RootSupervisor>>,
        mailbox_capacity: usize,
    ) -> Self {
        Self {
            target: ProviderTarget::Resident,
            engine,
            resident_root: root,
            mailbox_capacity: mailbox_capacity.max(1),
        }
    }
}

#[tonic::async_trait]
impl LayerProvider for WasmProvider {
    fn kind(&self) -> LayerKind {
        LayerKind::WasmCodemodule
    }

    fn contract(&self) -> Option<&'static str> {
        Some(crate::holo_contract::WASM_CONTRACT_CORE_V1)
    }

    fn name(&self) -> &'static str {
        match self.target {
            ProviderTarget::Direct => "wasmtime-direct",
            ProviderTarget::Resident => "wasmtime-resident",
        }
    }

    fn availability(
        &self,
        _context: &ProviderContext<'_>,
        target: ProviderTarget,
    ) -> Result<(), String> {
        if target == self.target {
            Ok(())
        } else {
            Err(format!(
                "{} provider is configured for {}, not {}",
                self.name(),
                self.target.name(),
                target.name()
            ))
        }
    }

    async fn prepare(&self, context: LayerPrepareContext) -> Result<Arc<dyn PreparedLayer>> {
        if context.target != self.target {
            return Err(LiveError::Conflict(format!(
                "provider {} cannot prepare a {} layer",
                self.name(),
                context.target.name()
            )));
        }
        let engine = self.engine.clone();
        let kappa = context.identity.archive_kappa;
        let position = context.layer.position;
        let entry = context.layer.entry;
        let payload = context.layer.content;
        let resident_bytes = payload.len();
        match self.target {
            ProviderTarget::Direct => {
                let compile_kappa = kappa.clone();
                let module = tokio::task::spawn_blocking(move || {
                    CoreWasmV1Guest::compile(&engine, &compile_kappa, &payload, &entry)
                })
                .await
                .map_err(|error| LiveError::Conflict(format!("join wasm prepare: {error}")))??;
                Ok(Arc::new(DirectWasmLayer {
                    position,
                    kappa,
                    guest: module,
                    resident_bytes,
                    running: AtomicBool::new(false),
                    processed: AtomicUsize::new(0),
                }))
            }
            ProviderTarget::Resident => {
                let root = self.resident_root.clone().ok_or_else(|| {
                    LiveError::Conflict("resident Wasm provider has no actor root".to_owned())
                })?;
                let processed = Arc::new(AtomicUsize::new(0));
                let actor_processed = processed.clone();
                let compile_kappa = kappa.clone();
                let actor = tokio::task::spawn_blocking(move || {
                    ResidentHoloActor::compile_entry(
                        compile_kappa,
                        &engine,
                        &payload,
                        &entry,
                        actor_processed,
                    )
                })
                .await
                .map_err(|error| {
                    LiveError::Conflict(format!("join resident wasm prepare: {error}"))
                })??;
                Ok(Arc::new(ResidentWasmLayer {
                    position,
                    root,
                    mailbox_capacity: self.mailbox_capacity,
                    prepared: Mutex::new(Some(actor)),
                    actor: Mutex::new(None),
                    resident_bytes,
                    queued: AtomicUsize::new(0),
                    processed,
                }))
            }
        }
    }
}

struct DirectWasmLayer {
    position: u32,
    kappa: String,
    guest: CoreWasmV1Guest,
    resident_bytes: usize,
    running: AtomicBool,
    processed: AtomicUsize,
}

#[tonic::async_trait]
impl PreparedLayer for DirectWasmLayer {
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
                "direct Wasm layer {} is not running",
                self.position
            )));
        }
        let started = Instant::now();
        let outputs = self.guest.run_inputs(&self.kappa, &inputs)?;
        self.processed.fetch_add(1, Ordering::Relaxed);
        Ok(LayerInvocation {
            outputs,
            completion: LayerCompletion::Returned,
            elapsed_micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        })
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Release);
        Ok(())
    }

    fn status(&self) -> LayerRuntimeStatus {
        LayerRuntimeStatus {
            resident_bytes: self.resident_bytes,
            queued: 0,
            processed: self.processed.load(Ordering::Relaxed),
        }
    }
}

struct ResidentWasmLayer {
    position: u32,
    root: ActorRef<RootSupervisor>,
    mailbox_capacity: usize,
    prepared: Mutex<Option<ResidentHoloActor>>,
    actor: Mutex<Option<ActorRef<ResidentHoloActor>>>,
    resident_bytes: usize,
    queued: AtomicUsize,
    processed: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl PreparedLayer for ResidentWasmLayer {
    fn position(&self) -> u32 {
        self.position
    }

    async fn start(&self) -> Result<()> {
        if self.lock_actor()?.is_some() {
            return Ok(());
        }
        let prepared = self
            .prepared
            .lock()
            .map_err(|_| LiveError::Conflict("prepared Wasm layer lock poisoned".to_owned()))?
            .take()
            .ok_or_else(|| {
                LiveError::Conflict(format!(
                    "resident Wasm layer {} has no prepared actor",
                    self.position
                ))
            })?;
        let actor = ResidentHoloActor::spawn_link_with_mailbox(
            &self.root,
            prepared,
            mailbox::bounded(self.mailbox_capacity),
        )
        .await;
        *self.lock_actor()? = Some(actor);
        Ok(())
    }

    async fn invoke(&self, inputs: Vec<Vec<u8>>) -> Result<LayerInvocation> {
        let actor = self.lock_actor()?.clone().ok_or_else(|| {
            LiveError::Conflict(format!(
                "resident Wasm layer {} is not running",
                self.position
            ))
        })?;
        self.queued.fetch_add(1, Ordering::Relaxed);
        let reply = actor.ask(Run { inputs }).await;
        self.queued.fetch_sub(1, Ordering::Relaxed);
        let outcome = match reply {
            Ok(outcome) => outcome,
            Err(SendError::HandlerError(error)) => return Err(error),
            Err(error) => {
                return Err(LiveError::Conflict(format!(
                    "resident Wasm layer {} is unavailable: {error}",
                    self.position
                )));
            }
        };
        Ok(LayerInvocation {
            outputs: outcome.outputs,
            completion: LayerCompletion::Returned,
            elapsed_micros: outcome.elapsed_micros,
        })
    }

    async fn stop(&self) -> Result<()> {
        let actor = self.lock_actor()?.take();
        if let Some(actor) = actor {
            let _ = actor.stop_gracefully().await;
            actor.wait_for_shutdown().await;
        }
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

impl ResidentWasmLayer {
    fn lock_actor(&self) -> Result<std::sync::MutexGuard<'_, Option<ActorRef<ResidentHoloActor>>>> {
        self.actor
            .lock()
            .map_err(|_| LiveError::Conflict("resident Wasm actor lock poisoned".to_owned()))
    }
}

/// Instantiate the module against a scratch store and resolve the contract
/// exports, so a malformed guest fails at load time instead of first run.
fn validate_contract(engine: &Engine, module: &Module, entry: &str) -> Result<()> {
    let mut store = Store::new(engine, ());
    let instance = instantiate(&mut store, module)?;
    contract_exports(&mut store, &instance, entry)?;
    Ok(())
}

fn run_once(module: &Module, kappa: &str, entry: &str, input: &[u8]) -> Result<Vec<u8>> {
    let mut store = Store::new(module.engine(), ());
    let instance = instantiate(&mut store, module)?;
    let (memory, alloc, run) = contract_exports(&mut store, &instance, entry)?;

    let input_len = i32::try_from(input.len()).map_err(|_| {
        LiveError::Protocol(format!(
            "input of {} bytes exceeds the 2 GiB guest contract limit for {kappa}",
            input.len()
        ))
    })?;
    let input_ptr = alloc
        .call(&mut store, input_len)
        .map_err(|error| trap(kappa, "holo_alloc", error))?;
    memory
        .write(&mut store, guest_offset(kappa, input_ptr)?, input)
        .map_err(|error| {
            LiveError::Protocol(format!("write input into guest memory of {kappa}: {error}"))
        })?;

    let packed = run
        .call(&mut store, (input_ptr, input_len))
        .map_err(|error| trap(kappa, entry, error))?;
    let packed = packed.cast_unsigned();
    let output_ptr = u32::try_from(packed >> 32).map_err(|_| {
        LiveError::Protocol(format!(
            "guest {kappa} returned an out-of-range output pointer"
        ))
    })?;
    let output_len = u32::try_from(packed & 0xFFFF_FFFF).expect("masked to 32 bits");
    let mut output = vec![0_u8; usize::try_from(output_len).unwrap_or(usize::MAX)];
    memory
        .read(
            &store,
            usize::try_from(output_ptr).unwrap_or(usize::MAX),
            &mut output,
        )
        .map_err(|error| {
            LiveError::Protocol(format!("read output from guest memory of {kappa}: {error}"))
        })?;
    Ok(output)
}

fn instantiate(store: &mut Store<()>, module: &Module) -> Result<Instance> {
    Instance::new(store, module, &[])
        .map_err(|error| LiveError::Protocol(format!("instantiate wasm guest: {error}")))
}

type Contract = (Memory, TypedFunc<i32, i32>, TypedFunc<(i32, i32), i64>);

fn contract_exports(store: &mut Store<()>, instance: &Instance, entry: &str) -> Result<Contract> {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| contract_export_error("memory", entry))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut *store, "holo_alloc")
        .map_err(|_| contract_export_error("holo_alloc(len: i32) -> i32", entry))?;
    let run = instance
        .get_typed_func::<(i32, i32), i64>(&mut *store, entry)
        .map_err(|_| {
            contract_export_error(&format!("{entry}(ptr: i32, len: i32) -> i64"), entry)
        })?;
    Ok((memory, alloc, run))
}

fn contract_export_error(export: &str, entry: &str) -> LiveError {
    LiveError::Protocol(format!(
        "wasm guest does not satisfy {CORE_WASM_V1} for manifest entry {entry:?}: required export `{export}` is missing or has an incompatible type"
    ))
}

fn guest_offset(kappa: &str, pointer: i32) -> Result<usize> {
    usize::try_from(pointer).map_err(|_| {
        LiveError::Protocol(format!(
            "guest {kappa} returned the negative pointer {pointer}"
        ))
    })
}

fn trap(kappa: &str, export: &str, error: impl std::fmt::Display) -> LiveError {
    LiveError::Protocol(format!("guest {kappa} trapped in `{export}`: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ECHO_WAT: &str = r#"
        (module
          (memory (export "memory") 1)
          (global $heap (mut i32) (i32.const 1024))
          (func (export "holo_alloc") (param $len i32) (result i32)
            (global.get $heap))
          (func (export "holo_run") (param $ptr i32) (param $len i32) (result i64)
            (i64.or
              (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
              (i64.extend_i32_u (local.get $len)))))
    "#;

    #[test]
    fn rejects_a_guest_missing_a_contract_export() {
        let engine = Engine::default();
        let error = ResidentHoloActor::compile(
            "blake3:test",
            &engine,
            ECHO_WAT
                .replace("holo_alloc", "not_the_allocator")
                .as_bytes(),
            Arc::new(AtomicUsize::new(0)),
        )
        .err()
        .expect("must fail");
        assert_eq!(error.code(), "LIVE_PROTOCOL_ERROR");
        assert!(error.to_string().contains("holo_alloc"), "{error}");
    }

    #[test]
    fn executes_a_contract_guest() {
        let engine = Engine::default();
        let actor = ResidentHoloActor::compile(
            "blake3:test",
            &engine,
            ECHO_WAT.as_bytes(),
            Arc::new(AtomicUsize::new(0)),
        )
        .expect("compile");
        let output = actor
            .guest
            .run_inputs("blake3:test", &[b"hello".to_vec()])
            .expect("run");
        assert_eq!(output, [b"hello"]);
    }

    #[test]
    fn executes_the_manifest_declared_entry() {
        let engine = Engine::default();
        let custom = ECHO_WAT.replace("holo_run", "transform");
        let outcome = execute_entry(
            &engine,
            "blake3:test",
            custom.as_bytes(),
            "transform",
            &[b"hello".to_vec()],
        )
        .expect("execute custom entry");
        assert_eq!(outcome.outputs, [b"hello"]);
    }

    #[test]
    fn rejects_a_missing_or_wrongly_typed_manifest_entry_during_compilation() {
        let engine = Engine::default();
        let missing = execute_entry(
            &engine,
            "blake3:test",
            ECHO_WAT.as_bytes(),
            "transform",
            &[],
        )
        .expect_err("missing manifest entry");
        assert_eq!(missing.code(), "LIVE_PROTOCOL_ERROR");
        assert!(missing.to_string().contains("transform"));
        assert!(missing.to_string().contains(CORE_WASM_V1));

        let wrong_type = ECHO_WAT.replace(
            "(func (export \"holo_run\") (param $ptr i32) (param $len i32) (result i64)",
            "(func (export \"holo_run\") (param $ptr i32) (param $len i32) (param $extra i32) (result i64)",
        );
        let wrong = execute_entry(
            &engine,
            "blake3:test",
            wrong_type.as_bytes(),
            CORE_WASM_V1_DEFAULT_ENTRY,
            &[],
        )
        .expect_err("wrongly typed manifest entry");
        assert_eq!(wrong.code(), "LIVE_PROTOCOL_ERROR");
        assert!(wrong.to_string().contains(CORE_WASM_V1_DEFAULT_ENTRY));
        assert!(wrong.to_string().contains("incompatible type"));
    }

    #[test]
    fn one_shot_execution_uses_the_same_guest_contract() {
        let engine = Engine::default();
        let outcome = execute(
            &engine,
            "blake3:test",
            ECHO_WAT.as_bytes(),
            &[b"hello".to_vec(), b"world".to_vec()],
        )
        .expect("execute");
        assert_eq!(outcome.outputs, [b"hello", b"world"]);
    }
}
