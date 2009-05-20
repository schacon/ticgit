# Encoding: UTF-8
require 'spec/helper'

describe TicGit::CLI do
  behaves_like :clean_repo

  it 'displays --help' do
    expected = format_expected(<<-OUT)
Usage: ti COMMAND [FLAGS] [ARGS]

The available ticgit commands are:
    assign                           Assings a ticket to someone
    attach                           Attach file to ticket
    checkout                         Checkout a ticket
    comment                          Comment on a ticket
    list                             List tickets
    milestone                        List and modify milestones
    new                              Create a new ticket
    points                           Assign points to a ticket
    recent                           List recent activities
    show                             Show a ticket
    state                            Change state of a ticket
    tag                              Modify tags of a ticket

Common options:
    -v, --version                    Show the version number
    -h, --help                       Display this help
    OUT

    cli do |line|
      line.should == expected.shift
    end
  end

  it 'displays --version' do
    cli '--version' do |line|
      line.should == '2009.05.18'
    end
  end

  it 'displays empty list' do
    expected = format_expected(<<-OUT)
TicId  Title                    State Date  Assgn    Tags
--------------------------------------------------------------------------------
    OUT

    cli 'list' do |line|
      line.should == expected.shift
    end
  end
end
