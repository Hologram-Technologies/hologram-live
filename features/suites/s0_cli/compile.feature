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
    And a fresh Hologram home
    When I initialize a Wasm application manifest
    Then the generated manifest is valid
    When I compile the application
    Then the compile command succeeds
    When I plan the compiled archive directly
    Then the direct plan is runnable without exposing payload bytes
    When I run the compiled archive directly with input "hello generator"
    Then the run output is "HELLO GENERATOR"

  Scenario: compile and plan a Component Model application without ABI fallback
    Given a Component v1 application manifest
    When I compile the application
    Then the compile command succeeds
    When I plan the compiled archive directly
    Then the component contract is inspectable and unavailable without fallback
