namespace :metric do
  desc 'committed changes per file according to git'
  task 'changes' do
    $stdout.sync = true
    out = lambda{|changes, rb| puts("%4d %s" % [changes, rb]) }
    changes = {}

    print 'counting changes '

    Dir.glob 'lib/**/*.rb' do |rb|
      count = `git log --pretty=oneline '#{rb}'`.count("\n")
      print '.'
      # out[changes, rb]
      changes[rb] = count
    end
    puts ' done.'

    sorted = changes.sort_by{|r,c| c }.reverse

    top = sorted.first(20)
    unless top.empty?
      puts "Top 20:"
      top.each{|(r,c)| out[c,r] }
    end

    bottom = sorted.last(20) - top
    unless bottom.empty?
      puts "Bottom 20:"
      bottom.each{|(r,c)| out[c,r] }
    end
  end
end
