require File.dirname(__FILE__) + "/spec_helper"

describe TicGitNG do
  include TicGitNGSpecHelper

  before(:each) do
    @path= setup_new_git_repo
    @orig_test_opts= test_opts
    @ticgitng= TicGitNG.open(@path, @orig_test_opts)
  end

  after(:each) do
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

      #@ticgitng.ticket_new('my new ticket')
      #git=Git.open(@path)

      git_path_2= tmp_dir + '/remote_1/'
      git_path_3= tmp_dir + '/remote_2/'

      #Make ticgit-ng branch in remote_1
      git2=Git.clone(@path, 'remote_1')
      git2.checkout('origin/ticgit-ng')
      #this creates the ticgit-ng branch, tracking origin/ticgit-ng
      git2.branch('ticgit-ng').checkout
      git2.checkout('master')

      git3=Git.clone(git_path_2, 'remote_2')
      git3.checkout('origin/ticgit-ng')
      git3.branch('ticgit-ng').checkout
      git3.checkout('master')

      ticgit2=TicGitNG.open(git_path_2, @orig_test_opts)
      ticgit2.ticket_new('my first ticket')
      ticgit3=TicGitNG.open(git_path_3, @orig_test_opts)
      ticgit3.ticket_new('my second ticket')
      ticgit2.ticket_new('my third ticket')

      #git.add_remote('upstream', git_path_2)
      #git.checkout('ticgit-ng')
      #git.pull('upstream', 'upstream/ticgit-ng')
      #git.checkout('master')
      ticgit3.sync_tickets('origin', true, false)

      ticgit3.tickets.length.should == ticgit2.tickets.length
    end
  end

  it "should be able to sync with other repos" do
    Dir.chdir(File.expand_path( tmp_dir=Dir.mktmpdir('ticgit-ng-gitdir1-') )) do
      #prep, get temp dirs, init git2

      git_path_2= tmp_dir + '/remote_1/'
      git_path_3= tmp_dir + '/remote_2/'
      git_path_4= tmp_dir + '/remote_3/'

      #Make ticgit-ng branch in remote_1
      git2=Git.clone(@path, 'remote_1')
      git2.checkout('origin/ticgit-ng')
      #this creates the ticgit-ng branch, tracking from the 
      #branch we are already on, origin/ticgit-ng
      git2.branch('ticgit-ng').checkout
      git2.checkout('master')

      #Make ticgit-ng branch in remote_2
      git3=Git.clone(@path, 'remote_2')
      git3.checkout('origin/ticgit-ng')
      git3.branch('ticgit-ng').checkout
      git3.checkout('master')

      #Make ticgit-ng branch in remote_2
      git4=Git.clone(@path, 'remote_3')
      git4.checkout('origin/ticgit-ng')
      git4.branch('ticgit-ng').checkout
      git4.checkout('master')

      ticgit2=TicGitNG.open(git_path_2, @orig_test_opts)
      ticgit2.tickets.length.should==0
      ticgit2.ticket_new('my first ticket')
      ticgit3=TicGitNG.open(git_path_3, @orig_test_opts)
      ticgit3.tickets.length.should==0
      ticgit3.ticket_new('my second ticket')
      ticgit4=TicGitNG.open(git_path_4, @orig_test_opts)
      ticgit4.tickets.length.should==0
      ticgit4.ticket_new('my third ticket')
      ticgit2.ticket_new('my fourth ticket')

      #git.add_remote('upstream', git_path_2)
      #git.checkout('ticgit-ng')
      #git.pull('upstream', 'upstream/ticgit-ng')
      #git.checkout('master')
      
      git3.add_remote('ticgit2', git_path_2)
      git4.add_remote('ticgit3', git_path_3)

      ticgit3.sync_tickets('ticgit2', true, false) #ticgit3 should now have tickets 1, 2, and 4
                                                   #and ticgit2 should now have the same
      ticgit3.tickets.length.should==3
      ticgit2.tickets.length.should==3

      ticgit4.sync_tickets('ticgit3', false, false) #ticgit4 should now have tickets 1,2,3,4
                                                    #but ticgit2 and 3 should only have 1,2,4
      ticgit4.tickets.length.should==4
      ticgit3.tickets.length.should==3
      ticgit2.tickets.length.should==3

      git4.add_remote('ticgit2', git_path_2)
      ticgit4.sync_tickets('ticgit2', true, false) #ticgit2 and 4 should now have 4 tickets while
                                                   #ticgit3 only has 3 tickets
      ticgit4.tickets.length.should==4
      ticgit3.tickets.length.should==3
      ticgit2.tickets.length.should==4
    end
  end
  it "Use the 'ticgit' branch if 'ticgit-ng' isn't available (legacy support)" do
      require 'pp'
      git=Git.clone( @path, 'remote_0' )
      branches=git.branches.local.map {|b| b.name }
      branches.include? 'ticgit'
      @ticgitng::Base.what_branch?.should == branches.include?      

      puts "sleeping"
      sleep 500
      #if 'ticgit'
      # use 'ticgit'
      #elsif 'ticgit-ng'
      # use 'ticgit-ng'
      #else
      # use 'ticgit-ng'
  end
end
