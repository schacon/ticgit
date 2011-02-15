require File.dirname(__FILE__) + "/spec_helper"

describe TicGitNG do
  include TicGitNGSpecHelper

  before(:all) do
    @path= setup_new_git_repo
    @orig_test_opts= test_opts
    @ticgitng= TicGitNG.open(@path, @orig_test_opts)
  end

  after(:all) do
    Dir.glob(File.expand_path("~/.ticgit-ng/-tmp*")).each {|file_name| FileUtils.rm_r(file_name, {:force=>true,:secure=>true}) }
    Dir.glob(File.expand_path("/tmp/ticgit-ng-*")).each {|file_name| FileUtils.rm_r(file_name, {:force=>true,:secure=>true}) }
  end

  it "Should merge in tickets from a remote source" do
    Dir.chdir(File.expand_path( tmp_dir=Dir.mktmpdir('ticgit-ng-gitdir1-') )) do
      #prep, get temp dirs, init git2
      @ticgitng.ticket_new('my new ticket')
      git2=Git.clone(@path, 'remote_1')
      git=Git.open(@path)
      git_path_2= tmp_dir + '/remote_1/'

      #Make ticgit-ng branch in remote_1
      git2.checkout('origin/ticgit-ng')
      git2.branch('ticgit-ng').checkout
      ticgit2=TicGitNG.open(git_path_2, @orig_test_opts)

      ticgit2.ticket_new('my second ticket')
      git2.checkout('master')

      git.add_remote('upstream', git_path_2)
      git.checkout('ticgit-ng')
      git.pull('upstream', 'upstream/ticgit-ng')
      git.checkout('master')

      ticgit2.tickets.length.should == @ticgitng.tickets.length
    end
  end

  it "should be able to sync with origin" do
    Dir.chdir(File.expand_path( tmp_dir=Dir.mktmpdir('ticgit-ng-gitdir1-') )) do
      #prep, get temp dirs, init git2

      @ticgitng.ticket_new('my new ticket')
      git=Git.open(@path)
      git_path_2= tmp_dir + '/remote_1/'

      #Make ticgit-ng branch in remote_1
      git2=Git.clone(@path, 'remote_1')
      git2.checkout('origin/ticgit-ng')
      #this creates the ticgit-ng branch, tracking origin/ticgit-ng
      git2.branch('ticgit-ng').checkout
      git2.checkout('master')

      ticgit2=TicGitNG.open(git_path_2, @orig_test_opts)
      ticgit2.ticket_new('my second ticket')
      @ticgitng.ticket_new('my third ticket')

      #git.add_remote('upstream', git_path_2)
      #git.checkout('ticgit-ng')
      #git.pull('upstream', 'upstream/ticgit-ng')
      #git.checkout('master')
      ticgit2.sync_tickets

      ticgit2.tickets.length.should == @ticgitng.tickets.length
    end
  end

  it "should be able to sync with other repos" do
    Dir.chdir(File.expand_path( tmp_dir=Dir.mktmpdir('ticgit-ng-gitdir1-') )) do
      #prep, get temp dirs, init git2

      @ticgitng.ticket_new('my new ticket')
      git=Git.open(@path)
      git_path_2= tmp_dir + '/remote_1/'

      #Make ticgit-ng branch in remote_1
      git2=Git.clone(@path, 'remote_1')
      git2.checkout('origin/ticgit-ng')
      #this creates the ticgit-ng branch, tracking origin/ticgit-ng
      git2.branch('ticgit-ng').checkout
      git2.checkout('master')

      ticgit2=TicGitNG.open(git_path_2, @orig_test_opts)
      ticgit2.ticket_new('my second ticket')
      @ticgitng.ticket_new('my third ticket')

      #git.add_remote('upstream', git_path_2)
      #git.checkout('ticgit-ng')
      #git.pull('upstream', 'upstream/ticgit-ng')
      #git.checkout('master')
      ticgit2.sync_tickets

      ticgit2.tickets.length.should == @ticgitng.tickets.length
    end
  end
end
