module TicGit
  module Command
    module Recent
      def parser(opts)
        opts.banner = 'Usage: ti recent'
      end

      def execute
        # "args[0]" seems to be superfluous.  It's usage
        # is undocumented, and supplying an argument
        # doesn't seem to do anything.
        tic.ticket_recent(args[0]).each do |commit|
          sha = commit.sha[0, 7]
          date = commit.date.strftime("%m/%d %H:%M")
          message = commit.message

          puts "#{sha}  #{date}\t#{message}"
        end
      end
    end
  end
end
