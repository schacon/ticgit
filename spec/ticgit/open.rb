require 'spec/helper'

describe 'TicGit::open' do
  behaves_like :clean_ticgit

  it "creates a new branch if it's not there" do
    @ticgit.git.branches.find{|b| b.name == 'ticgit' }.name.should == 'ticgit'
  end

  it "finds an existing ticgit branch if it's there" do
    tg = ticgit_open
    @ticgit.git.branches.size.should == tg.git.branches.size
  end

  it "finds the .git directory if it's there" do
    @ticgit.git.dir.path.should == @path
  end

  it "looks for the .git directory until it finds it" do
    tg = ticgit_open(File.join(@path, 'subdir'))
    tg.git.dir.path.should == @path
  end

  it "adds a .hold file to a new branch" do
    @ticgit.in_branch{ File.file?('.hold').should == true }
  end
end
