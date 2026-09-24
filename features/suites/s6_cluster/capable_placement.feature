@status:enforced
Feature: Capability-aware cluster placement
  Scenario: an incapable peer is excluded from placement
    Given two authenticated Hologram peers
    When only one peer advertises the required operation
    Then placement selects that capable peer
