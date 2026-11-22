//! In-process execution of the core-Wasm session world `hologram:guest/core-wasm@2`.
//!
//! A session guest exports:
//!
//! - `memory`, the guest linear memory;
//! - `holo_alloc(len: i32) -> i32`, which reserves `len` bytes for the next request;
//! - the manifest-declared entry, `holo_step(ptr: i32, len: i32) -> i64`, which takes one step
//!   over the request at `ptr` and returns the responses as `ptr << 32 | len`.
//!
//! One instance is one session. The guest keeps its state between steps and never shows it to
//! the host. The responses of a step are a frame of big-endian `u32`s: `count`, then `count`
//! times `length` and `response`, with nothing after. A trap ends the session, and a later step
//! on it runs nothing. The guest imports nothing.
//!
//! Unlike `core-wasm@1`, which runs every input on a fresh instance, a provider runs the inputs
//! of one invocation as the ordered steps of one fresh session.

use crate::application_plan::ProviderContext;
use crate::error::{LiveError, Result};
use crate::holo_provider::{
    LayerCompletion, LayerInvocation, LayerPrepareContext, LayerProvider, LayerRuntimeStatus,
    PreparedLayer, ProviderTarget,
};
use hologram::space::LayerKind;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

pub const CORE_WASM_V2: &str = "core-wasm-v2";
/// The only entry of the session world: a `@1` entry name is never accepted for it.
pub const CORE_WASM_V2_ENTRY: &str = "holo_step";
/// The most responses one step may return.
pub const RESPONSE_COUNT_LIMIT: u32 = 65_535;

/// A compiled session guest whose exports satisfy the session world.
#[derive(Clone)]
pub struct SessionGuest {
    module: Module,
    kappa: String,
}

impl SessionGuest {
    /// Compile `wasm` and check its exports on a scratch instance, so a malformed guest fails
    /// before any session opens.
    pub fn compile(engine: &Engine, kappa: &str, wasm: &[u8], entry: &str) -> Result<Self> {
        if entry != CORE_WASM_V2_ENTRY {
            return Err(LiveError::Protocol(format!(
                "{CORE_WASM_V2} entry must be {CORE_WASM_V2_ENTRY:?}, got {entry:?}"
            )));
        }
        let module = Module::new(engine, wasm).map_err(|error| {
            LiveError::InvalidHolo(format!("compile wasm layer of {kappa}: {error}"))
        })?;
        if module.imports().len() != 0 {
            return Err(LiveError::Protocol(format!(
                "wasm guest {kappa} does not satisfy {CORE_WASM_V2}: it imports"
            )));
        }
        let guest = Self {
            module,
            kappa: kappa.to_owned(),
        };
        guest.open()?;
        Ok(guest)
    }

    /// Open a session: a fresh instance with an empty state.
    pub fn open(&self) -> Result<Session> {
        let mut store = Store::new(self.module.engine(), ());
        let instance = Instance::new(&mut store, &self.module, &[])
            .map_err(|error| LiveError::Protocol(format!("instantiate wasm guest: {error}")))?;
        let (memory, alloc, step) = exports(&mut store, &instance)?;
        Ok(Session {
            store,
            memory,
            alloc,
            step,
            kappa: self.kappa.clone(),
            ended: false,
        })
    }
}

/// One open session: one instance, whose steps are taken in order.
pub struct Session {
    store: Store<()>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    step: TypedFunc<(i32, i32), i64>,
    kappa: String,
    ended: bool,
}

impl Session {
    /// Take one step and return its responses. A trap or a malformed responses frame ends the
    /// session; every later step fails without running.
    pub fn step(&mut self, request: &[u8]) -> Result<Vec<Vec<u8>>> {
        let frame = self.step_frame(request)?;
        decode_responses(&frame).map_err(LiveError::Protocol)
    }

