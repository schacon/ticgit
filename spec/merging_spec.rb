require File.dirname(__FILE__) + "/spec_helper"

describe TicGit do
  include TicGitSpecHelper

  before(:all) do
    @path= setup_new_git_repo
    @orig_test_opts= test_opts
    @ticgit= TicGit.open(@path, @orig_test_opts)
  end

  after(:all) do
    Dir.glob(File.expand_path("~/.ticgit/-tmp*")).each {|file_name| FileUtils.rm_r(file_name, {:force=>true,:secure=>true}) }
    Dir.glob(File.expand_path("/tmp/ticgit-*")).each {|file_name| FileUtils.rm_r(file_name, {:force=>true,:secure=>true}) }
  end

  it "Should merge in tickets from a remote source" do
    Dir.chdir(File.expand_path( tmp_dir=Dir.mktmpdir('ticgit-gitdir1-') )) do
      #prep, get temp dirs, init git2
      @ticgit.ticket_new('my new ticket')
      git2=Git.clone(@path, 'remote_1')
      git=Git.open(@path)
      git_path_2= tmp_dir + '/remote_1/'

      #Make ticgit branch in remote_1
      git2.checkout('origin/ticgit')
      git2.branch('ticgit').checkout
      ticgit2=TicGit.open(git_path_2, @orig_test_opts)

      ticgit2.ticket_new('my second ticket')
      git2.checkout('master')

      git.add_remote('upstream', git_path_2)
      git.checkout('ticgit')
      git.pull('upstream', 'upstream/ticgit')
      git.checkout('master')

      #Without calling reset_ticgit, the following line only shows 1 ticket.
      @ticgit.reset_ticgit
      #@ticgit.tickets.length
      
      ticgit2.tickets.length.should == @ticgit.tickets.length
    end
  end

  it "Should not enounter merge conflicts"

end
