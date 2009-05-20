require 'spec/helper'

describe 'ti points' do
  describe 'Entering point estimates' do
    behaves_like :clean_repo

    it 'enters points' do
      cli 'new', '-t', 'Add support for MS Word activity streams'
      cli 'list' # generate @last_tickets so we don't need the ticid
      cli 'points', '1', '3'

      expected = {
        'Title'    => /^Add support for MS Word activity streams$/,
        'TicId'    => /^\w{40}$/,
        'Assigned' => /m.fellinger@gmail.com/,
        'Opened'   => /\d{4}-\d\d-\d\d \d\d:\d\d:\d\d \+\d{4} \(0 days\)/,
        'State'    => /OPEN/,
        'Points'   => /3/,
      }

      cli 'show', '1' do |line|
        left, right = line.split(/\s*:\s*/, 2)
        matcher = expected[left]
        right.should =~ matcher
      end
    end
  end

  describe 'default points' do
    behaves_like :clean_repo

    it 'has no estimate unless points were set' do
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
end
