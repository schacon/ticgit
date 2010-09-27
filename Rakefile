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

require 'bundler/setup'
require 'rake/gempackagetask'
require "rspec/core/rake_task"
gem 'rspec', '~>2.0'

namespace :bundle do
  desc 'Install all required gems.'
  task :install do
    system 'bundle install'
  end

  desc 'List bundled gems.'
  task :show do
    system 'bundle show'
  end
end

namespace :test do
  desc 'Run all RSpec tests'
  RSpec::Core::RakeTask.new

  desc 'Remove RSpec temp directories'
  task :clean do
    rmtree Dir.glob('/tmp/ticgit-*')
  end
end

gemspec = eval(File.read('ticgit.gemspec'))
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
