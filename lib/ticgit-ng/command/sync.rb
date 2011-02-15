module TicGitNG
  module Command
    module Sync
      def parser(opts)
        opts.banner = "Usage: ti sync"
      end

      def execute
        tic.sync_tickets()
      end
    end
  end
end
