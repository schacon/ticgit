namespace :release do
  task :all => [:release_github, :release_rubyforge]

  desc 'Display instructions to release on github'
  task :github => [:reversion, :gemspec] do
    name, version = GEMSPEC.name, GEMSPEC.version

    puts <<INSTRUCTIONS
First add the relevant files:

git add MANIFEST CHANGELOG #{name}.gemspec lib/#{name}/version.rb

Then commit them, tag the commit, and push:

git commit -m 'Version #{version}'
git tag -a -m '#{version}' '#{version}'
git push

INSTRUCTIONS

  end

  # TODO: Not tested
  desc 'Display instructions to release on rubyforge'
  task :rubyforge => [:reversion, :gemspec, :package] do
    name, version = GEMSPEC.name, GEMSPEC.version

    puts <<INSTRUCTIONS
To publish to rubyforge do following:

rubyforge login
rubyforge add_release #{name} #{version} pkg/#{name}-#{version}.gem

After you have done these steps, see:

rake release:rubyforge_archives

INSTRUCTIONS
  end

  desc 'Display instructions to add archives after release:rubyforge'
  task :rubyforge_archives do
    release_id = latest_release_id

    puts "Adding archives for distro packagers is:", ""

    Dir["pkg/#{name}-#{version}.{gz,zip}"].each do |file|
      puts "rubyforge add_file #{name} #{name} #{release_id} '#{file}'"
    end

    puts
  end
end

# Use URI and proper XPATH, something along these lines:
#
# a = doc.at('a[@href=~"release_id"]')[:href]
# release_id = URI(a).query[/release_id=(\w+)/, 1]
def latest_release_id
  require 'open-uri'
  require 'hpricot'

  url = "http://rubyforge.org/frs/?group_id=#{PROJECT_RUBYFORGE_GROUP_ID}"
  doc = Hpricot(open(url))
  a = (doc/:a).find{|a| a[:href] =~ /release_id/}

  version = a.inner_html
  release_id = Hash[*a[:href].split('?').last.split('=').flatten]['release_id']
end
