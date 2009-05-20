begin
  require 'yard'

  task :ycov => ['.yardoc'] do
    YARD::Registry.load_yardoc
    code_objects = YARD::Registry.paths.map{|path| YARD::Registry.at(path) }
    without_doc, with_doc = code_objects.partition{|obj| obj.docstring.empty? }

    documented = with_doc.size
    undocumented = without_doc.size
    total = documented + undocumented
    percentage = (documented / 0.01) / total

    puts "Documentation coverage is %d/%d (%3.1f%%)" % [documented, total, percentage]
  end

  file '.yardoc' => FileList['lib/**/*.rb'] do
    files = ['lib/**/*.rb']
    options = ['--no-output', '--private']
    YARD::CLI::Yardoc.run(*(options + files))
  end
end
