module TicGit
  class CLI
    module Recent
      def execute
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
