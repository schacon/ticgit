desc 'install dependencies'
task :install_dependencies => [:gem_installer] do
  GemInstaller.new do
    setup_gemspec(GEMSPEC)
  end
end
