require 'tmpdir'
require 'set'
require 'stringio'
require 'enumerator'
require 'bacon'

require File.expand_path("../../lib/ticgit", __FILE__)

TICGIT_HISTORY = StringIO.new

module TicGit::Command
  def self.puts(*args)
    TICGIT_HISTORY.puts(*args)
  end
end

CLEANUP_AT_EXIT = Set.new

shared :ticgit do
  def setup_new_git_repo
    srand

    begin
      path = File.join(Dir.tmpdir, "ticgit-#{rand}")
    end while File.exist?(path)

    Dir.mkdir(path)
    CLEANUP_AT_EXIT << path

    Dir.chdir(path) do
      git = Git.init

      Dir.mkdir('subdir')

      new_file('test', 'content')
      new_file('subdir/testfile', 'content2')

      git.add
      git.commit('first commit')
    end

    path
  end

  def new_file(name, contents)
    File.open(name, 'w+'){|io| io.puts(contents) }
  end

  def cli(*args, &block)
    TICGIT_HISTORY.truncate 0
    TICGIT_HISTORY.rewind

    ticgit = TicGit::CLI.new(args.flatten, @path, TICGIT_HISTORY)
    ticgit.parse_options!
    ticgit.execute!

    replay_history(&block)
  rescue SystemExit => error
    replay_history(&block)
  end

  def replay_history
    TICGIT_HISTORY.rewind
    return unless block_given?

    while line = TICGIT_HISTORY.gets
      yield(line.strip)
    end
  end

  def format_expected(string)
    string.strip.enum_for(:each_line).map{|line| line.strip }
  end
end

shared :clean_repo do
  behaves_like :ticgit

  def reset
    @path = setup_new_git_repo
  end

  reset
end

shared :clean_ticgit do
  behaves_like :ticgit

  def reset
    @path = setup_new_git_repo
    @logger = Logger.new(File.join(@path, 'spec.log'))
    @ticgit = ticgit_open
  end

  def ticgit_open(path = @path)
    # don't mess up the output
    TicGit.open(path, :logger => @logger)
  end

  reset
end

# Patch bacon so we can cleanup the temporary repos at_exit, might suggest a
# patch to chris.

module Bacon
  Counter[:installed_summary] = 1

  class << self
    def summary_on_exit
      return if Counter[:installed_summary] > 0
      at_exit{ exit_with_summary }
      Counter[:installed_summary] += 1
    end
    alias summary_at_exit summary_on_exit

    def exit_with_summary
      handle_summary
      raise $! if $!
      exit 1 if Counter[:errors] + Counter[:failed] > 0
    end
  end
end

at_exit do
  CLEANUP_AT_EXIT.each{|path| FileUtils.rm_r(path) }
  Bacon.exit_with_summary
end
