use cucumber::{given, then, when, World as _};
use hologram_live::holo::inspect_bytes;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Default, cucumber::World)]
struct BddWorld {
    manifest: Option<PathBuf>,
    output_path: Option<PathBuf>,
    command_output: Option<Output>,
    temporary: Option<tempfile::TempDir>,
}

#[given("the example view application manifest")]
fn example_manifest(world: &mut BddWorld) {
    world.manifest = Some(
        workspace_root()
            .join("features")
            .join("fixtures")
            .join("view-app")
            .join("hologram.json"),
    );
    world.temporary = Some(tempfile::tempdir().expect("create scenario directory"));
}

#[when("I compile the application")]
fn compile_application(world: &mut BddWorld) {
    let destination = world
        .temporary
        .as_ref()
        .expect("scenario directory")
        .path()
        .join("view.holo");
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("compile")
        .arg(world.manifest.as_ref().expect("manifest"))
        .arg("--output")
        .arg(&destination)
        .output()
        .expect("run hologram compile");
    world.output_path = Some(destination);
    world.command_output = Some(output);
}

#[then("the compile command succeeds")]
fn compile_succeeds(world: &mut BddWorld) {
    let output = world.command_output.as_ref().expect("command output");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then("the output is a valid self-contained .holo archive")]
fn output_is_self_contained(world: &mut BddWorld) {
    let path = world.output_path.as_ref().expect("output path");
    let bytes = std::fs::read(path).expect("read compiled archive");
    let inspection = inspect_bytes("bdd", "view.holo", &bytes).expect("inspect archive");
    assert!(inspection.footer_verified);
    assert!(inspection
        .sections
        .iter()
        .any(|section| section.kind == "AppManifest"));
    assert!(inspection
        .sections
        .iter()
        .any(|section| section.kind == "ContentBlob"));
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[tokio::main]
async fn main() {
    BddWorld::cucumber()
        .fail_on_skipped_with(|feature, _rule, scenario| {
            feature
                .tags
                .iter()
                .chain(scenario.tags.iter())
                .any(|tag| tag.trim_start_matches('@') == "status:enforced")
        })
        .run_and_exit(concat!(env!("CARGO_MANIFEST_DIR"), "/features/suites"))
        .await;
}
