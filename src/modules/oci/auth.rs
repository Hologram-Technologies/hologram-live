//! Login from a password file (`auth.htpasswd`), as the reference registry
//! does it (distribution v3.1.1, `registry/auth/htpasswd`).
//!
//! HTTP Basic against an htpasswd file of bcrypt entries. The file is read
//! again when its modification time changes, as the reference checks it on
//! every request. A missing file is created with one user, `docker`, and a
//! random password written to the log once, as the reference does. An unknown
//! user costs one bcrypt against a dummy hash, so a user name does not show in
//! the time an answer takes. A good login is remembered for 60 s under a keyed
//! hash of the password, so a push of 40 layers pays one bcrypt, not 40;
//! failures are never remembered.

use super::error::{ErrorCode, OciError};
use super::path::Scope;
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

/// How long a good login is remembered.
const REMEMBER: Duration = Duration::from_mins(1);
/// The cost the reference provisions a missing file with (Go's default).
const PROVISION_COST: u32 = 10;

/// (user, keyed hash of the password) → when bcrypt last said yes, and for
/// which version of the file.
type Remembered = HashMap<(String, [u8; 32]), (Instant, SystemTime)>;

pub struct Htpasswd {
    path: PathBuf,
    realm: String,
    loaded: Mutex<Option<Loaded>>,
    remembered: Mutex<Remembered>,
    /// Random per process: the hash cannot be computed outside it.
    key: [u8; 32],
    /// Checked for users the file does not name. The reference's dummy costs
    /// 5, cheaper than a real entry, so the time of an answer tells whether a
    /// user exists; this one costs what a provisioned entry costs.
    dummy: String,
}

struct Loaded {
    modified: SystemTime,
    entries: Arc<HashMap<String, String>>,
}

/// Why a request is not let in.
#[derive(Debug)]
pub enum Denied {
    /// No credentials, or wrong ones: 401 with the Basic challenge.
    Challenge,
    /// The file cannot be read or parsed: the reference answers a bare 400.
    Broken(String),
}

impl Htpasswd {
    /// The login the reference's settings ask for, the file created when it
    /// is missing.
    ///
    /// # Errors
    ///
    /// The file cannot be created, or no random bytes are available.
    pub fn open(path: PathBuf, realm: String) -> std::io::Result<Self> {
        provision(&path)?;
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).map_err(std::io::Error::other)?;
        let dummy = bcrypt::hash(crate::util::hex(&key), PROVISION_COST).map_err(std::io::Error::other)?;
        Ok(Self {
            path,
            realm,
            loaded: Mutex::new(None),
            remembered: Mutex::new(HashMap::new()),
            key,
            dummy,
        })
    }

    /// The user a request logs in as.
    ///
    /// # Errors
    ///
    /// [`Denied::Challenge`] for no or wrong credentials, [`Denied::Broken`]
    /// when the file cannot be read.
    pub async fn authenticate(self: &Arc<Self>, headers: &HeaderMap) -> Result<String, Denied> {
        let (user, password) = basic(headers).ok_or(Denied::Challenge)?;
        let (entries, version) = self.entries()?;
        let remembered = (user.clone(), *blake3::keyed_hash(&self.key, password.as_bytes()).as_bytes());
        {
            let seen = self.remembered.lock().unwrap_or_else(PoisonError::into_inner);
            // Only for the file it was checked against: a changed file drops it.
            if seen
                .get(&remembered)
                .is_some_and(|(at, checked)| at.elapsed() < REMEMBER && *checked == version)
            {
                return Ok(user);
            }
        }
        let known = entries.get(&user).cloned();
        let hash = known.clone().unwrap_or_else(|| self.dummy.clone());
        // bcrypt is slow on purpose: off the async threads.
        let matches = tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash).unwrap_or(false))
            .await
            .unwrap_or(false);
        if matches && known.is_some() {
            let mut seen = self.remembered.lock().unwrap_or_else(PoisonError::into_inner);
            seen.retain(|_, (at, _)| at.elapsed() < REMEMBER);
            seen.insert(remembered, (Instant::now(), version));
            return Ok(user);
        }
        tracing::info!(user = user.as_str(), "user failed to authenticate");
        Err(Denied::Challenge)
    }

    /// The file's entries and the version they are from, read again when the
    /// file has changed since.
    fn entries(&self) -> Result<(Arc<HashMap<String, String>>, SystemTime), Denied> {
        let broken = |error: std::io::Error| Denied::Broken(format!("{}: {error}", self.path.display()));
        let modified = std::fs::metadata(&self.path).and_then(|meta| meta.modified()).map_err(broken)?;
        let mut loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(current) = loaded.as_ref().filter(|current| current.modified == modified) {
            return Ok((current.entries.clone(), modified));
        }
        let text = std::fs::read_to_string(&self.path).map_err(broken)?;
        let entries = Arc::new(parse(&text).map_err(Denied::Broken)?);
        *loaded = Some(Loaded { modified, entries: entries.clone() });
        drop(loaded);
        // A changed file may have removed or changed a user.
        self.remembered.lock().unwrap_or_else(PoisonError::into_inner).clear();
        Ok((entries, modified))
    }

    /// The answer for a request that is not let in.
    pub fn refuse(&self, denied: &Denied, method: &Method, scope: &Scope, query: Option<&str>) -> Response {
        match denied {
            Denied::Challenge => {
                let challenge = format!("Basic realm={}", go_quote(&self.realm));
                let mut response = OciError::new(ErrorCode::Unauthorized)
                    .with_detail_even_null(access_records(method, scope, query))
                    .into_response();
                if let Ok(value) = HeaderValue::from_bytes(challenge.as_bytes()) {
                    response.headers_mut().insert(WWW_AUTHENTICATE, value);
                }
                response
            }
            Denied::Broken(reason) => {
                // "Just return a bad request with no information", as the reference.
                tracing::error!(reason = reason.as_str(), "error checking authorization");
                StatusCode::BAD_REQUEST.into_response()
            }
        }
    }
}

