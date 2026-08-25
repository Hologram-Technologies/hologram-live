@status:enforced
Feature: Chat through the inference engine boundary
  Chat sends persist through the configured inference engine. The default
  echo engine repeats the user message as the assistant response.

  Scenario: the echo engine answers and the exchange is recorded
    Given a fresh Hologram home
    And an initialized configuration on a test port
    When I create a conversation titled "bdd chat"
    And I send "hello engine" to the conversation
    Then the assistant response echoes the message
    And both sides of the exchange are recorded
    And I stop the local service
