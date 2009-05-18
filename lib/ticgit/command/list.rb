module TicGit
  class CLI
    module List
      ## LIST TICKETS ##
      def parser
        OptionParser.new do |opts|
          opts.banner = "Usage: ti list [options]"
          opts.on("-o ORDER", "--order ORDER",
                  "Field to order by - one of : assigned,state,date"){|v|
            options.order = v
          }

          opts.on("-t TAG[,TAG]", "--tags TAG[,TAG]", Array,
                  "List only tickets with specific tag(s)",
                  "Prefix the tag with '-' to negate"){|v|
            options.tags ||= Set.new
            options.tags.merge v
          }
          opts.on("-s STATE[,STATE]", "--states STATE[,STATE]", Array,
                  "List only tickets in a specific state(s)",
                  "Prefix the state with '-' to negate"){|v|
            options.states ||= Set.new
            options.states.merge v
          }
          opts.on("-a ASSIGNED", "--assigned ASSIGNED",
                  "List only tickets assigned to someone"){|v|
            options.assigned = v
          }
          opts.on("-S SAVENAME", "--saveas SAVENAME",
                  "Save this list as a saved name"){|v|
            options.save = v
          }
          opts.on("-l", "--list", "Show the saved queries"){|v|
            options.list = true
          }
        end
      end

      def execute
        options.saved = ARGV[1] if ARGV[1]

        if tickets = tic.ticket_list(options.to_hash)
          counter = 0
          cols = [80, window_cols].max

          puts
          puts [' ', just('#', 4, 'r'),
            just('TicId', 6),
            just('Title', cols - 56),
            just('State', 5),
            just('Date', 5),
            just('Assgn', 8),
            just('Tags', 20) ].join(" ")

          puts "-" * cols

          tickets.each do |t|
            counter += 1
            tic.current_ticket == t.ticket_name ? add = '*' : add = ' '
            puts [add, just(counter, 4, 'r'),
              t.ticket_id[0,6],
              just(t.title, cols - 56),
              just(t.state, 5),
              t.opened.strftime("%m/%d"),
              just(t.assigned_name, 8),
              just(t.tags.join(','), 20) ].join(" ")
          end
          puts
        end

      end
    end
  end
end
