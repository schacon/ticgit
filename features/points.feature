Feature: Ticket points
  In order to estimate the project schedule
  As a project manager
  I want developers to enter point estimates for tickets
  
  Scenario: Entering points
    Given I am in a clean git repository
    When I execute ti "new -t 'Add support for MS Word activity streams'"
    And refresh my ti list index
    And I execute ti "est 1 3"
    Then the output of ti "show 1" should contain /Points *: *3/
  
  Scenario: Default points
    Given I am in a clean git repository
    When I execute ti "new -t 'Add support for MS Word activity streams'"
    And refresh my ti list index
    Then the output of ti "show 1" should contain /Points *: *no estimate/
  
