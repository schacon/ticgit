# Once git has a fix for the glibc in handling .mailmap and another fix for
# allowing empty mail address to be mapped in .mailmap we won't have to handle
# them manually.

desc 'Update AUTHORS'
task :authors do
  authors = Hash.new(0)

  `git shortlog -nse`.scan(/(\d+)\s(.+)\s<(.*)>$/) do |count, name, email|
    case name
    when "ahoward"
      name, email = "Ara T. Howard", "ara.t.howard@gmail.com"
    when "Martin Hilbig blueonyx@dev-area.net"
      name, email = "Martin Hilbig", "blueonyx@dev-area.net"
    when "Michael Fellinger m.fellinger@gmail.com"
      name, email = "Michael Fellinger", "m.fellinger@gmail.com"
    end

    authors[[name, email]] += count.to_i
  end

  File.open('AUTHORS', 'w+') do |io|
    io.puts "Following persons have contributed to #{GEMSPEC.name}."
    io.puts '(Sorted by number of submitted patches, then alphabetically)'
    io.puts ''
    authors.sort_by{|(n,e),c| [-c, n.downcase] }.each do |(name, email), count|
      io.puts("%6d %s <%s>" % [count, name, email])
    end
  end
end
