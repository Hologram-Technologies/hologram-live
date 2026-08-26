//! Compile-time conformance for the accepted Component Model v1 WIT world.

wasmtime::component::bindgen!({
    path: "specs/wit",
    world: "application",
});

#[test]
fn component_v1_world_remains_import_free() {
    let wit = include_str!("../specs/wit/hologram-application-v1.wit");
    assert!(!wit
        .lines()
        .any(|line| line.trim_start().starts_with("import ")));
    assert!(wit.contains("run: func(input: list<u8>) -> result<list<u8>, guest-error>"));
}
