@status:enforced
Feature: Immutable cluster convergence
  Scenario: authenticated peers converge immutable content
    Given two authenticated Hologram peers
    When an immutable object is stored on the seed peer
    Then the joining peer eventually contains the same object
