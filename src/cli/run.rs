use super::{helpers, Cli};
use clap::{Args, ValueEnum};
use hologram_live::actor::ActorSystem;
use hologram_live::audit::AuditLog;
use hologram_live::compile::{compile_manifest_with, HoloPackaging};
use hologram_live::config::AppConfig;
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::HoloExecutor;
use hologram_live::holo_capability::{EffectiveGrant, GrantSource};
use hologram_live::protocol::{HoloRunResult, RpcRequest, RpcResponse};
use serde_json::Value;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    /// Project directory, hologram.json, local .holo file, or catalog kappa.
    pub(crate) reference: String,
    #[arg(long = "input")]
    pub(crate) inputs: Vec<PathBuf>,
    /// Pass a UTF-8 input value without creating a temporary file.
    #[arg(long = "input-text")]
    pub(crate) input_texts: Vec<String>,
    /// Render application outputs as raw protocol bytes, UTF-8 text, or JSON.
    #[arg(long, value_enum, default_value_t = RunOutputFormat::Raw)]
    pub(crate) output_format: RunOutputFormat,
    /// Development-only effective grant for direct local execution.
    #[arg(long, value_name = "CAPABILITIES_JSON")]
    pub(crate) development_grant: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum RunOutputFormat {
    /// Preserve the `HoloRunResult` envelope and byte arrays.
    #[default]
    Raw,
    /// Decode each output as UTF-8 and print it directly.
    Text,
    /// Decode each output as JSON and print the value directly.
    Json,
}

pub async fn run(cli: Cli, args: RunArgs) -> Result<()> {
    let mut inputs = Vec::with_capacity(args.inputs.len() + args.input_texts.len());
    for path in args.inputs {
        inputs.push(
            tokio::fs::read(&path)
                .await
                .map_err(|error| LiveError::io(&path, error))?,
        );
    }
    inputs.extend(args.input_texts.into_iter().map(String::into_bytes));

    let local = PathBuf::from(&args.reference);
    if let Some(manifest) = project_manifest(&local) {
        let bytes = compile_project(manifest).await?;
        return execute_local(
            &cli,
            bytes,
            inputs,
            args.development_grant.as_deref(),
            args.output_format,
        )
        .await;
    }
    if local.is_file()
        || local
            .extension()
            .is_some_and(|extension| extension == "holo")
    {
        let bytes = tokio::fs::read(&local)
            .await
            .map_err(|error| LiveError::io(&local, error))?;
        return execute_local(
            &cli,
            bytes,
            inputs,
            args.development_grant.as_deref(),
            args.output_format,
        )
        .await;
    }
    if args.development_grant.is_some() {
        return Err(LiveError::Config(
            "--development-grant applies only to direct local .holo files; configure holo.development_grant on the service for catalog execution"
                .to_owned(),
        ));
    }
    // Precedence rule 3. Rules 1 and 2 -- a catalog kappa and an existing path
    // -- are handled above and are deliberately untouched, so no invocation
    // that worked before means something different now. The configuration is
    // loaded here rather than at the top for the same reason: the earlier
    // paths must not acquire a new failure mode.
    let (run_config, _) = helpers::load(&cli)?;
    if let hologram_live::artifact_ref::Resolution::Reference(reference) =
        hologram_live::artifact_ref::resolve(&args.reference, &run_config.registry)?
    {
        let bytes = pull_archive_bytes(&cli, &run_config, &reference).await?;
        return execute_local(&cli, bytes, inputs, None, args.output_format).await;
    }
    match helpers::call(
        &cli,
        RpcRequest::HoloRun {
            kappa: args.reference,
            inputs,
        },
    )
    .await?
    {
        RpcResponse::HoloRun(value) => print_result(&cli, &value, args.output_format),
        other => helpers::unexpected(other),
    }
}

