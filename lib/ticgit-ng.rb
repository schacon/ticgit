require 'fileutils'
require 'logger'
require 'optparse'
require 'ostruct'
require 'set'
require 'yaml'

# Add the directory containing this file to the start of the load path if it
# isn't there already.
$:.unshift(File.dirname(__FILE__)) unless
  $:.include?(File.dirname(__FILE__)) || $:.include?(File.expand_path(File.dirname(__FILE__)))

require 'rubygems'
# requires git >= 1.0.5
require 'git'
require 'ticgit-ng/base'
require 'ticgit-ng/cli'

module TicGitNG
  autoload :VERSION, 'ticgit-ng/version'
  autoload :Comment, 'ticgit-ng/comment'
  autoload :Ticket, 'ticgit-ng/ticket'

  # options
  #   :logger => Logger.new(STDOUT)
  def self.open(git_dir, options = {})
    Base.new(git_dir, options)
  end

  class OpenStruct < ::OpenStruct
    def to_hash
      @table.dup
    end
  end
end
