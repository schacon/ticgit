Feature: Create tickets
  In order to remember what needs to be worked on
  As a developer
  I want to be able to create tickets and view a list of tickets
  
  Scenario: Create one ticket
    Given I am in a clean git repository
    When I execute ti "new -t 'Add support for MS Word activity streams'"
    Then the output of `ti list` should contain "add support for ms word"
  
