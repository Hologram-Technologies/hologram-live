use super::*;

/// `probe.wasm` is `PrismPM`'s session ABI probe (UOR-Foundation/prismpm,
/// `sdk/npm/prism-code/test/fixtures/session-probe`, on main at 65819f12): the shim `PrismPM`
/// generates for `core-wasm@2`, with the bounds request 64, state 256, response 256 and
/// response count 4, around the test entry in `entry.rs`.
const PROBE: &[u8] = include_bytes!("fixtures/probe.wasm");
const PROBE_SHA256: &str = "7fbfb094f82f4dc182fb7a931e8cc96c945729ce8f82b59fa4ac9b771b5183b4";

/// `PrismPM`'s test entry, compiled natively: a big-endian `u32` counter and an accumulated
/// buffer. It is the same source the probe's guest is built from.
#[allow(clippy::cast_possible_truncation)] // the entry is PrismPM's source, unchanged
mod native {
    extern crate alloc;
    include!("fixtures/entry.rs");
}

/// `PrismPM`'s native guest: the entry, with the turn and answer frames and every declared bound
/// checked as the generated shim checks them. A refusal is `None`, where the guest traps.
struct NativeSession {
    state: Vec<u8>,
}

impl NativeSession {
    const REQUEST: usize = 64;
    const STATE: usize = 256;
    const RESPONSE: usize = 256;
    const COUNT: usize = 4;

    fn open() -> Self {
        Self { state: Vec::new() }
    }

    fn step(&mut self, request: &[u8]) -> Option<Vec<Vec<u8>>> {
        if request.len() > Self::REQUEST {
            return None;
        }
        let mut turn = u32::try_from(self.state.len())
            .unwrap()
            .to_be_bytes()
            .to_vec();
        turn.extend_from_slice(&self.state);
        turn.extend_from_slice(request);
        let answer = std::panic::catch_unwind(|| native::step(turn)).ok()?;
        let word = |at: usize| -> Option<usize> {
            Some(u32::from_be_bytes(answer.get(at..at + 4)?.try_into().ok()?) as usize)
        };
        let state_length = word(0)?;
        if state_length > Self::STATE {
            return None;
        }
        let state = answer.get(4..4 + state_length)?.to_vec();
        let mut at = 4 + state_length;
        let count = word(at)?;
        if count > Self::COUNT {
            return None;
        }
        at += 4;
        let mut responses = Vec::new();
        for _ in 0..count {
            let length = word(at)?;
            if length > Self::RESPONSE {
                return None;
            }
            responses.push(answer.get(at + 4..at + 4 + length)?.to_vec());
            at += 4 + length;
        }
        if at != answer.len() {
            return None;
        }
        self.state = state;
        Some(responses)
    }
}

fn guest() -> SessionGuest {
    SessionGuest::compile(&Engine::default(), "probe", PROBE, CORE_WASM_V2_ENTRY)
        .expect("the probe satisfies core-wasm@2")
}

