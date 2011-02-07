begin
  require 'grancher/task'

  Grancher::Task.new do |g|
    g.branch = 'gh-pages'
    g.push_to = 'origin'
    g.message = 'Updated website'
    g.directory 'ydoc', 'doc'
  end
rescue LoadError
  # oh well :)
end
