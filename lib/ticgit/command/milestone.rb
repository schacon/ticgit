module TicGit
  class CLI
    # tic milestone
    # tic milestone migration1 (list tickets)
    # tic milestone -n migration1 3/4/08 (new milestone)
    # tic milestone -a {1} (add ticket to milestone)
    # tic milestone -d migration1 (delete)
    module Milestone
      def parser
        OptionParser.new do |opts|
          opts.banner = "Usage: ti milestone [milestone_name] [options] [date]"
          opts.on("-n MILESTONE", "--new MILESTONE", "Add a new milestone to this project") do |v|
            options.new = v
          end
          opts.on("-a TICKET", "--new TICKET", "Add a ticket to this milestone") do |v|
            options.add = v
          end
          opts.on("-d MILESTONE", "--delete MILESTONE", "Remove a milestone") do |v|
            options.remove = v
          end
        end
      end
    end
  end
end