/// `Authorization: Basic …`, read as Go's `Request.BasicAuth` reads it.
fn basic(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, encoded) = value.split_at_checked(6)?;
    if !scheme.eq_ignore_ascii_case("basic ") {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (user, password) = decoded.split_once(':')?;
    Some((user.to_owned(), password.to_owned()))
}

/// The reference's parser: blank lines and `#` comments skipped, each other
/// line `user:hash`, split at the first colon; a later line wins.
fn parse(text: &str) -> Result<HashMap<String, String>, String> {
    let mut entries = HashMap::new();
    for (number, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (user, hash) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("htpasswd: invalid entry at line {}: {line:?}", number + 1))?;
        entries.insert(user.to_owned(), hash.to_owned());
    }
    Ok(entries)
}

/// As the reference: a missing file is created, directory 0700 and file
/// 0600, holding `docker:<bcrypt>` of a random password that is logged once.
fn provision(path: &Path) -> std::io::Result<()> {
    match std::fs::metadata(path) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        Err(_) => {}
    }
    // As Go's MkdirAll(dir, 0o700): the missing directories are made 0700,
    // and a directory that is already there is left as it is.
    if let Some(directory) = path.parent().filter(|directory| !directory.as_os_str().is_empty()) {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
    }
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret).map_err(std::io::Error::other)?;
    let password = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret);
    let hash = bcrypt::hash(&password, PROVISION_COST).map_err(std::io::Error::other)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        // Another process provisioned it first: theirs stands.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error),
    };
    std::io::Write::write_all(&mut file, format!("docker:{hash}").as_bytes())?;
    tracing::warn!(user = "docker", password = password.as_str(), "htpasswd is missing, provisioning with default user");
    Ok(())
}

/// What the request asks to do, as the reference names it in the 401's
/// `detail`: pull for reads, pull and push for writes, delete; pull on the
/// source of a mount; `registry:catalog:*` for the catalogue; nothing for the
/// base route.
fn access_records(method: &Method, scope: &Scope, query: Option<&str>) -> Value {
    let record = |kind: &str, name: &str, action: &str| {
        json!({ "Type": kind, "Class": "", "Name": name, "Action": action })
    };
    let repo = match scope {
        Scope::Catalog => return json!([record("registry", "catalog", "*")]),
        // Go encodes the nil slice as `"detail": null` (gate B, `13-auth`).
        Scope::Base => return Value::Null,
        Scope::Repository(name) => name.as_str(),
    };
    let for_method = |method: &Method| -> Vec<Value> {
        match *method {
            Method::GET | Method::HEAD => vec![record("repository", repo, "pull")],
            Method::POST | Method::PUT | Method::PATCH => {
                vec![record("repository", repo, "pull"), record("repository", repo, "push")]
            }
            Method::DELETE => vec![record("repository", repo, "delete")],
            _ => Vec::new(),
        }
    };
    let mut records = for_method(method);
    // As Go's `FormValue("from")`: decoded, and an empty value is no mount.
    if let Some(from) = super::path::query_param(query, "from").filter(|from| !from.is_empty()) {
        records.push(record("repository", &from, "pull"));
    }
    if records.is_empty() {
        Value::Null
    } else {
        Value::Array(records)
    }
}

/// Go's `%q` (`strconv.Quote`), for the realm in the challenge: `"` and `\`
/// escaped, control characters written as escapes, printable text as it is.
fn go_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control.is_control() && u32::from(control) < 0x80 => {
                let _ = write!(out, "\\x{:02x}", u32::from(control));
            }
            control if control.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
