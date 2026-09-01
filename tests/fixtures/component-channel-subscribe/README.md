# Component channel-subscribe fixture

The checked-in `channel-subscribe.wasm` is generated from this crate for
runtime contract tests. Input is an exact channel κ. The guest performs one
nonblocking receive and returns either the next message or empty bytes.
