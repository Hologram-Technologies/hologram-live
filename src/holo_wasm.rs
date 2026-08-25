//! In-process wasm execution for `.holo` archives.
//!
//! Guest contract v1 (core wasm, no WASI): the module must export
//!
//! - `memory` — the guest linear memory the host reads and writes through.
//! - `holo_alloc(len: i32) -> i32` — returns a guest pointer the host may
//!   write `len` bytes into (a bump allocator is sufficient; every run uses a
//!   fresh instance, so no reclamation is required).
//! - `holo_run(ptr: i32, len: i32) -> i64` — executes one input of `len`
//!   bytes at `ptr` and returns the output location packed as
//!   `out_ptr << 32 | out_len`; the host reads `out_len` bytes at `out_ptr`
//!   from `memory`.
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
    LayerInvocation, LayerPrepareContext, LayerProvider, LayerRuntimeStatus, PreparedLayer,
    ProviderTarget,
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

/// One execution request against a resident holo: one output per input.
pub struct Run {
    pub inputs: Vec<Vec<u8>>,
}

/// Outputs plus wall-clock execution time, measured inside the actor.
pub struct RunOutcome {
    pub outputs: Vec<Vec<u8>>,
    pub elapsed_micros: u64,
}

/// A resident, compiled wasm holo. Spawned and stopped by `HoloRuntime`.
#[derive(Actor)]
pub struct ResidentHoloActor {
    module: Module,
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
        let kappa = kappa.into();
        let module = compile_module(engine, &kappa, wasm)?;
        Ok(Self {
            module,
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
        let outputs = run_inputs(&self.module, &self.kappa, &message.inputs)?;
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
    let started = Instant::now();
    let module = compile_module(engine, kappa, wasm)?;
    let outputs = run_inputs(&module, kappa, inputs)?;
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
        let payload = context.layer.content;
        let resident_bytes = payload.len();
        match self.target {
            ProviderTarget::Direct => {
                let compile_kappa = kappa.clone();
                let module = tokio::task::spawn_blocking(move || {
                    compile_module(&engine, &compile_kappa, &payload)
                })
                .await
                .map_err(|error| LiveError::Conflict(format!("join wasm prepare: {error}")))??;
                Ok(Arc::new(DirectWasmLayer {
                    position,
                    kappa,
                    module,
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
                    ResidentHoloActor::compile(compile_kappa, &engine, &payload, actor_processed)
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
    module: Module,
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
        let outputs = run_inputs(&self.module, &self.kappa, &inputs)?;
        self.processed.fetch_add(1, Ordering::Relaxed);
        Ok(LayerInvocation {
            outputs,
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

fn compile_module(engine: &Engine, kappa: &str, wasm: &[u8]) -> Result<Module> {
    let module = Module::new(engine, wasm).map_err(|error| {
        LiveError::InvalidHolo(format!("compile wasm layer of {kappa}: {error}"))
    })?;
    validate_contract(engine, &module)?;
    Ok(module)
}

fn run_inputs(module: &Module, kappa: &str, inputs: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
    inputs
        .iter()
        .map(|input| run_once(module, kappa, input))
        .collect()
}

/// Instantiate the module against a scratch store and resolve the contract
/// exports, so a malformed guest fails at load time instead of first run.
fn validate_contract(engine: &Engine, module: &Module) -> Result<()> {
    let mut store = Store::new(engine, ());
    let instance = instantiate(&mut store, module)?;
    contract_exports(&mut store, &instance)?;
    Ok(())
}

fn run_once(module: &Module, kappa: &str, input: &[u8]) -> Result<Vec<u8>> {
    let mut store = Store::new(module.engine(), ());
    let instance = instantiate(&mut store, module)?;
    let (memory, alloc, run) = contract_exports(&mut store, &instance)?;

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
        .map_err(|error| trap(kappa, "holo_run", error))?;
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

fn contract_exports(store: &mut Store<()>, instance: &Instance) -> Result<Contract> {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| missing_export("memory"))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut *store, "holo_alloc")
        .map_err(|_| missing_export("holo_alloc(len: i32) -> i32"))?;
    let run = instance
        .get_typed_func::<(i32, i32), i64>(&mut *store, "holo_run")
        .map_err(|_| missing_export("holo_run(ptr: i32, len: i32) -> i64"))?;
    Ok((memory, alloc, run))
}

fn missing_export(export: &str) -> LiveError {
    LiveError::Protocol(format!(
        "wasm guest is missing the required export `{export}` (guest contract v1)"
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
        let output = run_once(&actor.module, "blake3:test", b"hello").expect("run");
        assert_eq!(output, b"hello");
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