/// Fetch a named artifact and return its archive bytes.
///
/// The archive is executed directly from bytes rather than imported, which
/// keeps this on the same path as a local `.holo` file: pulling grants no
/// authority, so a pulled archive must reach execution exactly the way a local
/// one does.
async fn pull_archive_bytes(
    cli: &Cli,
    config: &hologram_live::config::AppConfig,
    reference: &hologram_live::artifact_ref::ArtifactRef,
) -> Result<Vec<u8>> {
    use hologram_live::artifact_pull::{pull, LayerSource, PullProgress};
    use hologram_live::registry::kappa_client::KappaClient;
    use hologram_live::store::ObjectStore;

    let client = KappaClient::new(&config.registry)?;
    let store = ObjectStore::open(config.paths.data_dir.join("registry"))?;
    let reference = reference.clone();
    let json = cli.json;
    let mut emit = move |progress: PullProgress| {
        if json {
            if let Ok(line) = serde_json::to_string(&progress) {
                eprintln!("{line}");
            }
        } else if progress.source == LayerSource::Fetched {
            eprintln!(
                "fetched [{}/{}] {}",
                progress.index + 1,
                progress.total,
                progress.kappa
            );
        }
    };

    tokio::task::spawn_blocking(move || {
        let report = pull(&client, &store, &reference, &mut emit)?;
        store.get_cached(&report.archive_kappa)?.ok_or_else(|| {
            LiveError::NotFound(format!(
                "pulled archive {} is absent from the local store",
                report.archive_kappa
            ))
        })
    })
    .await
    .map_err(|error| LiveError::Conflict(format!("join artifact pull: {error}")))?
}

fn project_manifest(reference: &Path) -> Option<PathBuf> {
    if reference.is_dir() {
        return Some(reference.join("hologram.json"));
    }
    (reference.file_name() == Some(OsStr::new("hologram.json"))).then(|| reference.to_path_buf())
}

async fn compile_project(manifest: PathBuf) -> Result<Vec<u8>> {
    let compiled =
        tokio::task::spawn_blocking(move || compile_manifest_with(&manifest, HoloPackaging::Fat))
            .await
            .map_err(|error| LiveError::Conflict(format!("compile task failed: {error}")))??;
    Ok(compiled.bytes)
}

async fn execute_local(
    cli: &Cli,
    bytes: Vec<u8>,
    inputs: Vec<Vec<u8>>,
    development_grant: Option<&Path>,
    output_format: RunOutputFormat,
) -> Result<()> {
    let grant = match development_grant {
        Some(path) => {
            let grant =
                EffectiveGrant::from_development_file(path, GrantSource::DirectDevelopmentFile)?;
            tracing::warn!(
                path = %path.display(),
                effective_grant_kappa = %grant.kappa,
                "direct holo development grant is enabled"
            );
            grant
        }
        None => EffectiveGrant::local_baseline(),
    };
    let (config, _) = AppConfig::load(cli.config.as_deref())?;
    config.create_directories()?;
    let actors = ActorSystem::start();
    let audit = AuditLog::open(
        config.paths.state_dir.join("audit.jsonl"),
        config.server.actor_mailbox_capacity,
        actors.root(),
    )
    .await?;
    let object_store = Arc::new(hologram_live::store::ObjectStore::open(
        config.paths.data_dir.join("registry"),
    )?);
    let result = HoloExecutor::with_object_store(object_store)
        .execute_with_grant_and_audit(&bytes, inputs, &grant, &audit, "local-cli")
        .await?;
    print_result(cli, &result, output_format)
}

fn print_result(cli: &Cli, result: &HoloRunResult, format: RunOutputFormat) -> Result<()> {
    match format {
        RunOutputFormat::Raw => helpers::print(cli, result),
        RunOutputFormat::Text => {
            let outputs = decode_text_outputs(&result.outputs)?;
            if cli.json {
                return helpers::print(cli, &outputs);
            }
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            for output in outputs {
                stdout.write_all(output.as_bytes())?;
                if !output.ends_with('\n') {
                    stdout.write_all(b"\n")?;
                }
            }
            Ok(())
        }
        RunOutputFormat::Json => {
            let output = decode_json_outputs(&result.outputs)?;
            println!("{}", serde_json::to_string_pretty(&output)?);
            Ok(())
        }
    }
}

fn decode_text_outputs(outputs: &[Vec<u8>]) -> Result<Vec<&str>> {
    outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            std::str::from_utf8(output).map_err(|error| {
                LiveError::Protocol(format!(
                    "run output {index} is not valid UTF-8 and cannot use --output-format text: {error}"
                ))
            })
        })
        .collect()
}

