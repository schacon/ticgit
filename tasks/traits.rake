desc 'listing of available traits per class/module'
task 'traits' do
  nodes = Hash.new{|h,k| h[k] = []}
  Dir['lib/**/*.rb'].each do |file|
    content = File.read(file)
    traits = content.grep(/^\s*trait\s*:/)
    traits.each do |trait|
      space = content[0..content.index(trait)].scan(/^\s*(?:class|module)\s+(.*)$/)
      space = space.flatten.join('::')
      nodes[space] << trait.strip
    end
  end

  nodes.each do |space, traits|
    puts space
    traits.each do |trait|
      print '  ', trait, "\n"
    end
    puts
  end
end
