# P6. Login and TLS

> Task level. **Steps to be written before the phase starts** (day 11).
> Entry: P5 T1 (setting names). Engineer B. 3.5 days (days 12 to 16).

**Goal:** the password file and the certificate an operator already has work through the reference's setting names. Exit: `apps/registry/gates/clients/login.sh` and `tls.sh` green against the image; the two remaining compose files pass.

**Files:** `src/modules/oci/auth.rs` (300 lines), `src/tls.rs` (350), `src/server.rs` (+20), `src/config.rs` (+30).

---

## Task 1: htpasswd

**Requirements:** FR-007. **Files:** create `src/modules/oci/auth.rs`; modify `Cargo.toml` (`bcrypt`), `DEPENDENCIES.md`, `src/modules/oci/mod.rs` (one layer).

**Interfaces:**
- `pub struct Htpasswd { path: PathBuf, realm: String, state: RwLock<Loaded> }`; `Loaded { modified: SystemTime, entries: HashMap<String, String> }`
- `Htpasswd::check(&self, user: &str, password: &str) -> bool`: run inside `spawn_blocking` (bcrypt is slow by design)
- A `tower` layer on the module router, inside the span layer and outside the handler: no `Authorization` or a bad one → 401 `UNAUTHORIZED` with `WWW-Authenticate: Basic realm="<realm>"` (exact header and body from gate B scenario `auth-challenge`)
- A small cache: `HashMap<(user, blake3 keyed hash of password), Instant>`, 60 s, cleared when the file changes; the key is random per process. A `docker push` of 40 layers then pays one bcrypt, not 40
- The file is re-read when its modification time changes, checked at most once per second (closes CHK024)
- No `auth` configured → anonymous, as the reference

Step 1 of the step-level plan: confirm `bcrypt` is pure Rust (`cargo tree -p bcrypt` shows no `-sys`), supports `$2y$` (what `htpasswd -B` writes) and `$2a$`, `$2b$`.

New gate B scenarios written here: `auth-challenge`, `auth-good`, `auth-bad-password`, `auth-unknown-user`, `auth-base-only` (is `GET /v2/` itself challenged?), `auth-missing-htpasswd` (D6: what the reference does when the file does not exist).

**Done when:** `login.sh` green (good login, bad password refused, anonymous push refused with the Basic challenge); the six scenarios show zero unlisted differences; non-bcrypt entries in the file are refused the way the reference refuses them (scenario `auth-md5-entry`).

**Traps:**
- Compare with the cache only after a successful bcrypt. Never cache failures.
- A timing difference between "unknown user" and "bad password" leaks user names. Run one bcrypt against a fixed dummy hash for unknown users.
- If `auth-missing-htpasswd` shows the reference creating the file with a random password, do the same and log it once, the same way. It is drop-in behaviour, however odd.

## Task 2: TLS listener

**Requirements:** FR-008. **Files:** create `src/tls.rs`; modify `src/server.rs` (R13), `Cargo.toml` (`tokio-rustls`, `rustls-pemfile`; `hyper-util` features `server-auto`, `service`, `server-graceful`), `DEPENDENCIES.md`.

**Interfaces:**
- `pub struct TlsSettings { certificate: PathBuf, key: PathBuf, minimum: TlsVersion }`
- `pub async fn serve(listener: TcpListener, tls: Option<TlsSettings>, router: Router, shutdown: impl Future<Output = ()>) -> Result<()>`: replaces the `axum::serve` call. With `None` it behaves exactly as today (plain, h1 and h2c). With `Some`: an accept loop; **each handshake runs in its own task** with a 10 s timeout; then `hyper_util::server::conn::auto::Builder` serves the router on the TLS stream; ALPN offers `h2` and `http/1.1`
- Crypto provider: `ring`, already installed by `crate::util::install_crypto_provider`

**Done when:** `tls.sh` green: two containers on one Docker network, a throwaway CA installed in the client, `docker login`, `push`, `pull` by DNS name with no `insecure-registries`; a unit test opens 200 connections that never complete the handshake and asserts a 201st, well-behaved, connection is served within 1 s; `http.tls.minimumtls: tls1.3` refuses a TLS 1.2 client.

**Traps:**
- Do not implement axum's `Listener` trait around a TLS acceptor: `accept` would run one handshake at a time, and one slow client would stall every new connection.
- Graceful shutdown must drain TLS connections too; use `hyper_util::server::graceful::GracefulShutdown`.
- The certificate file may hold a chain; send all of it. The key may be PKCS#8, PKCS#1 or SEC1; `rustls-pemfile` reads all three.

## Task 3: The admin listener under TLS (ADR-028, second half)

**Requirements:** FR-021, FR-014 (reachability). **Files:** `src/server.rs`, `src/cli/oci.rs`.

The loopback gRPC listener from P5 T3 stays plain when TLS is on. So `docker exec registry hologram oci verify` needs no certificate, no hostname and no trust store. The CLI, when `--registry-config` or `HOLOGRAM_REGISTRY_CONFIG` is set or `argv[0]` is `registry`, dials `http://127.0.0.1:5001` with the token from `<root>/live/state/admin-token`.

**Done when:** a test in `tls.sh`: with TLS on, `docker exec <c> hologram oci verify --json` exits 0 (once P8 T3 lands; until then it asserts the connection and the "not built yet" answer); port 5001 is not reachable from the client container.

## Task 4: The other two compose files

**Requirements:** FR-001, SC-001. **Files:** `apps/registry/gates/compose/tls.yml`, `apps/registry/gates/compose/htpasswd.yml`; `gates.yml`.

Turn on the TLS and htpasswd variants in `compose-swap.sh`.

**Done when:** all three documented compose files pass with one line changed, in CI.

---

## Phase exit

`login.sh`, `tls.sh`, three compose files green. Closes FR-007, FR-008, FR-001, SC-001.
