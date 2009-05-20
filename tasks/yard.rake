desc 'Generate YARD documentation'
task :yard => :clean do
  sh("yardoc -o ydoc -r #{PROJECT_README}")
end
