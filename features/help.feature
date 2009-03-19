Feature: Ticgit help
  In order to use ticgit effectively
  As a developer
  I want to see online help for ticgit
  
  Scenario: List of ti commands
    Given I am in a clean git repository
    When I execute ti ""
    Then the output of ti "" should contain "points"
  