fn u32be(value: u32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

#[test]
fn the_fixture_is_prismpms_committed_probe() {
    use sha2::{Digest, Sha256};
    assert_eq!(format!("{:x}", Sha256::digest(PROBE)), PROBE_SHA256);
}

#[test]
fn the_counter_keeps_its_state_across_the_steps_of_one_session() {
    let mut session = guest().open().unwrap();
    assert_eq!(session.step(b"ab").unwrap(), [u32be(1), b"ab".to_vec()]);
    assert_eq!(session.step(b"c").unwrap(), [u32be(2), b"abc".to_vec()]);
    assert_eq!(session.step(b"d").unwrap(), [u32be(3), b"abcd".to_vec()]);
    assert_eq!(session.step(b"").unwrap(), Vec::<Vec<u8>>::new());
    assert_eq!(session.step(b"e").unwrap(), [u32be(4), b"abcde".to_vec()]);
}

#[test]
fn every_step_agrees_with_prismpms_native_guest() {
    let sessions: [&[&[u8]]; 4] = [
        &[b"ab", b"c", b"d", b"", b"e"],
        &[b"x"],
        &[b"", b"", b"hello", b"", b"world"],
        &[b"a", b"b", b"c", b"d", b"e", b"f", b"g", b"h"],
    ];
    let guest = guest();
    for requests in sessions {
        let mut wasm = guest.open().unwrap();
        let mut native = NativeSession::open();
        for request in requests {
            let expected = native.step(request).expect("the native guest answers");
            assert_eq!(wasm.step(request).unwrap(), expected, "request {request:?}");
        }
    }
}

#[test]
fn two_sessions_never_share_a_state() {
    let guest = guest();
    let mut first = guest.open().unwrap();
    let mut second = guest.open().unwrap();
    first.step(b"ab").unwrap();
    assert_eq!(second.step(b"x").unwrap(), [u32be(1), b"x".to_vec()]);
    assert_eq!(first.step(b"c").unwrap(), [u32be(2), b"abc".to_vec()]);
}

#[test]
fn what_the_shim_refuses_traps_ends_the_session_and_agrees_with_native() {
    let guest = guest();
    let over = vec![b'a'; NativeSession::REQUEST + 1];
    for request in [&[0xf0][..], &[0xf1], &[0xf2], &[0xf3], &[0xf4], &over] {
        let mut native = NativeSession::open();
        native.step(b"ab").unwrap();
        assert_eq!(native.step(request), None, "native refuses {request:?}");
        let mut session = guest.open().unwrap();
        session.step(b"ab").unwrap();
        let error = session.step(request).expect_err("the guest traps");
        assert!(error.to_string().contains("trapped"), "{error}");
        assert!(session.ended());
        let later = session
            .step(b"c")
            .expect_err("an ended session runs nothing");
        assert!(later.to_string().contains("has ended"), "{later}");
    }
}

#[test]
fn execute_session_runs_the_requests_in_order_on_one_instance() {
    let steps = execute_session(
        &Engine::default(),
        "probe",
        PROBE,
        CORE_WASM_V2_ENTRY,
        &[b"ab".to_vec(), b"c".to_vec()],
    )
    .unwrap();
    assert_eq!(
        steps,
        [
            vec![u32be(1), b"ab".to_vec()],
            vec![u32be(2), b"abc".to_vec()]
        ]
    );
}

#[test]
fn a_v1_entry_or_a_v1_guest_is_refused() {
    let engine = Engine::default();
    let entry = SessionGuest::compile(&engine, "probe", PROBE, "holo_run")
        .err()
        .expect("holo_run is not the session entry");
    assert!(entry.to_string().contains("holo_step"), "{entry}");
    let v1 = r#"(module
          (memory (export "memory") 1)
          (func (export "holo_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "holo_run") (param i32 i32) (result i64) (i64.const 0)))"#;
    let error = SessionGuest::compile(&engine, "v1", v1.as_bytes(), CORE_WASM_V2_ENTRY)
        .err()
        .expect("a @1 guest has no holo_step");
    assert!(error.to_string().contains("holo_step"), "{error}");
}

#[test]
fn a_malformed_responses_frame_is_refused() {
    let frame = |words: &[&[u8]]| words.concat();
    assert_eq!(decode_responses(&u32be(0)).unwrap(), Vec::<Vec<u8>>::new());
    assert_eq!(
        decode_responses(&frame(&[&u32be(2), &u32be(1), b"a", &u32be(0)])).unwrap(),
        [b"a".to_vec(), Vec::new()]
    );
    for bad in [
        Vec::new(),
        vec![0, 0, 0],
        frame(&[&u32be(1)]),
        frame(&[&u32be(1), &u32be(2), b"a"]),
        frame(&[&u32be(0), b"x"]),
        frame(&[&u32be(RESPONSE_COUNT_LIMIT + 1)]),
    ] {
        assert!(decode_responses(&bad).is_err(), "{bad:?}");
    }
}

/// A compiled archive whose primary layer is the session world runs through the one-shot
/// executor: its inputs are the ordered steps of one session, and each output is the step's
/// responses frame, which agrees with `PrismPM`'s native guest.
#[tokio::test]
async fn a_core_wasm_2_archive_runs_as_one_session_through_the_executor() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/holo_session/fixtures/hologram.json");
    let compiled = crate::compile::compile_manifest(&manifest).expect("compile the @2 fixture");
    let requests = [b"ab".to_vec(), b"c".to_vec(), Vec::new(), b"d".to_vec()];
    let result = crate::holo::HoloExecutor::default()
        .execute(&compiled.bytes, requests.to_vec())
        .await
        .expect("execute the session");
    let mut native = NativeSession::open();
    let expected = requests
        .iter()
        .map(|request| {
            let responses = native.step(request).expect("the native guest answers");
            let mut frame = u32be(u32::try_from(responses.len()).unwrap());
            for response in responses {
                frame.extend(u32be(u32::try_from(response.len()).unwrap()));
                frame.extend(response);
            }
            frame
        })
        .collect::<Vec<_>>();
    assert_eq!(result.outputs, expected);
    assert_eq!(
        decode_responses(&result.outputs[2]).unwrap(),
        Vec::<Vec<u8>>::new()
    );
}
