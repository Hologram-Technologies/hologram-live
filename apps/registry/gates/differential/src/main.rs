//! Gate B. One scripted session, played against the reference registry and
//! against Hologram Registry, compared field by field.
//!
//!   differential run --left reference --right reference
//!   differential run --left reference --right binary:target/debug/hologram --allowlist apps/registry/DIFFERENCES.md
//!   differential record --target reference --out golden
//!   differential check-golden --target reference

mod allowlist;
mod compare;
mod normalise;
mod play;
mod scenario;
mod sides;

use compare::Difference;
use normalise::Transcript;
use scenario::Scenario;
use sides::Target;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const HERE: &str = env!("CARGO_MANIFEST_DIR");

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

/// `--name value` pairs after the command.
fn options(args: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut rest = args.iter();
    while let Some(name) = rest.next() {
        let name = name.strip_prefix("--").ok_or_else(|| format!("expected --option, found {name}"))?;
        let value = rest.next().ok_or_else(|| format!("--{name} needs a value"))?;
        out.insert(name.to_owned(), value.clone());
    }
    Ok(out)
}

fn scenarios(only: Option<&String>) -> Result<Vec<Scenario>, String> {
    let dir = Path::new(HERE).join("scenarios");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let scenario = Scenario::load(&path)?;
        if only.is_none_or(|only| *only == scenario.name) {
            out.push(scenario);
        }
    }
    if out.is_empty() {
        return Err("no scenario matches".to_owned());
    }
    Ok(out)
}

/// A fresh registry, one scenario, one transcript.
fn transcript(target: &Target, scenario: &Scenario) -> Result<Transcript, String> {
    let running = sides::start(target, &scenario.needs)?;
    play::play(scenario, &running.base, &play::client()?)
}

fn run(args: &[String]) -> Result<bool, String> {
    let (command, rest) = args.split_first().ok_or("usage: differential run|record|check-golden …")?;
    let options = options(rest)?;
    let target = |name: &str| Target::parse(options.get(name).ok_or_else(|| format!("--{name} is required"))?);
    let golden = options.get("golden").or_else(|| options.get("out")).map_or_else(|| Path::new(HERE).join("golden"), PathBuf::from);
    let set = scenarios(options.get("scenario"))?;
    match command.as_str() {
        "run" => {
            let (left, right) = (target("left")?, target("right")?);
            let left_is_reference = options.get("left").is_some_and(|left| left == "reference");
            let mut differences = Vec::new();
            let mut wrong = Vec::new();
            for scenario in &set {
                let (a, b) = (transcript(&left, scenario)?, transcript(&right, scenario)?);
                if left_is_reference {
                    wrong.extend(play::unexpected(scenario, &a));
                }
                let found = compare::compare(&a, &b);
                eprintln!("{:<26} {:>2} steps, {:>2} differences  ({})", scenario.name, a.exchanges.len(), found.len(), scenario.summary);
                differences.extend(found);
            }
            let rows = match options.get("allowlist") {
                Some(path) => allowlist::parse(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)?,
                None => Vec::new(),
            };
            let ran: Vec<String> = set.iter().map(|scenario| scenario.name.clone()).collect();
            let verdict = allowlist::judge(&differences, &rows, &ran);
            let report = report(&verdict, &wrong, set.len());
            println!("{report}");
            if let Some(path) = options.get("report") {
                std::fs::write(path, &report).map_err(|e| format!("{path}: {e}"))?;
            }
            Ok(verdict.passed() && wrong.is_empty())
        }
        "record" => {
            let target = target("target")?;
            std::fs::create_dir_all(&golden).map_err(|e| e.to_string())?;
            for scenario in &set {
                let transcript = transcript(&target, scenario)?;
                let text = serde_json::to_string_pretty(&transcript).map_err(|e| e.to_string())?;
                let path = golden.join(format!("{}.json", scenario.name));
                std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
                eprintln!("recorded {}", path.display());
            }
            Ok(true)
        }
        "check-golden" => {
            let target = target("target")?;
            let mut differences: Vec<Difference> = Vec::new();
            for scenario in &set {
                let path = golden.join(format!("{}.json", scenario.name));
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let recorded: Transcript = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                differences.extend(compare::compare(&recorded, &transcript(&target, scenario)?));
            }
            println!("{}", report(&allowlist::judge(&differences, &[], &[]), &[], set.len()));
            Ok(differences.is_empty())
        }
        other => Err(format!("unknown command {other}")),
    }
}

fn report(verdict: &allowlist::Verdict, wrong: &[String], scenarios: usize) -> String {
    let mut out = format!(
        "### Gate B\n\n{scenarios} scenarios · {} unlisted differences · {} listed · {} stale rows · {} wrong scenario steps\n",
        verdict.unlisted.len(),
        verdict.listed.len(),
        verdict.stale.len(),
        wrong.len()
    );
    if !verdict.unlisted.is_empty() {
        out.push_str("\n| scenario | step | field | left | right |\n|---|---|---|---|---|\n");
        for d in &verdict.unlisted {
            out.push_str(&format!("| {} | {} | {} | {} | {} |\n", d.scenario, d.step, d.field, cell(&d.left), cell(&d.right)));
        }
    }
    for id in &verdict.stale {
        out.push_str(&format!("\nstale: {id} is listed and no longer happens\n"));
    }
    for line in wrong {
        out.push_str(&format!("\nwrong scenario: {line}\n"));
    }
    out
}

fn cell(text: &str) -> String {
    text.replace('\n', " ")
}
