module TicGit
  class CLI
    module Tag
      def parser
        OptionParser.new do |opts|
          opts.banner = "Usage: ti tag [tic_id] [options] [tag_name] "
          opts.on("-d", "Remove this tag from the ticket") do |v|
            options.remove = v
          end
        end
      end

      def execute
        if options.remove
          puts 'remove'
        end

        if ARGV.size > 2
          tid = ARGV[1].chomp
          tic.ticket_tag(ARGV[2].chomp, tid, options)
        elsif ARGV.size > 1
          tic.ticket_tag(ARGV[1], nil, options)
        else
          puts 'You need to at least specify one tag to add'
          puts
          puts parser
        end
      end
    end
  end
end
