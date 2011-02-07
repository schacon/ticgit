task :gem_installer do
  class GemInstaller
    def initialize(options = {}, &block)
      @gems = []
      @options = options

      run(&block)
    end

    def run(&block)
      instance_eval(&block) if block_given?
    end

    def gem(name, version = nil, options = {})
      if version.respond_to?(:merge!)
        options = version
      else
        options[:version] = version
      end

      @gems << [name, options]
    end

    def setup_gemspec(gemspec)
      gemspec.dependencies.each do |dependency|
        dependency.version_requirements.as_list.each do |version|
          gem(dependency.name, version)
        end
      end

      setup
    end

    def setup
      require 'rubygems'
      require 'rubygems/dependency_installer'

      @gems.each do |name, options|
        setup_gem(name, options)
      end
    end

    def setup_gem(name, options, try_install = true)
      print "activating #{name} ... "
      Gem.activate(name, *[options[:version]].compact)
      require(options[:lib] || name)
      puts "success."
    rescue LoadError => error
      puts error
      install_gem(name, options) if try_install
      setup_gem(name, options, try_install = false)
    end

    def install_gem(name, options)
      installer = Gem::DependencyInstaller.new(options)

      temp_argv(options[:extconf]) do
        print "Installing #{name} ... "
        installer.install(name, options[:version])
        puts "done."
      end
    end

    def temp_argv(extconf)
      if extconf ||= @options[:extconf]
        old_argv = ARGV.clone
        ARGV.replace(extconf.split(' '))
      end

      yield

    ensure
      ARGV.replace(old_argv) if extconf
    end
  end
end
