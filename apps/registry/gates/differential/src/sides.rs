//! Starting and stopping the two registries. Every scenario gets a fresh,
//! empty one on each side, so scenarios cannot disturb each other.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum Target {
    /// A container image: the reference, or later the product's own image.
    Image(String),
    /// A `hologram` binary, started with a configuration of its own.
    Binary(PathBuf),
    /// Something already running. It is not fresh per scenario: for debugging.
    Url(String),
}

impl Target {
    /// `reference`, `image:<ref>`, `binary:<path>`, or a URL.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text == "reference" {
            let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../reference.env");
            let pin = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
            let image = pin
                .lines()
                .find_map(|line| line.strip_prefix("REGISTRY_REF="))
                .ok_or_else(|| format!("{file} has no REGISTRY_REF"))?;
            // `name:tag@sha256:…` reads well; docker wants `name@sha256:…`.
            let image = match (image.split_once('@'), image.split_once(':')) {
                (Some((_, digest)), Some((name, _))) => format!("{name}@{digest}"),
                _ => return Err(format!("{file}: the reference must be pinned by digest")),
            };
            return Ok(Self::Image(image));
        }
        if let Some(image) = text.strip_prefix("image:") {
            return Ok(Self::Image(image.to_owned()));
        }
        if let Some(path) = text.strip_prefix("binary:") {
            return Ok(Self::Binary(PathBuf::from(path)));
        }
        if text.starts_with("http://") || text.starts_with("https://") {
            return Ok(Self::Url(text.trim_end_matches('/').to_owned()));
        }
        Err(format!("unknown target {text:?}: use reference, image:<ref>, binary:<path> or a URL"))
    }
}

pub struct Running {
    pub base: String,
    kind: Kind,
}

enum Kind {
    Container(String),
    Process(Child, PathBuf),
    External,
}

/// The reference's own environment names; the product reads the same ones.
fn environment(needs: &[String]) -> Result<Vec<(&'static str, &'static str)>, String> {
    let mut out = Vec::new();
    for need in needs {
        match need.as_str() {
            "delete" => out.push(("REGISTRY_STORAGE_DELETE_ENABLED", "true")),
            "readonly" => out.push(("REGISTRY_STORAGE_MAINTENANCE_READONLY_ENABLED", "true")),
            other => return Err(format!("no settings variant for need {other:?} yet")),
        }
    }
    Ok(out)
}

pub fn start(target: &Target, needs: &[String]) -> Result<Running, String> {
    let environment = environment(needs)?;
    let running = match target {
        Target::Url(base) => Running { base: base.clone(), kind: Kind::External },
        Target::Image(image) => {
            let mut command = Command::new("docker");
            command.args(["run", "-d", "--rm", "-p", "127.0.0.1::5000", "--tmpfs", "/var/lib/registry"]);
            for (name, value) in &environment {
                command.args(["-e", &format!("{name}={value}")]);
            }
            let id = output(command.arg(image))?;
            let port = output(Command::new("docker").args(["port", &id, "5000/tcp"]))?;
            let port = port.lines().next().and_then(|line| line.rsplit(':').next()).ok_or("docker port printed nothing")?.to_owned();
            Running { base: format!("http://127.0.0.1:{port}"), kind: Kind::Container(id) }
        }
        Target::Binary(binary) => {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!("gate-b-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map_err(|e| e.to_string())?.port();
            let dir = |name: &str| root.join(name).display().to_string().replace('\\', "/");
            let config = format!(
                "schema_version = 2\n\n[paths]\nconfig_dir = \"{}\"\ndata_dir = \"{}\"\nstate_dir = \"{}\"\ncache_dir = \"{}\"\n\n[server]\nlisten = \"127.0.0.1:{port}\"\n\n[modules]\nenabled = [\"dev.hologram.live.system\", \"dev.hologram.live.oci\"]\n",
                dir("config"), dir("data"), dir("state"), dir("cache"),
            );
            let path = root.join("live.toml");
            std::fs::write(&path, config).map_err(|e| e.to_string())?;
            // The configuration is named on the command line and every path
            // is inside `root`: no configuration outside the gate is read.
            let child = Command::new(binary)
                .arg("--config")
                .arg(&path)
                .arg("serve")
                .envs(environment.iter().copied())
                .env("HOME", &root)
                .env("USERPROFILE", &root)
                .env("HOLOGRAM_CONFIG_DIR", root.join("config"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("start {}: {e}", binary.display()))?;
            Running { base: format!("http://127.0.0.1:{port}"), kind: Kind::Process(child, root) }
        }
    };
    running.wait_ready()?;
    Ok(running)
}

impl Running {
    fn wait_ready(&self) -> Result<(), String> {
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(5)).build().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            // 401 is ready too: a registry that wants a login is up.
            if client.get(format!("{}/v2/", self.base)).send().is_ok() {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("{} did not answer /v2/ within a minute", self.base));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        match &mut self.kind {
            Kind::Container(id) => {
                let _ = Command::new("docker").args(["rm", "-f", id]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
            Kind::Process(child, root) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(root);
            }
            Kind::External => {}
        }
    }
}

fn output(command: &mut Command) -> Result<String, String> {
    let out = command.output().map_err(|e| format!("{command:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{command:?}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
