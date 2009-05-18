module TicGit
  class CLI
    module Show
      def execute
        t = tic.ticket_show(args[0])
        ticket_show(t) if t
      end
    end
  end
end
