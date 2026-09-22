//! The reference image's command line, understood by the same binary.
//!
//! The registry image links `hologram` as `/bin/registry` and as
//! `/entrypoint.sh`, so the commands operators already type keep working:
//! `registry serve /etc/distribution/config.yml`, `registry garbage-collect
//! --dry-run cfg.yml`, `registry --version`, and the image's own entry point,
//! which takes a bare `config.yml`. Applied only when the file name of
//! `argv[0]` is one of those; `hologram registry …` keeps its existing meaning.

use std::ffi::OsString;
use std::path::Path;

/// What the process should do instead of parsing `hologram` arguments.
#[derive(Debug, PartialEq, Eq)]
pub enum Rewritten {
    /// Parse these as `hologram` arguments.
    Args(Vec<OsString>),
    /// Print this and exit 0.
    Print(String),
    /// Print this and exit 2.
    Refuse(String),
}

const USAGE: &str = "Usage:
  registry serve <config>                    serve the registry
  registry garbage-collect [flags] <config>  remove unreferenced blobs (--dry-run, --delete-untagged)
  registry --version                         print the version

Hologram Registry, a drop-in replacement for the Docker Registry image.";

/// `None` when `argv[0]` is not a registry name: parse as `hologram`.
pub fn rewrite(args: &[OsString]) -> Option<Rewritten> {
    let name = args
        .first()
        .and_then(|arg0| Path::new(arg0).file_name())?
        .to_string_lossy()
        .to_lowercase();
    let entrypoint = name == "entrypoint.sh";
    if !(entrypoint || name == "registry" || name == "registry.exe") {
        return None;
    }
    let rest: Vec<String> = args[1..]
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let hologram = |tail: &[&str], extra: &[String]| {
        let mut out: Vec<OsString> = std::iter::once("hologram")
            .chain(tail.iter().copied())
            .map(OsString::from)
            .collect();
        out.extend(extra.iter().map(OsString::from));
        Rewritten::Args(out)
    };
    // The image's entry point takes a bare file, as the reference's does:
    // `docker run registry:3 /etc/distribution/config.yml`.
    let rest = match rest.first().map(String::as_str) {
        Some(first) if entrypoint && is_yaml(first) => {
            std::iter::once("serve".to_owned()).chain(rest).collect()
        }
        _ => rest,
    };
    Some(match rest.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => Rewritten::Print(USAGE.to_owned()),
        Some("--version" | "-v" | "version") => {
            Rewritten::Print(format!("registry hologram-registry {}", env!("CARGO_PKG_VERSION")))
        }
        Some("serve") => match &rest[1..] {
            [file] => hologram(&["serve", "--registry-config"], std::slice::from_ref(file)),
            // As the reference: with no file, REGISTRY_CONFIGURATION_PATH names it.
            [] => match std::env::var_os("REGISTRY_CONFIGURATION_PATH") {
                Some(path) => Rewritten::Args(
                    ["hologram", "serve", "--registry-config"]
                        .iter()
                        .map(OsString::from)
                        .chain(std::iter::once(path))
                        .collect(),
                ),
                None => Rewritten::Refuse(
                    "registry serve: give the configuration file, or set REGISTRY_CONFIGURATION_PATH"
                        .to_owned(),
                ),
            },
            _ => Rewritten::Refuse("registry serve takes one argument: the configuration file".to_owned()),
        },
        Some("garbage-collect") => {
            let (flags, files): (Vec<String>, Vec<String>) =
                rest[1..].iter().cloned().partition(|arg| arg.starts_with('-'));
            match files.as_slice() {
                [file] => {
                    let mut tail = flags;
                    tail.push("--registry-config".to_owned());
                    tail.push(file.clone());
                    hologram(&["oci", "garbage-collect"], &tail)
                }
                _ => Rewritten::Refuse("registry garbage-collect: give one configuration file".to_owned()),
            }
        }
        Some(other) => Rewritten::Refuse(format!(
            "registry: unknown command {other:?}; this image runs the registry only (serve, garbage-collect, --version)"
        )),
    })
}

fn is_yaml(argument: &str) -> bool {
    Path::new(argument).extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("yml") || extension.eq_ignore_ascii_case("yaml")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Option<Rewritten> {
        rewrite(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    fn args(items: &[&str]) -> Rewritten {
        Rewritten::Args(items.iter().map(OsString::from).collect())
    }

    #[test]
    fn the_reference_command_lines() {
        assert_eq!(
            run(&["registry", "serve", "/etc/distribution/config.yml"]),
            Some(args(&[
                "hologram",
                "serve",
                "--registry-config",
                "/etc/distribution/config.yml"
            ]))
        );
        assert_eq!(
            run(&[
                "/bin/registry",
                "garbage-collect",
                "--dry-run",
                "--delete-untagged",
                "cfg.yml"
            ]),
            Some(args(&[
                "hologram",
                "oci",
                "garbage-collect",
                "--dry-run",
                "--delete-untagged",
                "--registry-config",
                "cfg.yml"
            ]))
        );
        assert!(
            matches!(run(&["registry", "--version"]), Some(Rewritten::Print(text)) if text.starts_with("registry "))
        );
        assert!(matches!(run(&["registry"]), Some(Rewritten::Print(_))));
        assert!(matches!(
            run(&["registry", "serve"]),
            Some(Rewritten::Refuse(_))
        ));
        assert!(matches!(
            run(&["registry", "sh"]),
            Some(Rewritten::Refuse(_))
        ));
        // Only Windows splits a path on `\`; elsewhere this is one file name.
        #[cfg(windows)]
        assert!(matches!(
            run(&["C:\\bin\\REGISTRY.EXE", "--version"]),
            Some(Rewritten::Print(_))
        ));
        assert!(matches!(
            run(&["/usr/bin/REGISTRY", "--version"]),
            Some(Rewritten::Print(_))
        ));
    }

    /// `docker run registry:3 /etc/distribution/config.yml`, as the reference's entry point takes it.
    #[test]
    fn the_images_entry_point_takes_a_bare_file() {
        assert_eq!(
            run(&["/entrypoint.sh", "/etc/distribution/config.yml"]),
            Some(args(&[
                "hologram",
                "serve",
                "--registry-config",
                "/etc/distribution/config.yml"
            ]))
        );
        assert_eq!(
            run(&["/entrypoint.sh", "serve", "/etc/distribution/config.yml"]),
            Some(args(&[
                "hologram",
                "serve",
                "--registry-config",
                "/etc/distribution/config.yml"
            ]))
        );
    }

    #[test]
    fn any_other_name_is_hologram() {
        assert_eq!(run(&["hologram", "registry", "serve"]), None);
        assert_eq!(run(&["/usr/local/bin/hologram", "serve"]), None);
    }
}
