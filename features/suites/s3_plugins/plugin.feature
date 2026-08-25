@status:enforced
Feature: Subprocess plugin modules
  An operator can allowlist a third-party plugin executable; the daemon
  verifies its sha256, spawns it under supervision, and forwards JSON
  invocations over a Unix domain socket. Unknown plugin ids keep the typed
  capability error the client already recovers around.

  Scenario: list and call an allowlisted plugin
    Given a fresh Hologram home
    And an initialized configuration on a test port
    And the echo example plugin is enabled in the configuration
    When I list plugins
    Then the plugin list contains the echo plugin
    When I call plugin "dev.hologram.examples.echo" operation "echo.ping" with payload '{"hi":1}'
    Then the plugin response is '{"echo":{"hi":1}}'
    When I call plugin "dev.hologram.examples.missing" operation "echo.ping" with payload '{}'
    Then the plugin call fails with code "LIVE_CAPABILITY_MISSING"
    And I stop the local service
