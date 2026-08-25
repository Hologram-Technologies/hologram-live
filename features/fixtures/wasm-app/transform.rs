//! Original source for `transform.wat` (documentation only — the fixture
//! ships the hand-tuned WAT so CI needs no wasm toolchain). This is the Rust
//! program the WAT mirrors: an ASCII-uppercase transform implementing guest
//! contract v1 (see `src/holo_wasm.rs`).
//!
//! To rebuild an equivalent wasm binary instead of using the WAT text:
//!
//! ```sh
//! rustup target add wasm32-unknown-unknown
//! rustc --target wasm32-unknown-unknown -O --crate-type cdylib transform.rs
//! ```
//!
//! then point the `wasm` layer's `path` in `hologram.json` at the produced
//! `transform.wasm`.

#![no_std]
#![no_main]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

static mut HEAP: usize = 1024;

#[no_mangle]
pub extern "C" fn holo_alloc(len: i32) -> i32 {
    unsafe {
        let ptr = HEAP;
        HEAP += len as usize;
        ptr as i32
    }
}

#[no_mangle]
pub extern "C" fn holo_run(ptr: i32, len: i32) -> i64 {
    unsafe {
        let input = core::slice::from_raw_parts(ptr as *const u8, len as usize);
        let out = holo_alloc(len) as *mut u8;
        for (i, byte) in input.iter().enumerate() {
            *out.add(i) = byte.to_ascii_uppercase();
        }
        ((out as i64) << 32) | i64::from(len)
    }
}
