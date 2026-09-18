@status:enforced
Feature: The HTTP surface answers what it does not serve
  The daemon serves REST and gRPC on one port. A path no route claims is a
  404 in the daemon's error envelope; it is never reported as a success, and
  gRPC callers are unaffected.

  Scenario: an unknown path is a 404 and the gRPC service still answers
    Given a fresh Hologram home
    And an initialized configuration on a test port
    When I restart the local service
    And I request the path "/api/v1/nope" over HTTP
    Then the HTTP answer is 404 with error code "LIVE_NOT_FOUND"
    When I create a conversation titled "after the fallback"
    Then I stop the local service
