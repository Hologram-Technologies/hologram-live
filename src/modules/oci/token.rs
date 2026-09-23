//! `auth.token`: bearer tokens, the way every public registry does login.
//!
//! The reference *validates* tokens and expects a separate token server to
//! issue them (`registry/auth/token`): `realm` is where a client goes for one,
//! `service` is the audience, `issuer` and `jwks` say who may sign. This
//! registry validates the same way.
//!
//! It also issues them, which the reference does not, because a single binary
//! that wants anonymous pull and authenticated push has nowhere else to go: a
//! password file protects reads as well as writes, and a client that sees
//! `/v2/` answer 200 never learns to log in (skopeo and podman push nothing
//! at all against such a host). With `auth.token.local`, `/auth/token` gives
//! an anonymous client a pull-only token and a client with a password
//! everything the password file allows. That difference is in
//! `apps/registry/DIFFERENCES.md`.

use super::auth::{basic, Denied};
use super::error::{ErrorCode, OciError};
use super::path::Scope;
use axum::http::header::{HeaderMap, AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

/// How long an issued token lives. The reference's own token server uses a
/// few minutes; a push of many layers must finish inside one.
const LIFETIME_SECONDS: u64 = 15 * 60;

/// One entry of a token's `access` claim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Access {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub actions: Vec<String>,
}

/// What a token says, as the distribution token specification defines it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub exp: u64,
    pub nbf: u64,
    pub iat: u64,
    pub jti: String,
    #[serde(default)]
    pub access: Vec<Access>,
}

/// The registry's side of token authentication.
pub struct TokenAuth {
    /// Where a client goes for a token (`auth.token.realm`).
    realm: String,
    /// The audience this registry accepts (`auth.token.service`).
    service: String,
    /// Who may sign (`auth.token.issuer`).
    issuer: String,
    /// The keys that verify a signature.
    keys: Vec<jsonwebtoken::DecodingKey>,
    /// Set when this registry issues its own tokens.
    local: Option<Arc<LocalIssuer>>,
}

impl TokenAuth {
    /// Validate only, against keys from a JWKS file: the reference's shape.
    ///
    /// # Errors
    ///
    /// The JWKS cannot be read or holds no key this registry can use.
    pub fn validating(
        realm: String,
        service: String,
        issuer: String,
        jwks: &Path,
    ) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(jwks)?;
        let keys = keys_from_jwks(&text).map_err(std::io::Error::other)?;
        if keys.is_empty() {
            return Err(std::io::Error::other(format!(
                "{}: no key this registry can verify with (EC P-256 or RSA)",
                jwks.display()
            )));
        }
        Ok(Self {
            realm,
            service,
            issuer,
            keys,
            local: None,
        })
    }

    /// Validate tokens this registry issues itself (`auth.token.local`).
    ///
    /// # Errors
    ///
    /// The signing key cannot be created or read.
    pub fn local(
        realm: String,
        service: String,
        issuer: String,
        key_file: &Path,
        passwords: Option<Arc<super::auth::Htpasswd>>,
    ) -> std::io::Result<Self> {
        let issuer_side = LocalIssuer::open(key_file, issuer.clone(), service.clone(), passwords)?;
        let keys = vec![issuer_side.verifying_key()];
        Ok(Self {
            realm,
            service,
            issuer,
            keys,
            local: Some(Arc::new(issuer_side)),
        })
    }

    /// The issuer, when this registry is its own.
    #[must_use]
    pub fn issuer_side(&self) -> Option<Arc<LocalIssuer>> {
        self.local.clone()
    }

    /// Who the bearer is, if the token lets them do this.
    ///
    /// # Errors
    ///
    /// [`Denied::Challenge`] for a missing, unreadable, expired or
    /// insufficient token: the client is told where to get a better one.
    pub fn authenticate(
        &self,
        headers: &HeaderMap,
        method: &Method,
        scope: &Scope,
        query: Option<&str>,
    ) -> Result<String, Denied> {
        let token = bearer(headers).ok_or(Denied::Challenge)?;
        let claims = self.verify(&token).ok_or(Denied::Challenge)?;
        let wanted = required(method, scope, query);
        let granted: BTreeSet<(String, String, String)> = claims
            .access
            .iter()
            .flat_map(|access| {
                access.actions.iter().map(move |action| {
                    (access.kind.clone(), access.name.clone(), action.clone())
                })
            })
            .collect();
        let covered = wanted.iter().all(|(kind, name, action)| {
            granted.contains(&(kind.clone(), name.clone(), action.clone()))
                || granted.contains(&(kind.clone(), name.clone(), "*".to_owned()))
        });
        if covered {
            Ok(claims.sub)
        } else {
            Err(Denied::Challenge)
        }
    }

    fn verify(&self, token: &str) -> Option<Claims> {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.set_audience(&[self.service.as_str()]);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.leeway = 30;
        self.keys.iter().find_map(|key| {
            jsonwebtoken::decode::<Claims>(token, key, &validation)
                .ok()
                .map(|data| data.claims)
        })
    }

    /// The answer for a request without a good enough token: where to get one,
    /// and for what.
    #[must_use]
    pub fn refuse(&self, method: &Method, scope: &Scope, query: Option<&str>) -> Response {
        let wanted = required(method, scope, query);
        let mut scopes: Vec<String> = Vec::new();
        for (kind, name, _) in &wanted {
            let actions: Vec<&str> = wanted
                .iter()
                .filter(|(k, n, _)| k == kind && n == name)
                .map(|(_, _, action)| action.as_str())
                .collect();
            let entry = format!("{kind}:{name}:{}", actions.join(","));
            if !scopes.contains(&entry) {
                scopes.push(entry);
            }
        }
        let challenge = if scopes.is_empty() {
            format!(
                "Bearer realm=\"{}\",service=\"{}\"",
                self.realm, self.service
            )
        } else {
            format!(
                "Bearer realm=\"{}\",service=\"{}\",scope=\"{}\"",
                self.realm,
                self.service,
                scopes.join(" ")
            )
        };
        let mut response = OciError::new(ErrorCode::Unauthorized)
            .with_detail_even_null(super::auth::access_detail(method, scope, query))
            .into_response();
        if let Ok(value) = HeaderValue::from_str(&challenge) {
            response.headers_mut().insert(WWW_AUTHENTICATE, value);
        }
        response
    }
}

