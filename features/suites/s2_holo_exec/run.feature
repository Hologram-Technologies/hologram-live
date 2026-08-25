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
    When I compile the application
    Then the compile command succeeds
    When I run the compiled archive directly with input "hello file"
    Then the run output is "HELLO FILE"
    And the run reports allowed authorization from "local_baseline"

  Scenario: capability requests require an explicit sufficient grant
    Given a Wasm application that requests network fetch
    When I compile the application
    Then the compile command succeeds
    When I run the compiled archive without a development grant
    Then the run fails with an authorization-denied error
    When I run the compiled archive with its development grant and input "authorized"
    Then the run output is "AUTHORIZED"
    And the run reports allowed authorization from "direct_development_file"

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
    And I stop the local service
