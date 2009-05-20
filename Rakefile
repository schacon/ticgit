require 'rake'
require 'rake/clean'
require 'rake/gempackagetask'
require 'time'
require 'date'

PROJECT_SPECS = Dir['spec/ticgit/**/*.rb']
PROJECT_MODULE = 'Foo'
PROJECT_VERSION = ENV['VERSION'] || Date.today.strftime("%Y.%m.%d")

GEMSPEC = Gem::Specification.new{|s|
  s.name         = 'Ticgit'
  s.version      = PROJECT_VERSION
  s.author       = "Michael 'manveru' Fellinger"
  s.summary      = "A distributed ticketing system for git projects."
  # s.description  = "A distributed ticketing system for git projects."
  s.email        = 'm.fellinger@gmail.com'
  s.homepage     = 'http://github.com/manveru/ticgit'
  s.bindir       = 'bin'
  s.executables  = %w[ti ticgitweb]
  s.files        = `git ls-files`.split("\n").sort
  s.has_rdoc     = true
  s.platform     = Gem::Platform::RUBY
  s.require_path = 'lib'
  s.default_executable = s.executables.first

  s.add_dependency('git', '~> 1.0.5')
}

Dir['tasks/*.rake'].each{|f| import(f) }

task :default => [:bacon]

CLEAN.include('')
