require 'spec/helper'

describe 'ti state' do
  describe 'New tickets have OPEN status' do
    behaves_like :clean_repo

    it 'creates new tickets with OPEN status' do
      cli 'new', '-t', 'Add support for MS Word activity streams'
      cli 'list' # generate @last_tickets so we don't need the ticid

      expected = {
        'Title'    => /^Add support for MS Word activity streams$/,
        'TicId'    => /^\w{40}$/,
        'Assigned' => /m.fellinger@gmail.com/,
        'Opened'   => /\d{4}-\d\d-\d\d \d\d:\d\d:\d\d \+\d{4} \(0 days\)/,
        'State'    => /OPEN/,
        'Points'   => /no estimate/,
      }

      cli 'show', '1' do |line|
        left, right = line.split(/\s*:\s*/, 2)
        matcher = expected[left]
        right.should =~ matcher
      end
    end
  end

  describe 'Tickets may have RESOLVED status' do
    behaves_like :clean_repo

    it 'closes a ticket with the state command' do
      cli 'new', '-t', 'Add support for MS Word activity streams'
      cli 'list' # generate @last_tickets so we don't need the ticid
      cli 'state', '1', 'resolved'

      expected = {
        'Title'    => /^Add support for MS Word activity streams$/,
        'TicId'    => /^\w{40}$/,
        'Assigned' => /m.fellinger@gmail.com/,
        'Opened'   => /\d{4}-\d\d-\d\d \d\d:\d\d:\d\d \+\d{4} \(0 days\)/,
        'State'    => /RESOLVED/,
        'Points'   => /no estimate/,
      }

      cli 'show', '1' do |line|
        left, right = line.split(/\s*:\s*/, 2)
        matcher = expected[left]
        right.should =~ matcher
      end
    end
  end
end
