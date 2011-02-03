desc "show a todolist from all the TODO tags in the source"
task :todo do
  Dir.glob '{lib,spec}/**/*.rb' do |file|
    lastline = todo = comment = long_comment = false

    File.readlines(file).each_with_index do |line, lineno|
      lineno += 1
      comment = line =~ /^\s*?#.*?$/
      long_comment = line =~ /^=begin/
      long_comment = line =~ /^=end/
      todo = true if line =~ /TODO/ and (long_comment or comment)
      todo = false if line.gsub('#', '').strip.empty?
      todo = false unless comment or long_comment
      if todo
        unless lastline and lastline + 1 == lineno
          puts
          puts "vim #{file} +#{lineno}"
        end

        l = line.strip.gsub(/^#\s*/, '')
        print '  ' unless l =~ /^-/
        puts l
        lastline = lineno
      end
    end
  end
end
