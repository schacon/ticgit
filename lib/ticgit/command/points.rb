module TicGit
  class CLI
    # Assigns points to a ticket
    #
    # Usage:
    # ti points {1} {points}   (assigns points to a specified ticket)
    module Points
      def parser
        OptionParser.new do |opts|
          opts.banner = "ti points [ticket_id] points\t(assigns points to a specified ticket"
        end
      end

      def execute
        case args.size
        when 1
          new_points = args[0].strip
        when 2
          tid = args[0]
          new_points = args[0].strip
        else
          puts parser
          exit 1
        end

        tic.ticket_points(new_points, tid)
      end
    end
  end
end
