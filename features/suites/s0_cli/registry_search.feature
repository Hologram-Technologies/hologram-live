@status:enforced
Feature: Object search over the registry provider
  Objects are searchable by their stored metadata through a bounded, paginated
  surface. The result page is ordered ascending by object id and carries a
  cursor only while further matches remain.

  Scenario: filtering objects by kind
    Given a fresh Hologram home
    And an initialized configuration on a test port
    When I store a file object named "alpha.txt"
    And I store a file object named "beta.txt"
    And I search objects with kind "file"
    Then the search result contains 2 objects
    And every returned object has kind "file"
    And I stop the local service

  Scenario: paging through results without overlap
    Given a fresh Hologram home
    And an initialized configuration on a test port
    When I store a file object named "one.txt"
    And I store a file object named "two.txt"
    And I store a file object named "three.txt"
    And I search objects with limit 2
    Then the search result contains 2 objects
    And the search result carries a cursor
    When I search the next page with limit 2
    Then the search result contains 1 object
    And no object appears on both pages
    And I stop the local service
