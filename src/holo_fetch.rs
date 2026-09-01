//! Capability-checked, host-mediated HTTPS fetches for Component guests.

use hologram::space::NetworkEndpointScope;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub(crate) const FETCH_TARGET_MAX_BYTES: usize = 2 * 1024;
pub(crate) const FETCH_RESPONSE_MAX_BYTES: usize = 1024 * 1024;
pub(crate) const FETCH_DEADLINE: Duration = Duration::from_millis(1_500);
pub(crate) const FETCH_CONCURRENCY_MAX: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchResponse {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

pub(crate) trait FetchTransport: Send + Sync {
    fn get(&self, target: &str) -> Result<FetchResponse, &'static str>;
}

pub(crate) struct FetchRuntime {
    transport: Arc<dyn FetchTransport>,
    in_flight: AtomicUsize,
    concurrency_max: usize,
}

impl FetchRuntime {
    fn production() -> Self {
        Self::new(Arc::new(HttpsTransport), FETCH_CONCURRENCY_MAX)
    }

    pub(crate) fn new(transport: Arc<dyn FetchTransport>, concurrency_max: usize) -> Self {
        Self {
            transport,
            in_flight: AtomicUsize::new(0),
            concurrency_max,
        }
    }

    pub(crate) fn fetch(
        &self,
        scopes: &[NetworkEndpointScope],
        target: &str,
    ) -> Result<FetchResponse, &'static str> {
        if target.len() > FETCH_TARGET_MAX_BYTES {
            return Err("fetch target exceeds the host limit");
        }
        let endpoint = NetworkEndpointScope::parse(target)
            .map_err(|_| "fetch target is not a canonical HTTPS endpoint")?;
        if !scopes.iter().any(|scope| scope.admits(&endpoint)) {
            return Err("fetch target is outside the application's admitted endpoint set");
        }
        let _permit = self.acquire()?;

        // Re-check immediately before entering the transport. Keeping this check
        // on the operation boundary prevents a future mutable target or redirect
        // implementation from bypassing the admitted origin/path relation.
        if !scopes.iter().any(|scope| scope.admits(&endpoint)) {
            return Err("fetch target is outside the application's admitted endpoint set");
        }
        let response = self.transport.get(endpoint.as_str())?;
        if response.body.len() > FETCH_RESPONSE_MAX_BYTES {
            tracing::warn!(
                outcome = "response_too_large",
                "mediated Component fetch denied"
            );
            return Err("fetch response exceeds the host limit");
        }
        tracing::info!(
            outcome = "returned",
            status = response.status,
            response_bytes = response.body.len(),
            "mediated Component fetch completed"
        );
        Ok(response)
    }

    fn acquire(&self) -> Result<FetchPermit<'_>, &'static str> {
        let mut current = self.in_flight.load(Ordering::Acquire);
        loop {
            if current >= self.concurrency_max {
                return Err("fetch concurrency limit reached; retry later");
            }
            match self.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(FetchPermit(&self.in_flight)),
                Err(observed) => current = observed,
            }
        }
    }
}

pub(crate) fn default_fetch_runtime() -> Arc<FetchRuntime> {
    static RUNTIME: OnceLock<Arc<FetchRuntime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| Arc::new(FetchRuntime::production()))
        .clone()
}

struct FetchPermit<'a>(&'a AtomicUsize);

impl Drop for FetchPermit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct HttpsTransport;

impl FetchTransport for HttpsTransport {
    fn get(&self, target: &str) -> Result<FetchResponse, &'static str> {
        let permit = acquire_network_worker()?;
        let target = target.to_owned();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("hologram-fetch".to_owned())
            .spawn(move || {
                let result = https_get(&target);
                let _ = sender.send(result);
                drop(permit);
            })
            .map_err(|_| "fetch transport initialization failed")?;
        receiver
            .recv_timeout(FETCH_DEADLINE)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => "fetch request exceeded the host deadline",
                mpsc::RecvTimeoutError::Disconnected => "fetch request failed",
            })?
    }
}

fn https_get(target: &str) -> Result<FetchResponse, &'static str> {
    let (host, port) = target_authority(target)?;
    let resolved = (host, port)
        .to_socket_addrs()
        .map_err(|_| "fetch DNS resolution failed")?;
    let addresses = resolved
        .filter(|address| is_public_destination(address.ip()))
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("fetch destination is forbidden by host address policy");
    }

    // A fresh client per operation pins every connection attempt to this
    // checked resolution set. It cannot consult environment proxies, retain
    // cookies, reuse credentials, or automatically follow a redirect.
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .timeout(FETCH_DEADLINE)
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| "fetch transport initialization failed")?;
    let mut response = client
        .get(target)
        .send()
        .map_err(|_| "fetch request failed")?;
    if response
        .content_length()
        .is_some_and(|length| length > FETCH_RESPONSE_MAX_BYTES as u64)
    {
        return Err("fetch response exceeds the host limit");
    }
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .by_ref()
        .take((FETCH_RESPONSE_MAX_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| "fetch response read failed")?;
    if body.len() > FETCH_RESPONSE_MAX_BYTES {
        return Err("fetch response exceeds the host limit");
    }
    Ok(FetchResponse { status, body })
}

