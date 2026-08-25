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
    And I load the archive
    And I run the archive with input "hello hologram"
    Then the run output is "HELLO HOLOGRAM"
    And the archive appears in the resident list
    When I unload the archive
    Then running the archive fails with a not-found error
    And I stop the local service
