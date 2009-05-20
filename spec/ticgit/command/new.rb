require 'spec/helper'

describe 'ti new' do
  describe 'Create a ticket' do
    behaves_like :clean_repo

    it 'creates a ticket via -t' do
      expected = {
        'Title'    => /^Add support for MS Word activity streams$/,
        'TicId'    => /^\w{40}$/,
        'Assigned' => /m.fellinger@gmail.com/,
        'Opened'   => /\d{4}-\d\d-\d\d \d\d:\d\d:\d\d \+\d{4} \(0 days\)/,
        'State'    => /OPEN/,
        'Points'   => /no estimate/,
      }

      cli 'new', '-t', 'Add support for MS Word activity streams' do |line|
        left, right = line.split(/\s*:\s*/, 2)
        matcher = expected[left]
        right.should =~ matcher
      end
    end
  end

  describe 'Create many tickets' do
    behaves_like :clean_repo

    it 'creates many tickets via -t' do
      cli 'new', '-t', 'MS Word activity streams'
      cli 'new', '-t', 'Keynote activity streams'
      cli 'new', '-t', 'Excel activity streams'
      cli 'new', '-t', 'IntelliJ activity streams'

      expected = [
        /TicId\s+Title\s+State\s+Date\s+Assgn\s+Tags/,
        /-{80}/,
        /MS Word activity streams/,
        /Keynote activity streams/,
        /Excel activity streams/,
        /IntelliJ activity streams/,
      ]

      cli 'list', '-s', 'title' do |line|
        line.should =~ expected.shift
      end
    end
  end
end
