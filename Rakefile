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

ti_gemspec = Gem::Specification.new do |s|
    s.platform  =   Gem::Platform::RUBY
    s.name      =   'ticgit'
    s.version   =   '0.4.0'
    s.summary   =   'A distributed ticketing system for Git projects.'
    s.files     =   FileList['bin/ti', 'lib/**/*']
    s.bindir = 'bin'
    s.executables = %w[ti]
    s.default_executable = 'ti'
    s.require_paths = %w[lib bin]
    s.add_dependency('git', '>= 1.0.5')
    s.add_development_dependency('rake', '>= 0.8.7')
    s.add_development_dependency('bundler')
end
Rake::GemPackageTask.new(ti_gemspec) { |pkg| pkg.need_tar = true }

ticgitweb_gemspec = Gem::Specification.new do |s|
    s.platform  =   Gem::Platform::RUBY
    s.name      =   'ticgitweb'
    s.version   =   '0.4.0'
    s.summary   =   'A distributed ticketing system for Git projects.'
    s.files     =   FileList['bin/ticgitweb']
    s.bindir = 'bin'
    s.executables = %w[ticgitweb]
    s.default_executable = 'ticgitweb'
    s.add_dependency('haml', '>= 3.0.23')
    s.add_dependency('sinatra', '~> 1.1')
    s.add_dependency('git', '>= 1.0.5')
    s.add_dependency('ticgit', '>= 0.4.0')
end
Rake::GemPackageTask.new(ticgitweb_gemspec) { |pkg| pkg.need_tar = true }

desc "Clean out the coverage and pkg directories"
task :clean do
  rm_rf 'coverage'
  rm_rf 'pkg'
  rm Dir.glob('ticgit*gem')
end

# Current will not run as a task in all cases. Manually remove with:
#
#    gem uninstall -axI ticgitweb ticgit
#
# instead.
task :uninstall do
  %w[ticgit ticgitweb].each do |gem|
    puts "Uninstalling #{gem} ... "
    exec "gem uninstall --all --executables --ignore-dependencies #{gem}"
  end
end

namespace :make do
  desc 'Make ticgit package for ti executable'
  task :ti => "pkg/#{ti_gemspec.name}-#{ti_gemspec.version}.gem" do
    puts "Generating #{ti_gemspec.name}-#{ti_gemspec.version}.gem"
  end

  desc 'Make ticgitweb package'
  task :ticgitweb => "pkg/#{ticgitweb_gemspec.name}-#{ticgitweb_gemspec.version}.gem" do
    puts "Generating #{ticgitweb_gemspec.name}-#{ticgitweb_gemspec.version}.gem"
  end
end # namespace :make

# Rubygems currently can't install both gems at the same time as a task;
# ticgitweb always says it can't find ticgit. Install manually with:
#
#   gem install pkg/*gem
#
# instead.
task :install do
  gems = FileList['pkg/*gem']
  gems.sort.each do |gem|
    puts "Installing #{gem} ..."
    system "gem install #{gem}"
  end
end

task :default => ['make:ti', 'make:ticgitweb']
