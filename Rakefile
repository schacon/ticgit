require 'rubygems'
require 'rake'
require 'rake/clean'
require 'rake/rdoctask'
#require 'spec/rake/spectask'

CLEAN.include('**/*.gem')

desc "Create the postgis_adapter gem"
task :create_gem => [:clean] do
  spec = eval(IO.read('ticgit-ng.gemspec'))
  gem = Gem::Builder.new(spec).build
  Dir.mkdir("pkg") unless Dir.exists? "pkg"
  FileUtils.mv("#{File.dirname(__FILE__)}/#{gem}", "pkg")
end

Rake::RDocTask.new do |rdoc|
  version = File.exist?('VERSION') ? File.read('VERSION').chomp : ""
  rdoc.rdoc_dir = 'rdoc'
  rdoc.title = "postgis_adapter #{version}"
  rdoc.rdoc_files.include('README*')
  rdoc.rdoc_files.include('lib/**/*.rb')
end

task :default => :create_gem

# Spec::Rake::SpecTask.new(:spec) do |spec|
#   spec.libs << 'lib' << 'spec'
#   spec.spec_files = FileList['spec/**/*_spec.rb']
# end

# Spec::Rake::SpecTask.new(:rcov) do |spec|
#   spec.libs << 'lib' << 'spec'
#   spec.pattern = 'spec/**/*_spec.rb'
#   spec.rcov = true
# end
