@status:enforced
Feature: Compile Hologram applications
  A module author can package a source manifest and its layers into a portable
  self-contained Hologram archive.

  Scenario: compile a view module into a fat .holo archive
    Given the example view application manifest
    When I compile the application
    Then the compile command succeeds
    And the output is a valid self-contained .holo archive

  Scenario: generate and run a Wasm application manifest
    Given a new application directory
    When I initialize a Wasm application manifest
    Then the generated manifest is valid
    When I compile the application
    Then the compile command succeeds
    When I run the compiled archive directly with input "hello generator"
    Then the run output is "HELLO GENERATOR"
