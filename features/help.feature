Feature: Ticgit help
  In order to use ticgit effectively
  As a developer
  I want to see online help for ticgit
  
  Scenario: List of ti commands
    Given I am in a clean git repository
    When I execute ti ""
    Then the output of ti "" should contain "Please specify at least one action to execute"
    And the output of ti "" should contain "list"
    And the output of ti "" should contain "state"
    And the output of ti "" should contain "show"
    And the output of ti "" should contain "new"
    And the output of ti "" should contain "checkout"
    And the output of ti "" should contain "comment"
    And the output of ti "" should contain "tag"
    And the output of ti "" should contain "assign"
    And the output of ti "" should contain "points"
  