fn acquire_network_worker() -> Result<NetworkWorkerPermit, &'static str> {
    static WORKERS: AtomicUsize = AtomicUsize::new(0);
    let mut current = WORKERS.load(Ordering::Acquire);
    loop {
        if current >= FETCH_CONCURRENCY_MAX {
            return Err("fetch concurrency limit reached; retry later");
        }
        match WORKERS.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(NetworkWorkerPermit(&WORKERS)),
            Err(observed) => current = observed,
        }
    }
}

struct NetworkWorkerPermit(&'static AtomicUsize);

impl Drop for NetworkWorkerPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn target_authority(target: &str) -> Result<(&str, u16), &'static str> {
    let authority = target
        .strip_prefix("https://")
        .and_then(|rest| rest.split_once('/').map(|(authority, _)| authority))
        .ok_or("fetch target is not a canonical HTTPS endpoint")?;
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or("fetch target is not a canonical HTTPS endpoint")?;
    let port = port
        .parse()
        .map_err(|_| "fetch target is not a canonical HTTPS endpoint")?;
    Ok((host, port))
}

fn is_public_destination(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_v4(address),
        IpAddr::V6(address) => is_public_v6(address),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !(octets[0] == 0
        || octets[0] == 10
        || octets[0] == 127
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 169 && octets[1] == 254)
        || (octets[0] == 172 && (16..=31).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
        || (octets[0] == 192 && octets[1] == 168)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 224)
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    !((segments[0] & 0xe000) != 0x2000
        || address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] <= 0x01ff)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || segments[0] == 0x2002)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingTransport {
        targets: Mutex<Vec<String>>,
        response: FetchResponse,
    }

    impl FetchTransport for RecordingTransport {
        fn get(&self, target: &str) -> Result<FetchResponse, &'static str> {
            self.targets
                .lock()
                .expect("targets")
                .push(target.to_owned());
            Ok(self.response.clone())
        }
    }

    fn scope(value: &str) -> NetworkEndpointScope {
        NetworkEndpointScope::parse(value).expect("scope")
    }

    #[test]
    fn authorization_precedes_transport_and_obeys_segment_boundaries() {
        let transport = Arc::new(RecordingTransport {
            targets: Mutex::new(Vec::new()),
            response: FetchResponse {
                status: 200,
                body: b"ok".to_vec(),
            },
        });
        let runtime = FetchRuntime::new(transport.clone(), 1);
        let scopes = [scope("https://api.example.com:443/v1")];
        assert_eq!(
            runtime
                .fetch(&scopes, "https://api.example.com:443/v1/models")
                .expect("admitted fetch")
                .body,
            b"ok"
        );
        assert_eq!(
            runtime.fetch(&scopes, "https://api.example.com:443/v10"),
            Err("fetch target is outside the application's admitted endpoint set")
        );
        assert_eq!(transport.targets.lock().expect("targets").len(), 1);
    }

    #[test]
    fn diagnostics_never_repeat_sensitive_endpoint_values() {
        let secret = "https://secret.example.com:443/private";
        let runtime = FetchRuntime::new(
            Arc::new(RecordingTransport {
                targets: Mutex::new(Vec::new()),
                response: FetchResponse {
                    status: 200,
                    body: Vec::new(),
                },
            }),
            1,
        );
        let error = runtime.fetch(&[], secret).expect_err("denied");
        assert!(!error.contains(secret));
        assert!(!error.contains("secret.example.com"));
    }

    #[test]
    fn concurrency_and_response_limits_apply_outside_the_transport() {
        let scope = scope("https://api.example.com:443/v1");
        let target = scope.as_str();
        let full = FetchRuntime::new(
            Arc::new(RecordingTransport {
                targets: Mutex::new(Vec::new()),
                response: FetchResponse {
                    status: 200,
                    body: Vec::new(),
                },
            }),
            0,
        );
        assert_eq!(
            full.fetch(std::slice::from_ref(&scope), target),
            Err("fetch concurrency limit reached; retry later")
        );

        let oversized = FetchRuntime::new(
            Arc::new(RecordingTransport {
                targets: Mutex::new(Vec::new()),
                response: FetchResponse {
                    status: 200,
                    body: vec![0; FETCH_RESPONSE_MAX_BYTES + 1],
                },
            }),
            1,
        );
        assert_eq!(
            oversized.fetch(std::slice::from_ref(&scope), target),
            Err("fetch response exceeds the host limit")
        );
    }

    #[test]
    fn private_reserved_and_mapped_addresses_are_forbidden() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.168.0.1",
            "192.88.99.1",
            "198.18.0.1",
            "203.0.113.1",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2001::1",
            "2002:7f00:1::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(
                !is_public_destination(address.parse().expect("address")),
                "{address}"
            );
        }
        assert!(is_public_destination("1.1.1.1".parse().expect("public v4")));
        assert!(is_public_destination(
            "2606:4700:4700::1111".parse().expect("public v6")
        ));
    }
}