/// The registry issuing its own tokens.
pub struct LocalIssuer {
    encoding: jsonwebtoken::EncodingKey,
    kid: String,
    public_point: Vec<u8>,
    issuer: String,
    service: String,
    passwords: Option<Arc<super::auth::Htpasswd>>,
}

impl LocalIssuer {
    /// Read the signing key, or make one and keep it 0600 beside the volume's
    /// other state, so tokens stay valid across restarts.
    fn open(
        key_file: &Path,
        issuer: String,
        service: String,
        passwords: Option<Arc<super::auth::Htpasswd>>,
    ) -> std::io::Result<Self> {
        if let Some(directory) = key_file.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let pkcs8 = match std::fs::read(key_file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let random = ring::rand::SystemRandom::new();
                let generated = ring::signature::EcdsaKeyPair::generate_pkcs8(
                    &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                    &random,
                )
                .map_err(|error| std::io::Error::other(format!("generate a signing key: {error}")))?;
                let bytes = generated.as_ref().to_vec();
                write_private(key_file, &bytes)?;
                bytes
            }
            Err(error) => return Err(error),
        };
        let pair = ring::signature::EcdsaKeyPair::from_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &pkcs8,
            &ring::rand::SystemRandom::new(),
        )
        .map_err(|error| std::io::Error::other(format!("read the signing key: {error}")))?;
        let public_point = {
            use ring::signature::KeyPair as _;
            pair.public_key().as_ref().to_vec()
        };
        let encoding = jsonwebtoken::EncodingKey::from_ec_der(&pkcs8);
        let kid = key_id(&public_point);
        Ok(Self {
            encoding,
            kid,
            public_point,
            issuer,
            service,
            passwords,
        })
    }

    fn verifying_key(&self) -> jsonwebtoken::DecodingKey {
        let spki = spki_p256(&self.public_point);
        jsonwebtoken::DecodingKey::from_ec_der(&spki[spki.len() - 65..])
    }

    /// The key set, so anything else can verify what this registry signed.
    #[must_use]
    pub fn jwks(&self) -> String {
        let (x, y) = self.public_point[1..].split_at(32);
        serde_json::json!({
            "keys": [{
                "kty": "EC",
                "crv": "P-256",
                "use": "sig",
                "alg": "ES256",
                "kid": self.kid,
                "x": base64_url(x),
                "y": base64_url(y),
            }]
        })
        .to_string()
    }

    /// Mint a token for what the client asked for and may have.
    ///
    /// Anonymous gets the read half of the scopes it asked for; a password
    /// that the file accepts gets all of them. An empty `access` is a valid
    /// token: that is how a client learns it is anonymous.
    pub async fn issue(&self, headers: &HeaderMap, scopes: &str) -> Result<String, Denied> {
        let (subject, authenticated) = match basic(headers) {
            Some((user, password)) => match &self.passwords {
                Some(file) => {
                    let name = file.check(&user, &password).await?;
                    (name, true)
                }
                // No password file: nobody can be more than anonymous.
                None => return Err(Denied::Challenge),
            },
            None => ("anonymous".to_owned(), false),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let access = scopes
            .split(' ')
            .filter(|scope| !scope.is_empty())
            .filter_map(|scope| parse_scope(scope, authenticated))
            .collect::<Vec<_>>();
        let claims = Claims {
            iss: self.issuer.clone(),
            sub: subject,
            aud: self.service.clone(),
            exp: now + LIFETIME_SECONDS,
            nbf: now.saturating_sub(5),
            iat: now,
            jti: format!("{now}-{}", self.kid),
            access,
        };
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(self.kid.clone());
        jsonwebtoken::encode(&header, &claims, &self.encoding)
            .map_err(|error| Denied::Broken(format!("sign a token: {error}")))
    }
}

