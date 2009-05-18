Feature: Create tickets
  In order to remember what needs to be worked on
  As a developer
  I want to be able to create tickets and view a list of tickets
  
  Scenario: Create one ticket
    Given I am in a clean git repository
    When I execute ti "new -t 'Add support for MS Word activity streams'"
    Then the output of ti "list" should contain "Add support for MS Word"
  
  Scenario: Create many ticket
    Given I am in a clean git repository
    When I execute ti "new -t 'MS Word activity streams'"
    And I execute ti "new -t 'Keynote activity streams'"
    And I execute ti "new -t 'Excel activity streams'"
    And I execute ti "new -t 'IntelliJ activity streams'"
    Then the output of ti "list" should contain "MS Word activity streams"
    And the output of ti "list" should contain "Keynote activity streams"
    And the output of ti "list" should contain "Excel activity streams"
    And the output of ti "list" should contain "IntelliJ activity streams"
  
