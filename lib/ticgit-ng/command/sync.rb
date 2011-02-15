module TicGitNG
  module Command
    module Sync
      def parser(opts)
        opts.banner = "Usage: ti sync [options]"
        opts.on_head(
          "-r REPO", "--repo REPO", "Sync ticgit-ng branch with REPO"){|v|
          options.repo = v
        }
        opts.on_head(
          "-p y/n", "--push yes/no", "Push to the remote repo? (Yes or No)"){|v|
          if v=~ /y/
            options.push= true
          elsif v=~ /n/
            options.push= false
          end
        }
      end

      def execute
        tic.sync_tickets(options.repo, options.push)
      end
    end
  end
end
