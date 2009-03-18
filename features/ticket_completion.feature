Feature: Ticket completion
  In order to focus on work that needs to be done
  As a developer
  I want to keep track of which tickets have been completed
  
  Scenario: New tickets are open
    Given I am in a clean git repository
    When I execute ti "new -t 'Add support for MS Word activity streams'"
    Then the output of `ti list` should contain "open"
    And the output of `ti show 1` should contain /State *: *OPEN/
    
  Scenario: Closing a ticket
    Given I am in a clean git repository
    When I execute ti "new -t 'Add support for MS Word activity streams'"
    And refresh my ti list index
    And I execute ti "state 1 resolved"
    Then the output of `ti list` should not contain "open"
    And the output of `ti list` should not contain "support for ms word"
    And the output of `ti list -s resolved` should contain "support for ms word"
    And the output of `ti list -s resolved` should contain "resol"
    # We can refer to the ticket with "1" only because running `ti list -s resolved` above
    # has the side-effect of generating a list index of the resolved tickets
    And the output of `ti show 1` should contain /State *: *RESOLVED/
  
