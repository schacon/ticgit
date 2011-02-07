desc 'code coverage'
task :rcov => :clean do
  specs = PROJECT_SPECS

  ignore = %w[ gem rack bacon innate hpricot nagoro/lib/nagoro ]

  if RUBY_VERSION >= '1.8.7'
    ignore << 'begin_with' << 'end_with'
  end
  if RUBY_VERSION < '1.9'
    ignore << 'fiber'
  end

  ignored = ignore.join(',')

  cmd = "rcov --aggregate coverage.data --sort coverage -t --%s -x '#{ignored}' %s"

  while spec = specs.shift
    puts '', "Gather coverage for #{spec} ..."
    html = specs.empty? ? 'html' : 'no-html'
    sh(cmd % [html, spec])
  end
end
