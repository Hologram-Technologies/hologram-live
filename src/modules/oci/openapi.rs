//! The registry's routes in the server's one `OpenAPI` document.
//!
//! The registry adds no documentation endpoint of its own: `/openapi.json` and
//! `/docs` already exist, and a module contributes its paths to them. The
//! paths are written by hand because `{name}` may contain slashes, which no
//! router macro can express; `path.rs` is what parses them.

use utoipa::openapi::path::{HttpMethod, OperationBuilder, ParameterBuilder, ParameterIn, PathItemBuilder};
use utoipa::openapi::{OpenApi, OpenApiBuilder, PathsBuilder, Required, ResponseBuilder};

type Answers = &'static [(&'static str, &'static str)];
type Operation = (HttpMethod, &'static str, Answers);

const ERRORS: &str = "An error in the registry envelope: {\"errors\":[{\"code\",\"message\",\"detail\"}]}";

const ROUTES: &[(&str, &[Operation])] = &[
    ("/v2/", &[(HttpMethod::Get, "Say that this is a registry, and whether a login is needed", &[("200", "{}"), ("401", ERRORS)])]),
    ("/v2/_catalog", &[(HttpMethod::Get, "List repositories, paged with n and last", &[("200", "{\"repositories\":[…]}"), ("400", ERRORS)])]),
    ("/v2/{name}/tags/list", &[(HttpMethod::Get, "List a repository's tags, paged with n and last", &[("200", "{\"name\",\"tags\"}"), ("404", ERRORS)])]),
    (
        "/v2/{name}/manifests/{reference}",
        &[
            (HttpMethod::Get, "Read a manifest by tag or digest", &[("200", "The manifest, byte for byte as pushed"), ("404", ERRORS)]),
            (HttpMethod::Head, "A manifest's digest, type and length", &[("200", "Headers only"), ("404", ERRORS)]),
            (HttpMethod::Put, "Store a manifest (at most 4 MiB)", &[("201", "Stored; Location and Docker-Content-Digest name it"), ("400", ERRORS), ("413", ERRORS)]),
            (HttpMethod::Delete, "Remove a manifest, or a tag; off unless delete is enabled", &[("202", "Removed"), ("404", ERRORS), ("405", ERRORS)]),
        ],
    ),
    (
        "/v2/{name}/blobs/{digest}",
        &[
            (HttpMethod::Get, "Read a blob, whole or one Range of it, streamed", &[("200", "The blob"), ("206", "One range of it"), ("404", ERRORS), ("416", ERRORS)]),
            (HttpMethod::Head, "A blob's length and digest", &[("200", "Headers only"), ("404", ERRORS)]),
            (HttpMethod::Delete, "Remove a blob from this repository; off unless delete is enabled", &[("202", "Removed"), ("404", ERRORS), ("405", ERRORS)]),
        ],
    ),
    (
        "/v2/{name}/blobs/uploads/",
        &[(HttpMethod::Post, "Start an upload; with digest, push in one request; with mount and from, link a blob another repository holds", &[("201", "The blob exists"), ("202", "An upload session; follow Location"), ("400", ERRORS)])],
    ),
    (
        "/v2/{name}/blobs/uploads/{uuid}",
        &[
            (HttpMethod::Get, "Where an upload stands", &[("204", "Range says how far"), ("404", ERRORS)]),
            (HttpMethod::Patch, "Append a chunk, streamed", &[("202", "Appended"), ("404", ERRORS), ("416", ERRORS)]),
            (HttpMethod::Put, "Finish an upload with the digest the whole must hash to", &[("201", "The blob exists"), ("400", ERRORS), ("404", ERRORS)]),
            (HttpMethod::Delete, "Abandon an upload", &[("204", "Gone"), ("404", ERRORS)]),
        ],
    ),
    ("/v2/{name}/referrers/{digest}", &[(HttpMethod::Get, "The manifests whose subject is this digest, as an OCI index", &[("200", "An index; empty when there are none"), ("400", ERRORS)])]),
];

pub fn document() -> OpenApi {
    let mut paths = PathsBuilder::new();
    for (path, operations) in ROUTES {
        let mut item = PathItemBuilder::new();
        for (method, summary, answers) in *operations {
            let mut operation = OperationBuilder::new().tag("registry").summary(Some(*summary));
            for parameter in path.split('/').filter_map(|part| part.strip_prefix('{')?.strip_suffix('}')) {
                let description = (parameter == "name").then_some("A repository name. It may contain slashes.");
                operation = operation.parameter(
                    ParameterBuilder::new()
                        .name(parameter)
                        .parameter_in(ParameterIn::Path)
                        .required(Required::True)
                        .description(description),
                );
            }
            for (status, description) in *answers {
                operation = operation.response(*status, ResponseBuilder::new().description(*description).build());
            }
            item = item.operation(method.clone(), operation.build());
        }
        paths = paths.path(*path, item.build());
    }
    OpenApiBuilder::new().paths(paths.build()).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_the_parser_knows_is_documented() {
        let document = document();
        for path in [
            "/v2/",
            "/v2/_catalog",
            "/v2/{name}/tags/list",
            "/v2/{name}/manifests/{reference}",
            "/v2/{name}/blobs/{digest}",
            "/v2/{name}/blobs/uploads/",
            "/v2/{name}/blobs/uploads/{uuid}",
            "/v2/{name}/referrers/{digest}",
        ] {
            assert!(document.paths.paths.contains_key(path), "{path}");
        }
        let manifests = &document.paths.paths["/v2/{name}/manifests/{reference}"];
        assert!(manifests.get.is_some() && manifests.head.is_some() && manifests.put.is_some() && manifests.delete.is_some());
    }
}
