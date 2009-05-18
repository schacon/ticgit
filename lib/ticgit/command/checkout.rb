module TicGit
  class CLI
    module Checkout
      def parser
        OptionParser.new do |opts|
          opts.banner = "ti checkout [ticid]"
        end
      end

      def execute
        tid = args[0]
        tic.ticket_checkout(tid)
      end
    end
  end
end
