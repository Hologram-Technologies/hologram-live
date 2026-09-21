@status:specified
Feature: Registry mode's network surface
  `registry serve config.yml` is a public anonymous registry, as the reference
  is by default, and nothing that administers the server faces the network
  (ADR 028).

  Not yet enforced by this runner: it drives the stock binary, and registry
  mode needs the `oci` feature. `tests/oci_http.rs` runs each scenario against
  the real binary: `registry_mode_serves_v2_publicly_and_no_administration`
  and, on Unix, `registry_mode_serves_administration_on_an_owner_only_socket`.

  Background:
    Given a registry started from a config.yml, with no live.toml

  Scenario: the public port serves the registry and the public pages
    When I GET "/v2/", "/", "/healthz", "/openapi.json" and "/docs"
    Then each answers 200

  Scenario: the module API is not on the public port, with or without a token
    When I GET "/api/v1/modules", "/api/v1/capabilities" or "/api/v1/objects"
    Then each answers 404

  Scenario: gRPC, which carries shutdown, is not on the public port
    When I call "/hologram.live.v1.HologramLive/Call" on the public port
    Then the answer's grpc-status is 12

  Scenario: administration is on an owner-only socket
    Then "<root>/live/state/admin.sock" exists with mode 0600
    And GET "/api/v1/modules" over it answers 200 and lists dev.hologram.live.oci
