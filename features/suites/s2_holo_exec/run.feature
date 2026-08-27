@status:enforced
Feature: Resident .holo wasm execution
  An operator can load a compiled wasm application into the daemon as a
  resident holo, run inputs through it, and unload it again. Runs against an
  unloaded archive fail with a typed not-found error.

  Scenario: load, run, and unload a resident wasm application
    Given the example wasm application manifest
    When I compile the application
    Then the compile command succeeds
    Given a fresh Hologram home
    And an initialized configuration on a test port
    When I import the compiled archive
    And I plan the imported archive
    Then the resident plan identifies the imported archive
    When I load the archive
    And I run the archive with input "hello hologram"
    Then the run output is "HELLO HOLOGRAM"
    And the archive appears in the resident list
    When I unload the archive
    Then running the archive fails with a not-found error
    And I stop the local service

  Scenario: execute a local .holo file without a service
    Given the example wasm application manifest
    And a fresh Hologram home
    When I compile the application
    Then the compile command succeeds
    When I run the compiled archive directly with input "hello file"
    Then the run output is "HELLO FILE"
    And the run completion is returned without an exit code
    And the run reports allowed authorization from "local_baseline"
    And the capability audit records "allowed" from "local_baseline" for principal "local-cli"

  Scenario: invoke the manifest-declared Wasm entry directly and resident
    Given a Wasm application with a custom manifest entrypoint
    And a fresh Hologram home
    When I compile the application
    Then the compile command succeeds
    When I run the compiled archive directly with input "custom direct"
    Then the run output is "CUSTOM DIRECT"
    Given an initialized configuration on a test port
    When I import the compiled archive
    And I load the archive
    And I run the archive with input "custom resident"
    Then the run output is "CUSTOM RESIDENT"
    And the run completion is returned without an exit code
    And I stop the local service

  Scenario: capability requests require an explicit sufficient grant
    Given a Wasm application that requests network fetch
    And a fresh Hologram home
    When I compile the application
    Then the compile command succeeds
    When I run the compiled archive without a development grant
    Then the run fails with an authorization-denied error
    And the capability audit records "denied" from "local_baseline" for principal "local-cli"
    When I run the compiled archive with its development grant and input "authorized"
    Then the run output is "AUTHORIZED"
    And the run reports allowed authorization from "direct_development_file"
    And the capability audit records "allowed" from "direct_development_file" for principal "local-cli"
    And the capability audit contains no source document or payload data

  Scenario: resident execution uses only the service development grant
    Given a Wasm application that requests network fetch
    When I compile the application
    Then the compile command succeeds
    Given a fresh Hologram home
    And an initialized configuration on a test port
    And the service uses the development grant
    When I import the compiled archive
    And I load the archive
    And I run the archive with input "resident grant"
    Then the run output is "RESIDENT GRANT"
    And the run reports allowed authorization from "service_development_file"
    And the capability audit records "allowed" from "service_development_file" for principal "local-user"
    And the capability audit contains no source document or payload data
    And I stop the local service

  Scenario: resident execution exposes authorization evidence over HTTP
    Given the example wasm application manifest
    When I compile the application
    Then the compile command succeeds
    Given a fresh Hologram home
    And an initialized configuration on a test port
    When I import the compiled archive
    And I load the archive
    And I run the archive over HTTP with input "http evidence"
    Then the run output is "HTTP EVIDENCE"
    And the run completion is returned without an exit code
    And the run reports allowed authorization from "local_baseline"
    And I stop the local service

  Scenario: a declared resident application loads at service startup
    Given the example wasm application manifest
    When I compile the application
    Then the compile command succeeds
    Given a fresh Hologram home
    And an initialized configuration on a test port
    When I import the compiled archive
    And the service declares the imported archive as a resident application
    And I restart the local service
    Then the archive appears in the resident list
    When I run the archive with input "declared boot"
    Then the run output is "DECLARED BOOT"
    And I stop the local service
