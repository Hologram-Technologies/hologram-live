//! Play one scenario against one registry and write down what it answered.

use crate::normalise::{normalise, RawExchange, Transcript};
use crate::scenario::{Scenario, StepSpec};
use reqwest::blocking::Client;
use reqwest::Method;
use std::collections::BTreeMap;
use std::time::Duration;

pub fn client() -> Result<Client, String> {
    Client::builder()
        // A registry that answers 307 and one that answers 200 are different;
        // a client that follows redirects hides it.
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

pub fn play(scenario: &Scenario, base: &str, client: &Client) -> Result<Transcript, String> {
    let mut captured = BTreeMap::new();
    let mut exchanges = Vec::new();
    for step in &scenario.steps {
        exchanges.push(normalise(send(scenario, step, base, client, &mut captured)?, base));
    }
    // The state both sides are left in, read through the API.
    let mut state = Vec::new();
    if scenario.state.catalog {
        state.push(("state:catalog".to_owned(), "/v2/_catalog".to_owned()));
    }
    for repo in &scenario.state.tags {
        let repo = scenario.expand(repo, &captured)?;
        state.push((format!("state:tags:{repo}"), format!("/v2/{repo}/tags/list")));
    }
    for (id, path) in state {
        let step = StepSpec {
            id,
            method: "GET".to_owned(),
            path: Some(path),
            url: None,
            headers: BTreeMap::new(),
            query: BTreeMap::new(),
            body: None,
            capture: BTreeMap::new(),
            expect_status: None,
        };
        exchanges.push(normalise(send(scenario, &step, base, client, &mut captured)?, base));
    }
    Ok(Transcript { scenario: scenario.name.clone(), exchanges })
}

/// Steps whose answer differs from `expect_status`: a wrong scenario, when the
/// side is the reference.
pub fn unexpected(scenario: &Scenario, transcript: &Transcript) -> Vec<String> {
    scenario
        .steps
        .iter()
        .zip(&transcript.exchanges)
        .filter_map(|(step, exchange)| {
            let expected = step.expect_status?;
            (expected != exchange.status)
                .then(|| format!("{}/{}: the scenario expects {expected}, the reference answered {}", scenario.name, step.id, exchange.status))
        })
        .collect()
}

fn send(
    scenario: &Scenario,
    step: &StepSpec,
    base: &str,
    client: &Client,
    captured: &mut BTreeMap<String, String>,
) -> Result<RawExchange, String> {
    let target = step.path.clone().or_else(|| step.url.clone()).unwrap_or_default();
    let expanded = scenario.expand(&target, captured)?;
    // A captured Location may be relative or absolute and may carry opaque
    // query state. It is followed verbatim; the step's query is appended.
    let mut url = if expanded.starts_with("http://") || expanded.starts_with("https://") {
        expanded
    } else {
        format!("{base}{expanded}")
    };
    for (name, value) in &step.query {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(name);
        url.push('=');
        url.push_str(&encode(&scenario.expand(value, captured)?));
    }
    let method = Method::from_bytes(step.method.as_bytes()).map_err(|e| format!("{}: {e}", step.id))?;
    let mut request = client.request(method, &url);
    for (name, value) in &step.headers {
        request = request.header(name.as_str(), scenario.expand(value, captured)?);
    }
    if let Some(body) = &step.body {
        request = request.body(scenario.body(body, captured)?);
    }
    let response = request.send().map_err(|e| format!("{}/{}: {e}", scenario.name, step.id))?;
    let status = response.status().as_u16();
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in response.headers() {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        headers
            .entry(name.as_str().to_ascii_lowercase())
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    let body = response.bytes().map_err(|e| format!("{}/{}: {e}", scenario.name, step.id))?.to_vec();
    for (name, source) in &step.capture {
        let header = source.strip_prefix("header:").ok_or_else(|| format!("{}: capture {source:?} is not header:<name>", step.id))?;
        let value = headers
            .get(header)
            .ok_or_else(|| format!("{}/{}: answered {status} without the {header} header the next step needs", scenario.name, step.id))?;
        captured.insert(name.clone(), value.clone());
    }
    Ok(RawExchange { id: step.id.clone(), method: step.method.clone(), target, status, headers, body })
}

/// Percent-encode a query value, leaving what a digest and a name are made of.
fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~:/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