fn decode_json_outputs(outputs: &[Vec<u8>]) -> Result<Value> {
    let mut values = outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            serde_json::from_slice(output).map_err(|error| {
                LiveError::Protocol(format!(
                    "run output {index} is not valid JSON and cannot use --output-format json: {error}"
                ))
            })
        })
        .collect::<Result<Vec<Value>>>()?;
    if values.len() == 1 {
        Ok(values.remove(0))
    } else {
        Ok(Value::Array(values))
    }
}

#[cfg(test)]
mod tests {

    /// Adding reference support must not change what an existing invocation
    /// means. Rules 1 and 2 are checked before any registry work, so a path
    /// that exists keeps winning even when it looks exactly like a reference.
    #[test]
    fn an_existing_file_still_wins_over_a_registry_reference() {
        use hologram_live::artifact_ref::{resolve, Resolution};

        let config = hologram_live::config::RegistryConfig::default();
        let directory = std::env::temp_dir().join(format!(
            "hologram-run-precedence-{}-{}",
            std::process::id(),
            hologram_live::util::now_millis()
        ));
        std::fs::create_dir_all(&directory).expect("create");
        // A filename that is also a valid artifact reference: the ambiguity
        // this rule exists to settle.
        let archive = directory.join("qwen3.5:4b");
        std::fs::write(&archive, b"archive").expect("write");

        let resolved = resolve(&archive.to_string_lossy(), &config).expect("resolve");
        assert!(
            matches!(resolved, Resolution::File(_)),
            "a path that exists must resolve to a file, got {resolved:?}"
        );

        let _ = std::fs::remove_dir_all(directory);
    }

    /// A catalog kappa must never be treated as a name to fetch.
    #[test]
    fn a_kappa_still_resolves_to_the_catalog() {
        use hologram_live::artifact_ref::{resolve, Resolution};

        let config = hologram_live::config::RegistryConfig::default();
        let kappa = "blake3:cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959";
        assert!(
            matches!(
                resolve(kappa, &config).expect("resolve"),
                Resolution::Kappa(_)
            ),
            "a kappa must not be sent to the registry as a name"
        );
    }
    use super::*;

    #[test]
    fn text_output_decodes_utf8() {
        let outputs = vec![b"first".to_vec(), "second λ".as_bytes().to_vec()];
        assert_eq!(
            decode_text_outputs(&outputs).expect("decode"),
            ["first", "second λ"]
        );
    }

    #[test]
    fn text_output_rejects_binary_bytes() {
        let error = decode_text_outputs(&[vec![0xff]])
            .expect_err("invalid UTF-8 should be rejected")
            .to_string();
        assert!(error.contains("run output 0 is not valid UTF-8"), "{error}");
    }

    #[test]
    fn json_output_unwraps_one_value() {
        let output = decode_json_outputs(&[br#"{"answer":42}"#.to_vec()]).expect("decode");
        assert_eq!(output, serde_json::json!({"answer": 42}));
    }

    #[test]
    fn json_output_preserves_multiple_values_as_an_array() {
        let output =
            decode_json_outputs(&[b"1".to_vec(), br#"{"two":2}"#.to_vec()]).expect("decode");
        assert_eq!(output, serde_json::json!([1, {"two": 2}]));
    }

    #[test]
    fn json_output_rejects_non_json_payloads() {
        let error = decode_json_outputs(&[b"not JSON".to_vec()])
            .expect_err("invalid JSON should be rejected")
            .to_string();
        assert!(error.contains("run output 0 is not valid JSON"), "{error}");
    }

    #[test]
    fn project_directory_resolves_its_manifest() {
        let directory = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            project_manifest(directory.path()),
            Some(directory.path().join("hologram.json"))
        );
    }

    #[test]
    fn manifest_file_is_a_project_reference_but_archive_is_not() {
        assert_eq!(
            project_manifest(Path::new("example/hologram.json")),
            Some(PathBuf::from("example/hologram.json"))
        );
        assert_eq!(project_manifest(Path::new("example/app.holo")), None);
    }

    #[tokio::test]
    async fn source_project_compiles_into_an_executable_archive() {
        let manifest =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("features/fixtures/wasm-app/hologram.json");
        let bytes = compile_project(manifest).await.expect("compile project");

        let result = HoloExecutor::default()
            .execute(&bytes, vec![b"hello project test".to_vec()])
            .await
            .expect("execute compiled project");

        assert_eq!(result.outputs, [b"HELLO PROJECT TEST"]);
    }
}