    /// Take one step and return its responses frame exactly as the guest wrote it, once it has
    /// been checked to be well formed.
    pub fn step_frame(&mut self, request: &[u8]) -> Result<Vec<u8>> {
        if self.ended {
            return Err(LiveError::Conflict(format!(
                "session of {} has ended",
                self.kappa
            )));
        }
        let result = self.run(request);
        if result.is_err() {
            self.ended = true;
        }
        let frame = result?;
        if let Err(reason) = decode_responses(&frame) {
            self.ended = true;
            return Err(LiveError::Protocol(format!(
                "guest {} answered {reason}",
                self.kappa
            )));
        }
        Ok(frame)
    }

    /// Whether a trap or a malformed answer ended this session.
    pub fn ended(&self) -> bool {
        self.ended
    }

    fn run(&mut self, request: &[u8]) -> Result<Vec<u8>> {
        let kappa = self.kappa.clone();
        let length = i32::try_from(request.len()).map_err(|_| {
            LiveError::Protocol(format!(
                "request of {} bytes exceeds the 2 GiB guest contract limit for {kappa}",
                request.len()
            ))
        })?;
        let pointer = self
            .alloc
            .call(&mut self.store, length)
            .map_err(|error| trap(&kappa, "holo_alloc", error))?;
        let offset = usize::try_from(pointer).map_err(|_| {
            LiveError::Protocol(format!(
                "guest {kappa} returned the negative pointer {pointer}"
            ))
        })?;
        self.memory
            .write(&mut self.store, offset, request)
            .map_err(|error| {
                LiveError::Protocol(format!(
                    "write request into guest memory of {kappa}: {error}"
                ))
            })?;
        let packed = self
            .step
            .call(&mut self.store, (pointer, length))
            .map_err(|error| trap(&kappa, CORE_WASM_V2_ENTRY, error))?
            .cast_unsigned();
        let output = usize::try_from(packed >> 32).unwrap_or(usize::MAX);
        let output_length = usize::try_from(packed & 0xFFFF_FFFF).unwrap_or(usize::MAX);
        let mut frame = vec![0_u8; output_length];
        self.memory
            .read(&self.store, output, &mut frame)
            .map_err(|error| {
                LiveError::Protocol(format!(
                    "read responses from guest memory of {kappa}: {error}"
                ))
            })?;
        Ok(frame)
    }
}

/// Run `requests` as the ordered steps of one fresh session and return each step's responses.
pub fn execute_session(
    engine: &Engine,
    kappa: &str,
    wasm: &[u8],
    entry: &str,
    requests: &[Vec<u8>],
) -> Result<Vec<Vec<Vec<u8>>>> {
    let mut session = SessionGuest::compile(engine, kappa, wasm, entry)?.open()?;
    requests
        .iter()
        .map(|request| session.step(request))
        .collect()
}

/// Decode a responses frame: `count`, then `count` times `length` and `response`, all
/// big-endian `u32`, with nothing after.
pub fn decode_responses(frame: &[u8]) -> std::result::Result<Vec<Vec<u8>>, String> {
    fn take<'a>(frame: &'a [u8], at: &mut usize, length: usize) -> Option<&'a [u8]> {
        let end = at.checked_add(length)?;
        let bytes = frame.get(*at..end)?;
        *at = end;
        Some(bytes)
    }
    fn word(frame: &[u8], at: &mut usize) -> Option<u32> {
        take(frame, at, 4).map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four bytes")))
    }
    let mut at = 0;
    let count = word(frame, &mut at).ok_or("a responses frame without a count")?;
    if count > RESPONSE_COUNT_LIMIT {
        return Err(format!("{count} responses, over {RESPONSE_COUNT_LIMIT}"));
    }
    let mut responses = Vec::with_capacity(count as usize);
    for index in 0..count {
        let length = word(frame, &mut at)
            .ok_or_else(|| format!("a responses frame that ends before response {index}"))?;
        let bytes = take(frame, &mut at, length as usize)
            .ok_or_else(|| format!("a responses frame shorter than response {index}"))?;
        responses.push(bytes.to_vec());
    }
    if at != frame.len() {
        return Err(format!(
            "{} bytes after the last response",
            frame.len() - at
        ));
    }
    Ok(responses)
}

