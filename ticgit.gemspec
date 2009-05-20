# -*- encoding: utf-8 -*-

Gem::Specification.new do |s|
  s.name = %q{ticgit}
  s.version = "2009.05.20"

  s.required_rubygems_version = Gem::Requirement.new(">= 0") if s.respond_to? :required_rubygems_version=
  s.authors = ["Scott Chacon", "Michael 'manveru' Fellinger"]
  s.date = %q{2009-05-20}
  s.default_executable = %q{ti}
  s.email = %q{m.fellinger@gmail.com}
  s.executables = ["ti", "ticgitweb"]
  s.files = [".gitignore", "AUTHORS", "CHANGELOG", "LICENSE", "README", "Rakefile", "bin/ti", "bin/ticgitweb", "examples/post-commit", "lib/ticgit.rb", "lib/ticgit/base.rb", "lib/ticgit/cli.rb", "lib/ticgit/command.rb", "lib/ticgit/command/assign.rb", "lib/ticgit/command/checkout.rb", "lib/ticgit/command/comment.rb", "lib/ticgit/command/list.rb", "lib/ticgit/command/milestone.rb", "lib/ticgit/command/new.rb", "lib/ticgit/command/points.rb", "lib/ticgit/command/recent.rb", "lib/ticgit/command/show.rb", "lib/ticgit/command/state.rb", "lib/ticgit/command/tag.rb", "lib/ticgit/comment.rb", "lib/ticgit/ticket.rb", "lib/ticgit/version.rb", "note/IMPLEMENT", "note/NOTES", "note/OUTPUT", "note/TODO", "spec/helper.rb", "spec/ticgit/base.rb", "spec/ticgit/cli.rb", "spec/ticgit/command/new.rb", "spec/ticgit/command/points.rb", "spec/ticgit/command/state.rb", "spec/ticgit/open.rb", "tasks/authors.rake", "tasks/bacon.rake", "tasks/changelog.rake", "tasks/cucumber.rake", "tasks/gem.rake", "tasks/gem_installer.rake", "tasks/git.rake", "tasks/grancher.rake", "tasks/install_dependencies.rake", "tasks/manifest.rake", "tasks/metric_changes.rake", "tasks/rcov.rake", "tasks/release.rake", "tasks/reversion.rake", "tasks/setup.rake", "tasks/todo.rake", "tasks/traits.rake", "tasks/yard.rake", "tasks/ycov.rake", "ticgit.gemspec"]
  s.has_rdoc = true
  s.homepage = %q{http://github.com/manveru/ticgit}
  s.require_paths = ["lib"]
  s.rubygems_version = %q{1.3.1}
  s.summary = %q{A distributed ticketing system for git projects.}

  if s.respond_to? :specification_version then
    current_version = Gem::Specification::CURRENT_SPECIFICATION_VERSION
    s.specification_version = 2

    if Gem::Version.new(Gem::RubyGemsVersion) >= Gem::Version.new('1.2.0') then
      s.add_runtime_dependency(%q<git>, [">= 1.0.5"])
    else
      s.add_dependency(%q<git>, [">= 1.0.5"])
    end
  else
    s.add_dependency(%q<git>, [">= 1.0.5"])
  end
end
