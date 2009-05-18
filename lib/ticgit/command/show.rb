module TicGit
  module Command
    module Show
      def parser(opts)
        opts.banner = "Usage: ti show [ticid]"
      end

      def execute
        t = tic.ticket_show(args[0])
        ticket_show(t) if t
      end
    end
  end
end
