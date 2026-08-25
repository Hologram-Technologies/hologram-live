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

use crate::error::{LiveError, Result};
use kameo::message::{Context, Message};
use kameo::Actor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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
        let module = Module::new(engine, wasm).map_err(|error| {
            LiveError::InvalidHolo(format!("compile wasm layer of {kappa}: {error}"))
        })?;
        validate_contract(engine, &module)?;
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
        let mut outputs = Vec::with_capacity(message.inputs.len());
        for input in &message.inputs {
            outputs.push(run_once(&self.module, &self.kappa, input)?);
        }
        self.processed.fetch_add(1, Ordering::Relaxed);
        Ok(RunOutcome {
            outputs,
            elapsed_micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        })
    }
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
}
