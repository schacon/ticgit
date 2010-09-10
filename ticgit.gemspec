Gem::Specification.new do |s|
    s.platform  =   Gem::Platform::RUBY
    s.name      =   'ticgit'
    s.version   =   "0.3.7.#{Time.now.to_i}"
    s.date      =   Time.now.strftime '%Y-%m-%d'
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
    s.specification_version = 2 if s.respond_to? :specification_version=
end
