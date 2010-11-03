# Ensure that the basics are installed before doing anything more
# complicated.
%w[rubygems bundler].each do |gem|
  begin
    require gem
  rescue LoadError
    $stderr.puts 'Missing gem: ' << gem
    $load_error = true
  end
end
exit 1 if $load_error

# This namespace must be loaded near the top in order to be able to
# display bundle-related rake tasks in the next section.
namespace :bundle do
  namespace :install do
    desc 'Install gems for all tasks, including testing.'
    task :all do
      system 'bundle install'
    end

    desc 'Install standard CLI and web dependencies.'
    task :std do
      system 'bundle install --without dev'
    end

    # The gem will not currently build properly without the ticgitweb
    # dependencies. Leave this task commented until the gemspec will
    # build just the CLI.
    #desc 'Install CLI dependencies only.'
    #task :cli do
    #  system 'bundle install --without dev web'
    #end
  end

  desc 'List bundled gems.'
  task :show do
    system 'bundle show'
  end
end

# If 'bundler install' hasn't been run, display the available bundler
# tasks.
unless File.directory? '.bundle'
  $stderr.puts 'You must run one of the bundle:install tasks first:'
  $stderr.puts
  Rake::Task.tasks.each {|task| $stderr.puts "    rake #{task}"}
  $stderr.puts
  exit 1 if ARGV.to_s.grep(/bundle:install/).empty?
end

require 'bundler/setup'
require 'rake/gempackagetask'

begin
  require "rspec/core/rake_task"
  namespace :test do
    desc 'Run all RSpec tests'
    RSpec::Core::RakeTask.new

    desc 'Remove RSpec temp directories'
    task :clean do
      rmtree Dir.glob('/tmp/ticgit-*')
    end
  end
rescue LoadError
  $stderr.puts 'RSpec ~> 2.0 needed for testing.'
  $stderr.puts
end

files = FileList['bin/*', 'lib/**/*']
gemspec = Gem::Specification.new do |s|
    s.platform  =   Gem::Platform::RUBY
    s.name      =   'ticgit-ng'
    s.version   =   '0.4.0'
    s.summary   =   'A distributed ticketing system for Git projects.'
    s.files     =   FileList[files]
    s.bindir = 'bin'
    s.executables = %w[ti ticgitweb]
    s.default_executable = 'ti'
    s.add_dependency('git', '>= 1.0.5')
    s.require_paths = %w[lib bin]
end
Rake::GemPackageTask.new(gemspec) do |pkg|
  pkg.need_tar = true
end

desc "Clean out the coverage and pkg directories"
task :clean do
  rm_rf 'coverage'
  rm_rf 'pkg'
  rm Dir.glob('ticgit*gem')
end

task :make => "pkg/#{gemspec.name}-#{gemspec.version}.gem" do
  puts "Generating #{gemspec.name}-#{gemspec.version}.gem"
end

task :install do
    puts "Installing #{gemspec.name}-#{gemspec.version}.gem ..."
    system "gem install pkg/#{gemspec.name}-#{gemspec.version}.gem"
end

task :default => [:make, :install]
