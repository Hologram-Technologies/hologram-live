// The test entry of the session ABI probe (issue #629). It stands in for the
// generated code of a model so the runtime can be tested against the real
// generated shim; it is never packaged in a holo and is not an application.
//
// The turn is the state length, the state and the request. The state is a
// big-endian u32 counter followed by an accumulated buffer. A request whose
// first byte is below 0xf0 is appended to the buffer, the counter increments,
// and the step answers [counter, buffer]. The empty request answers nothing
// and keeps the state. Requests 0xf0 to 0xf4 answer what the shim must refuse.
pub fn step(turn: alloc::vec::Vec<u8>) -> alloc::vec::Vec<u8> {
    use alloc::vec::Vec;
    let state_len = u32::from_be_bytes([turn[0], turn[1], turn[2], turn[3]]) as usize;
    let state = &turn[4..4 + state_len];
    let request = &turn[4 + state_len..];
    let (counter, buffer) = if state.is_empty() {
        (0u32, &[][..])
    } else {
        (u32::from_be_bytes([state[0], state[1], state[2], state[3]]), &state[4..])
    };
    let mut answer = Vec::new();
    let push = |out: &mut Vec<u8>, bytes: &[u8]| {
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
    };
    match request.first().copied() {
        None => {
            push(&mut answer, state);
            answer.extend_from_slice(&0u32.to_be_bytes());
        }
        // A state longer than the declared maximum.
        Some(0xf0) => {
            push(&mut answer, &[0u8; 257]);
            answer.extend_from_slice(&0u32.to_be_bytes());
        }
        // A byte after the last response.
        Some(0xf1) => {
            push(&mut answer, state);
            answer.extend_from_slice(&0u32.to_be_bytes());
            answer.push(0);
        }
        // More responses than declared.
        Some(0xf2) => {
            push(&mut answer, state);
            answer.extend_from_slice(&5u32.to_be_bytes());
            for _ in 0..5 { push(&mut answer, &[]); }
        }
        // A response longer than declared.
        Some(0xf3) => {
            push(&mut answer, state);
            answer.extend_from_slice(&1u32.to_be_bytes());
            push(&mut answer, &[0u8; 257]);
        }
        // A checked failure in the model, which traps.
        Some(0xf4) => panic!(),
        Some(_) => {
            let counter = counter + 1;
            let mut next = Vec::new();
            next.extend_from_slice(&counter.to_be_bytes());
            next.extend_from_slice(buffer);
            next.extend_from_slice(request);
            push(&mut answer, &next);
            answer.extend_from_slice(&2u32.to_be_bytes());
            push(&mut answer, &counter.to_be_bytes());
            push(&mut answer, &next[4..]);
        }
    }
    answer
}
