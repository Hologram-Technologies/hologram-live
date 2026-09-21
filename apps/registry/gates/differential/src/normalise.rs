//! What counts, and what does not, when two registries are compared.
//!
//! This file decides whether the gate can be trusted, so it is tested hardest.
//! A value that differs between two runs of the SAME registry (an upload id,
//! a port, an opaque state token, a date) is replaced by a marker. Everything
//! else is kept and compared.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The headers that are compared. Every other header is dropped; header order
/// and timing never count.
pub const COMPARED_HEADERS: [&str; 17] = [
    "accept-ranges",
    "allow",
    "cache-control",
    "content-length",
    "content-range",
    "content-type",
    "docker-content-digest",
    "docker-distribution-api-version",
    "docker-upload-uuid",
    "etag",
    "link",
    "location",
    "oci-filters-applied",
    "oci-subject",
    "range",
    "www-authenticate",
    "x-content-type-options",
];

/// Bodies over this size are compared by hash and length.
const INLINE_BODY: usize = 64 * 1024;

/// One request and its answer, as played.
#[derive(Debug, Clone)]
pub struct RawExchange {
    pub id: String,
    pub method: String,
    /// The step's path or url as the scenario wrote it, placeholders and all.
    pub target: String,
    pub status: u16,
    /// Names lowercased; repeated headers joined with `, `.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exchange {
    pub id: String,
    pub method: String,
    pub target: String,
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Body {
    Empty,
    Json(Value),
    Text(String),
    Bytes { sha256: String, len: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub scenario: String,
    pub exchanges: Vec<Exchange>,
}

pub fn normalise(mut raw: RawExchange, base: &str) -> Exchange {
    // A multipart answer invents a boundary per response. Its value is not
    // behaviour; that the answer is multipart, and what the parts hold, is.
    let boundary = raw
        .headers
        .get("content-type")
        .and_then(|value| value.split("boundary=").nth(1))
        .map(|rest| rest.trim_matches('"').to_owned())
        .filter(|boundary| !boundary.is_empty());
    if let Some(boundary) = boundary {
        raw.body = replace_bytes(&raw.body, boundary.as_bytes(), b"<boundary>");
        if let Some(value) = raw.headers.get_mut("content-type") {
            *value = value.replace(&boundary, "<boundary>");
        }
        // The marker is shorter than the boundary it replaced.
        raw.headers.remove("content-length");
    }
    let mut headers: BTreeMap<String, String> = raw
        .headers
        .into_iter()
        .filter(|(name, _)| COMPARED_HEADERS.contains(&name.as_str()))
        .map(|(name, value)| (name, scrub(&value, base)))
        .collect();
    let (body, changed) = body(&raw.body, base);
    if changed {
        // Its length moved with the marker; comparing it would be noise.
        headers.remove("content-length");
    }
    Exchange { id: raw.id, method: raw.method, target: raw.target, status: raw.status, headers, body }
}

fn replace_bytes(haystack: &[u8], needle: &[u8], with: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(haystack.len());
    let mut at = 0;
    while at < haystack.len() {
        if haystack[at..].starts_with(needle) {
            out.extend_from_slice(with);
            at += needle.len();
        } else {
            out.push(haystack[at]);
            at += 1;
        }
    }
    out
}

/// The body, and whether scrubbing changed it.
fn body(bytes: &[u8], base: &str) -> (Body, bool) {
    if bytes.is_empty() {
        return (Body::Empty, false);
    }
    if bytes.len() > INLINE_BODY {
        return (Body::Bytes { sha256: crate::scenario::sha256(bytes), len: bytes.len() }, false);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return (Body::Bytes { sha256: crate::scenario::sha256(bytes), len: bytes.len() }, false);
    };
    let scrubbed = scrub(text, base);
    let changed = scrubbed != text;
    // JSON compares by value: key order and spacing are not behaviour.
    match serde_json::from_str::<Value>(&scrubbed) {
        Ok(value @ (Value::Object(_) | Value::Array(_))) => (Body::Json(value), changed),
        _ => (Body::Text(scrubbed), changed),
    }
}

/// Replace what differs between two runs of one registry.
pub fn scrub(text: &str, base: &str) -> String {
    let text = if base.is_empty() { text.to_owned() } else { text.replace(base, "<base>") };
    strip_state(&mask_uuids(&text))
}

fn is_uuid(candidate: &[u8]) -> bool {
    candidate.len() == 36
        && candidate.iter().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn mask_uuids(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        let boundary = at == 0 || !bytes[at - 1].is_ascii_hexdigit();
        if boundary && at + 36 <= bytes.len() && is_uuid(&bytes[at..at + 36]) {
            out.push_str("<uuid>");
            at += 36;
        } else {
            // Step one whole character, so multi-byte text survives.
            let width = text[at..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[at..at + width]);
            at += width;
        }
    }
    out
}

/// The reference carries opaque upload state in `?_state=…`. Clients follow
/// it verbatim; it is not behaviour. Remove it, and the `?` it leaves behind.
fn strip_state(text: &str) -> String {
    let mut out = text.to_owned();
    while let Some(start) = out.find("_state=") {
        let end = out[start..]
            .find(|c: char| c == '&' || c == '>' || c == '"' || c == ';' || c.is_whitespace())
            .map_or(out.len(), |offset| start + offset);
        // Take the `&` that followed, or the `?`/`&` that led.
        let (from, to) = if out[end..].starts_with('&') {
            (start, end + 1)
        } else {
            (start.saturating_sub(1), end)
        };
        out.replace_range(from..to, "");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "6f1c2a9e-3b7d-4c55-9f0a-2d8e7b6a5c41";

    fn raw(status: u16, headers: &[(&str, &str)], body: &[u8]) -> RawExchange {
        RawExchange {
            id: "step".to_owned(),
            method: "GET".to_owned(),
            target: "/v2/".to_owned(),
            status,
            headers: headers.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect(),
            body: body.to_vec(),
        }
    }

    fn header<'a>(exchange: &'a Exchange, name: &str) -> Option<&'a str> {
        exchange.headers.get(name).map(String::as_str)
    }

    #[test]
    fn upload_ids_and_state_tokens_vanish_but_the_shape_stays() {
        let location = format!("http://127.0.0.1:5000/v2/gate-b/x/blobs/uploads/{UUID}?_state=abcDEF123");
        let exchange = normalise(
            raw(202, &[("location", &location), ("docker-upload-uuid", UUID), ("range", "0-0")], b""),
            "http://127.0.0.1:5000",
        );
        assert_eq!(header(&exchange, "location"), Some("<base>/v2/gate-b/x/blobs/uploads/<uuid>"));
        assert_eq!(header(&exchange, "docker-upload-uuid"), Some("<uuid>"));
        assert_eq!(header(&exchange, "range"), Some("0-0"), "meaningful values are untouched");
    }

    #[test]
    fn state_is_removed_wherever_it_sits_in_the_query() {
        assert_eq!(strip_state("/u/1?_state=abc&digest=sha256:00"), "/u/1?digest=sha256:00");
        assert_eq!(strip_state("/u/1?digest=sha256:00&_state=abc"), "/u/1?digest=sha256:00");
        assert_eq!(strip_state("</u/1?_state=abc>; rel=\"next\""), "</u/1>; rel=\"next\"");
        assert_eq!(strip_state("/plain"), "/plain");
    }

    #[test]
    fn a_relative_and_an_absolute_location_are_different_on_purpose() {
        let absolute = normalise(raw(202, &[("location", &format!("http://h:5000/v2/r/blobs/uploads/{UUID}"))], b""), "http://h:5000");
        let relative = normalise(raw(202, &[("location", &format!("/v2/r/blobs/uploads/{UUID}"))], b""), "http://h:5000");
        assert_ne!(header(&absolute, "location"), header(&relative, "location"), "location-form must be able to fail");
    }

    #[test]
    fn only_compared_headers_survive() {
        let exchange = normalise(
            raw(
                200,
                &[("date", "Mon, 21 Sep 2026 10:00:00 GMT"), ("x-content-type-options", "nosniff"), ("content-length", "2"), ("server", "whatever")],
                b"{}",
            ),
            "http://h:5000",
        );
        assert_eq!(header(&exchange, "date"), None);
        assert_eq!(header(&exchange, "server"), None);
        assert_eq!(header(&exchange, "x-content-type-options"), Some("nosniff"));
        assert_eq!(header(&exchange, "content-length"), Some("2"));
    }

    #[test]
    fn json_bodies_compare_by_value_not_by_key_order_or_spacing() {
        let a = normalise(
            raw(404, &[("content-type", "application/json")], br#"{"errors":[{"code":"BLOB_UNKNOWN","message":"blob unknown to registry","detail":{"digest":"sha256:00"}}]}"#),
            "x",
        );
        let b = normalise(
            raw(
                404,
                &[("content-type", "application/json; charset=utf-8")],
                b"{ \"errors\": [ { \"detail\": {\"digest\":\"sha256:00\"}, \"message\":\"blob unknown to registry\", \"code\":\"BLOB_UNKNOWN\" } ] }\n",
            ),
            "x",
        );
        assert_eq!(a.body, b.body);
        assert_ne!(header(&a, "content-type"), header(&b, "content-type"), "the charset difference is real and is reported");
    }

    #[test]
    fn content_length_is_dropped_only_when_the_body_held_a_normalised_value() {
        let with_id = format!("{{\"upload\":\"{UUID}\"}}");
        let moved = normalise(raw(200, &[("content-length", "50")], with_id.as_bytes()), "x");
        assert_eq!(header(&moved, "content-length"), None);
        let blob = normalise(raw(200, &[("content-length", "3")], b"abc"), "x");
        assert_eq!(header(&blob, "content-length"), Some("3"), "a blob's length is signal");
        let head = normalise(raw(200, &[("content-length", "3145728")], b""), "x");
        assert_eq!(header(&head, "content-length"), Some("3145728"), "so is a HEAD's");
    }

    #[test]
    fn a_large_or_binary_body_is_its_hash_and_length() {
        let big = vec![b'a'; INLINE_BODY + 1];
        assert!(matches!(normalise(raw(200, &[], &big), "x").body, Body::Bytes { len, .. } if len == INLINE_BODY + 1));
        assert!(matches!(normalise(raw(200, &[], &[0xff, 0xfe]), "x").body, Body::Bytes { len: 2, .. }));
        assert_eq!(normalise(raw(200, &[], b"404 page not found\n"), "x").body, Body::Text("404 page not found\n".to_owned()));
    }

    #[test]
    fn a_multipart_boundary_is_not_behaviour() {
        let answer = |boundary: &str| {
            let body = format!("--{boundary}\r\nContent-Range: bytes 0-0/9\r\n\r\nA\r\n--{boundary}--\r\n");
            normalise(raw(206, &[("content-type", &format!("multipart/byteranges; boundary={boundary}")), ("content-length", "99")], body.as_bytes()), "x")
        };
        let (one, two) = (answer("8666096a738fc901f6c4"), answer("ca05ca6e31667c5c7a2b"));
        assert_eq!(one, two);
        assert_eq!(header(&one, "content-type"), Some("multipart/byteranges; boundary=<boundary>"));
    }

    #[test]
    fn a_hex_run_longer_than_a_uuid_is_not_a_uuid() {
        let digest = "sha256:6f1c2a9e3b7d4c559f0a2d8e7b6a5c416f1c2a9e3b7d4c559f0a2d8e7b6a5c41";
        assert_eq!(mask_uuids(digest), digest);
    }
}
