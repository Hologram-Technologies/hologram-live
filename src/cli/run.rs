use super::{helpers, Cli};
use clap::{Args, ValueEnum};
use hologram_live::error::{LiveError, Result};
use hologram_live::holo::HoloExecutor;
use hologram_live::protocol::{HoloRunResult, RpcRequest, RpcResponse};
use serde_json::Value;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    /// Catalog kappa, or a local self-contained .holo file.
    pub(crate) reference: String,
    #[arg(long = "input")]
    pub(crate) inputs: Vec<PathBuf>,
    /// Render application outputs as raw protocol bytes, UTF-8 text, or JSON.
    #[arg(long, value_enum, default_value_t = RunOutputFormat::Raw)]
    pub(crate) output_format: RunOutputFormat,
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
    let mut inputs = Vec::with_capacity(args.inputs.len());
    for path in args.inputs {
        inputs.push(
            tokio::fs::read(&path)
                .await
                .map_err(|error| LiveError::io(&path, error))?,
        );
    }
    let local = PathBuf::from(&args.reference);
    if local.is_file()
        || local
            .extension()
            .is_some_and(|extension| extension == "holo")
    {
        let bytes = tokio::fs::read(&local)
            .await
            .map_err(|error| LiveError::io(&local, error))?;
        let result = HoloExecutor::default().execute(&bytes, inputs).await?;
        return print_result(&cli, &result, args.output_format);
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
}
