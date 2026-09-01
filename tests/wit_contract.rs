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

#[test]
fn component_store_read_profiles_share_one_fixed_import() {
    let world = include_str!("../specs/wit/store-read/hologram-application-store-read-v1.wit");
    let imports = world
        .lines()
        .filter(|line| line.trim_start().starts_with("import "))
        .collect::<Vec<_>>();
    assert_eq!(imports, ["  import hologram:host/store@1.0.0;"]);
    assert!(world.contains("export guest;"));

    // Exact-root and typed-graph authority are selected by distinct contracts,
    // while deliberately retaining the same narrow guest ABI.
    assert_ne!(
        hologram_live::holo_contract::WASM_CONTRACT_COMPONENT_STORE_READ_V1,
        hologram_live::holo_contract::WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1
    );
}

#[test]
fn component_store_write_profile_has_one_fixed_import() {
    let world = include_str!("../specs/wit/store-write/hologram-application-store-write-v1.wit");
    let imports = world
        .lines()
        .filter(|line| line.trim_start().starts_with("import "))
        .collect::<Vec<_>>();
    assert_eq!(imports, ["  import hologram:host/store-write@1.0.0;"]);
    assert!(world.contains("export guest;"));
}

#[test]
fn component_channel_profiles_each_have_one_fixed_import() {
    fn imports(world: &str) -> Vec<&str> {
        world
            .lines()
            .filter(|line| line.trim_start().starts_with("import "))
            .collect()
    }

    let publish =
        include_str!("../specs/wit/channel-publish/hologram-application-channel-publish-v1.wit");
    let subscribe = include_str!(
        "../specs/wit/channel-subscribe/hologram-application-channel-subscribe-v1.wit"
    );
    assert_eq!(
        imports(publish),
        ["  import hologram:host/channel-publish@1.0.0;"]
    );
    assert_eq!(
        imports(subscribe),
        ["  import hologram:host/channel-subscribe@1.0.0;"]
    );
}
