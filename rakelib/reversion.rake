desc "update version.rb"
task :reversion do
  File.open("lib/#{GEMSPEC.name}/version.rb", 'w+') do |file|
    file.puts("module #{PROJECT_MODULE}")
    file.puts('  VERSION = %p' % GEMSPEC.version.to_s)
    file.puts('end')
  end
end
