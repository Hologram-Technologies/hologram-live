use super::{helpers, Cli};
use clap::{Args, ValueEnum};
use hologram_live::compile::{compile_manifest_with, HoloPackaging};
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::HoloExecutor;
use hologram_live::holo_capability::{EffectiveGrant, GrantSource};
use hologram_live::protocol::{HoloRunResult, RpcRequest, RpcResponse};
use serde_json::Value;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
    let result = match development_grant {
        Some(path) => {
            let grant =
                EffectiveGrant::from_development_file(path, GrantSource::DirectDevelopmentFile)?;
            tracing::warn!(
                path = %path.display(),
                effective_grant_kappa = %grant.kappa,
                "direct holo development grant is enabled"
            );
            HoloExecutor::default()
                .execute_with_grant(&bytes, inputs, &grant)
                .await?
        }
        None => HoloExecutor::default().execute(&bytes, inputs).await?,
    };
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
