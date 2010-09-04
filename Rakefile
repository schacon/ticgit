require 'rubygems'
require 'bundler'

unless system 'bundle check'
  exit 1 unless system 'bundle install'
end

require 'bundler/setup'
require 'rake/gempackagetask'
require 'spec/rake/spectask'

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
  desc "Run all specs in spec directory"
  Spec::Rake::SpecTask.new(:spec) do |t|
    t.spec_files = FileList['spec/**/*_spec.rb']
    t.spec_opts = ['--color']
  end

  desc "Run rcov on the spec files"
  Spec::Rake::SpecTask.new(:coverage) do |t|
    t.spec_files = FileList['spec/**/*_spec.rb']
    t.spec_opts = ['--color']
    t.rcov = true
    t.rcov_opts = ['--exclude',
      'spec\/spec,bin\/spec,examples,\/var\/lib\/gems,\/Library\/Ruby,\.autotest']
  end
end

spec = eval(File.read('ticgit.gemspec'))
Rake::GemPackageTask.new(spec) do |pkg|
  pkg.need_tar = true
end

desc "Clean out the coverage and pkg directories"
task :clean do
  rm_rf 'coverage'
  rm_rf 'pkg'
  rm Dir.glob('ticgit*gem')
end

task :make => "pkg/#{spec.name}-#{spec.version}.gem" do
  puts "Generated #{spec.name}-#{spec.version}.gem"
end

task :install do
  system "gem install pkg/#{spec.name}-#{spec.version}.gem"
end

task :default => [:make, :install]