type Exports = (Memory, TypedFunc<i32, i32>, TypedFunc<(i32, i32), i64>);

fn exports(store: &mut Store<()>, instance: &Instance) -> Result<Exports> {
    let missing = |export: &str| {
        LiveError::Protocol(format!(
            "wasm guest does not satisfy {CORE_WASM_V2}: required export `{export}` is missing or has an incompatible type"
        ))
    };
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| missing("memory"))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut *store, "holo_alloc")
        .map_err(|_| missing("holo_alloc(len: i32) -> i32"))?;
    let step = instance
        .get_typed_func::<(i32, i32), i64>(&mut *store, CORE_WASM_V2_ENTRY)
        .map_err(|_| missing("holo_step(ptr: i32, len: i32) -> i64"))?;
    Ok((memory, alloc, step))
}

fn trap(kappa: &str, export: &str, error: impl std::fmt::Display) -> LiveError {
    LiveError::Protocol(format!("guest {kappa} trapped in `{export}`: {error}"))
}

/// The direct provider of the session world: each invocation is one fresh session whose
/// ordered steps are the invocation's inputs, and each output is that step's responses frame.
pub struct SessionProvider {
    engine: Engine,
}

impl SessionProvider {
    pub fn direct(engine: Engine) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl LayerProvider for SessionProvider {
    fn kind(&self) -> LayerKind {
        LayerKind::WasmCodemodule
    }

    fn contract(&self) -> Option<&'static str> {
        Some(crate::holo_contract::WASM_CONTRACT_CORE_V2)
    }

    fn name(&self) -> &'static str {
        "wasmtime-session-direct"
    }

    fn availability(
        &self,
        _context: &ProviderContext<'_>,
        target: ProviderTarget,
    ) -> Result<(), String> {
        if target == ProviderTarget::Direct {
            Ok(())
        } else {
            Err(format!(
                "{} provider runs sessions directly, not {}",
                self.name(),
                target.name()
            ))
        }
    }

    async fn prepare(&self, context: LayerPrepareContext) -> Result<Arc<dyn PreparedLayer>> {
        if context.target != ProviderTarget::Direct {
            return Err(LiveError::Conflict(format!(
                "provider {} cannot prepare a {} layer",
                self.name(),
                context.target.name()
            )));
        }
        let engine = self.engine.clone();
        let kappa = context.identity.archive_kappa;
        let entry = context.layer.entry;
        let payload = context.layer.content;
        let resident_bytes = payload.len();
        let compile_kappa = kappa.clone();
        let guest = tokio::task::spawn_blocking(move || {
            SessionGuest::compile(&engine, &compile_kappa, &payload, &entry)
        })
        .await
        .map_err(|error| LiveError::Conflict(format!("join wasm prepare: {error}")))??;
        Ok(Arc::new(DirectSessionLayer {
            position: context.layer.position,
            guest,
            resident_bytes,
            running: AtomicBool::new(false),
            processed: AtomicUsize::new(0),
        }))
    }
}

struct DirectSessionLayer {
    position: u32,
    guest: SessionGuest,
    resident_bytes: usize,
    running: AtomicBool,
    processed: AtomicUsize,
}

#[tonic::async_trait]
impl PreparedLayer for DirectSessionLayer {
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
                "direct session layer {} is not running",
                self.position
            )));
        }
        let started = Instant::now();
        let mut session = self.guest.open()?;
        let outputs = inputs
            .iter()
            .map(|request| session.step_frame(request))
            .collect::<Result<Vec<_>>>()?;
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

#[cfg(test)]
mod tests;
