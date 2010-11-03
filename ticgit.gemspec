Gem::Specification.new do |s|
    s.platform  =   Gem::Platform::RUBY
    s.name      =   'ticgit-ng'
    s.version   =   '0.4.0'
    s.summary   =   'A distributed ticketing system for Git projects.'
    s.files     =   %w[bin/ti
                       bin/ticgitweb
                       lib/ticgit.rb
                       lib/ticgit/base.rb
                       lib/ticgit/cli.rb
                       lib/ticgit/comment.rb
                       lib/ticgit/ticket.rb]
    s.bindir = 'bin'
    s.executables = %w[ti ticgitweb]
    s.default_executable = 'ti'

    s.add_dependency('git', '>= 1.0.5')
    s.require_paths = %w[lib bin]
end