/// A requested scope, cut down to what the client may have.
fn parse_scope(scope: &str, authenticated: bool) -> Option<Access> {
    let mut parts = scope.split(':');
    let kind = parts.next()?.to_owned();
    let name = parts.next()?.to_owned();
    let actions = parts.next().unwrap_or_default();
    let wanted: Vec<String> = actions
        .split(',')
        .filter(|action| !action.is_empty())
        .map(str::to_owned)
        .collect();
    let allowed: Vec<String> = if authenticated {
        wanted
    } else {
        // Anonymous reads. The catalogue is a read of the whole registry, and
        // the reference's own token server gives it to nobody by default.
        wanted
            .into_iter()
            .filter(|action| action == "pull" && kind == "repository")
            .collect()
    };
    if allowed.is_empty() {
        return None;
    }
    Some(Access {
        kind,
        name,
        actions: allowed,
    })
}

/// What this request needs, as (type, name, action).
fn required(method: &Method, scope: &Scope, query: Option<&str>) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let repo = match scope {
        Scope::Base => return out,
        Scope::Catalog => {
            out.push((
                "registry".to_owned(),
                "catalog".to_owned(),
                "*".to_owned(),
            ));
            return out;
        }
        Scope::Repository(name) => name.clone(),
    };
    match *method {
        Method::GET | Method::HEAD => out.push(("repository".to_owned(), repo.clone(), "pull".to_owned())),
        Method::POST | Method::PUT | Method::PATCH => {
            out.push(("repository".to_owned(), repo.clone(), "pull".to_owned()));
            out.push(("repository".to_owned(), repo.clone(), "push".to_owned()));
        }
        Method::DELETE => out.push((
            "repository".to_owned(),
            repo.clone(),
            "delete".to_owned(),
        )),
        _ => {}
    }
    if let Some(from) = super::path::query_param(query, "from").filter(|from| !from.is_empty()) {
        out.push(("repository".to_owned(), from, "pull".to_owned()));
    }
    out
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_at_checked(7)?;
    scheme
        .eq_ignore_ascii_case("bearer ")
        .then(|| token.trim().to_owned())
}

/// The keys of a JWKS this registry can verify with.
fn keys_from_jwks(text: &str) -> Result<Vec<jsonwebtoken::DecodingKey>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let mut keys = Vec::new();
    for key in value["keys"].as_array().into_iter().flatten() {
        if key["kty"] == "EC" && key["crv"] == "P-256" {
            let (Some(x), Some(y)) = (key["x"].as_str(), key["y"].as_str()) else {
                continue;
            };
            let (Ok(x), Ok(y)) = (base64_url_decode(x), base64_url_decode(y)) else {
                continue;
            };
            if x.len() == 32 && y.len() == 32 {
                let mut point = vec![0x04];
                point.extend_from_slice(&x);
                point.extend_from_slice(&y);
                keys.push(jsonwebtoken::DecodingKey::from_ec_der(&point));
            }
        }
    }
    Ok(keys)
}

/// `SubjectPublicKeyInfo` for a P-256 point: the algorithm is fixed, so the
/// prefix is a constant and the point is the rest.
fn spki_p256(point: &[u8]) -> Vec<u8> {
    const PREFIX: [u8; 26] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    let mut out = PREFIX.to_vec();
    out.extend_from_slice(point);
    out
}

/// A short, stable name for the key, as JWKS `kid`.
fn key_id(point: &[u8]) -> String {
    let digest = blake3::hash(point);
    base64_url(&digest.as_bytes()[..12])
}

/// A Unix time as RFC 3339, which is what a token answer carries as
/// `issued_at` (Howard Hinnant's `civil_from_days`, the other way round).
#[must_use]
pub fn rfc3339(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let time = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 { shifted + 3 } else { shifted - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn base64_url_decode(text: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text)
}

/// A key file is the server's alone.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes)
}

/// `StatusCode` is used by the routes this module serves.
pub const UNAUTHORIZED: StatusCode = StatusCode::UNAUTHORIZED;
