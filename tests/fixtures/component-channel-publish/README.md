# Component channel-publish fixture

The checked-in `channel-publish.wasm` is generated from this crate for runtime
contract tests. Input is an exact channel κ, a newline, and the message bytes.
The guest publishes once and returns the channel κ.
