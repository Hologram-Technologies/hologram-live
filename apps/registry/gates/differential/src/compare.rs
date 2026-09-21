//! Two transcripts of one scenario, field by field.

use crate::normalise::{Body, Transcript};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub scenario: String,
    pub step: String,
    /// `status`, `header:<name>`, `body`, `body.errors[0].code`, `steps`.
    pub field: String,
    pub left: String,
    pub right: String,
}

pub fn compare(left: &Transcript, right: &Transcript) -> Vec<Difference> {
    let mut out = Vec::new();
    let scenario = left.scenario.clone();
    let mut push = |step: &str, field: String, was: String, is: String| {
        out.push(Difference { scenario: scenario.clone(), step: step.to_owned(), field, left: was, right: is });
    };
    if left.exchanges.len() != right.exchanges.len() {
        push("*", "steps".to_owned(), left.exchanges.len().to_string(), right.exchanges.len().to_string());
    }
    for (a, b) in left.exchanges.iter().zip(&right.exchanges) {
        if a.status != b.status {
            push(&a.id, "status".to_owned(), a.status.to_string(), b.status.to_string());
        }
        let names: BTreeSet<&String> = a.headers.keys().chain(b.headers.keys()).collect();
        for name in names {
            let (x, y) = (a.headers.get(name), b.headers.get(name));
            if x != y {
                push(&a.id, format!("header:{name}"), show(x), show(y));
            }
        }
        if a.body != b.body {
            match (error_code(&a.body), error_code(&b.body)) {
                (Some(x), Some(y)) if x != y => push(&a.id, "body.errors[0].code".to_owned(), x, y),
                _ => push(&a.id, "body".to_owned(), render(&a.body), render(&b.body)),
            }
        }
    }
    out
}

fn show(value: Option<&String>) -> String {
    value.map_or_else(|| "(absent)".to_owned(), Clone::clone)
}

fn error_code(body: &Body) -> Option<String> {
    match body {
        Body::Json(value) => value["errors"][0]["code"].as_str().map(str::to_owned),
        _ => None,
    }
}

fn render(body: &Body) -> String {
    let text = match body {
        Body::Empty => "(empty)".to_owned(),
        Body::Json(value) => value.to_string(),
        Body::Text(text) => format!("{text:?}"),
        Body::Bytes { sha256, len } => format!("{len} bytes {sha256}"),
    };
    // A table cell: one line, no pipes, not a page long.
    let text = text.replace('|', "\\|").replace('\n', " ");
    match text.char_indices().nth(240) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalise::Exchange;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn exchange(status: u16, headers: &[(&str, &str)], body: Body) -> Exchange {
        Exchange {
            id: "get".to_owned(),
            method: "GET".to_owned(),
            target: "/v2/".to_owned(),
            status,
            headers: headers.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect::<BTreeMap<_, _>>(),
            body,
        }
    }

    fn transcript(exchanges: Vec<Exchange>) -> Transcript {
        Transcript { scenario: "base".to_owned(), exchanges }
    }

    #[test]
    fn equal_transcripts_have_no_differences() {
        let one = transcript(vec![exchange(200, &[("content-type", "application/json")], Body::Json(json!({})))]);
        assert!(compare(&one, &one.clone()).is_empty());
    }

    #[test]
    fn each_kind_of_difference_is_named() {
        let left = transcript(vec![exchange(404, &[("allow", "GET")], Body::Json(json!({"errors":[{"code":"BLOB_UNKNOWN"}]})))]);
        let right = transcript(vec![exchange(400, &[("link", "x")], Body::Json(json!({"errors":[{"code":"DIGEST_INVALID"}]})))]);
        let fields: Vec<String> = compare(&left, &right).into_iter().map(|d| d.field).collect();
        assert_eq!(fields, ["status", "header:allow", "header:link", "body.errors[0].code"]);
    }

    #[test]
    fn a_missing_step_is_a_difference() {
        let left = transcript(vec![exchange(200, &[], Body::Empty), exchange(200, &[], Body::Empty)]);
        let right = transcript(vec![exchange(200, &[], Body::Empty)]);
        let differences = compare(&left, &right);
        assert_eq!(differences.len(), 1);
        assert_eq!((differences[0].field.as_str(), differences[0].left.as_str(), differences[0].right.as_str()), ("steps", "2", "1"));
    }
}
