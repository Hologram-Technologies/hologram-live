@status:enforced
Feature: Compile Hologram applications
  A module author can package a source manifest and its layers into a portable
  self-contained Hologram archive.

  Scenario: compile a view module into a fat .holo archive
    Given the example view application manifest
    When I compile the application
    Then the compile command succeeds
    And the output is a valid self-contained .holo archive
    When I plan the compiled archive directly
    Then the direct plan reports that the portable View surface is unavailable

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

  Scenario: compile, plan, and run a bounded Component Model application
    Given a Component v1 application manifest
    And a fresh Hologram home
    When I compile the application
    Then the compile command succeeds
    When I plan the compiled archive directly
    Then the component contract selects the bounded component provider
    When I run the compiled archive directly with input "hello component"
    Then the run output is "hello component"

  Scenario: mediate a capability-gated Component object read
    Given a Component store-read application manifest
    And a fresh Hologram home
    And an admitted object in the local registry
    When I compile the application
    Then the compile command succeeds
    When I plan the compiled archive directly
    Then the store-read contract selects the mediated component provider
    When I run the compiled archive without a development grant
    Then the run fails with an authorization-denied error
    When I run the store-read archive with its development grant
    Then the run output is "bdd store bytes"

  Scenario: mediate a capability-gated Component typed graph read
    Given a Component store-graph-read application manifest
    And a fresh Hologram home
    And an admitted typed object graph in the local registry
    When I compile the application
    Then the compile command succeeds
    When I plan the compiled archive directly
    Then the store-graph-read contract selects the bounded graph component provider
    When I run the compiled archive without a development grant
    Then the run fails with an authorization-denied error
    When I run the store-graph-read archive with its development grant
    Then the run output is "bdd graph leaf"

  Scenario: mediate a capability-gated Component object write
    Given a Component store-write application manifest
    And a fresh Hologram home
    And an admitted writable object target
    When I compile the application
    Then the compile command succeeds
    When I plan the compiled archive directly
    Then the store-write contract selects the mediated component provider
    When I run the compiled archive without a development grant
    Then the run fails with an authorization-denied error
    When I run the store-write archive with its development grant
    Then the admitted object bytes are present in the local registry

  Scenario: mediate a capability-gated Component channel publish
    Given a Component channel-publish application manifest
    And a fresh Hologram home
    And an admitted publish channel
    When I compile the application
    Then the compile command succeeds
    When I plan the compiled archive directly
    Then the channel-publish contract selects the mediated component provider
    When I run the compiled archive without a development grant
    Then the run fails with an authorization-denied error
    When I run the channel-publish archive with its development grant
    Then the admitted channel is returned
