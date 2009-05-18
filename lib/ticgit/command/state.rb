module TicGit
  class CLI
    module State
      def execute
        if args.size > 1
          tid, new_state = args[0].strip, args[1].strip

          if valid_state?(new_state)
            tic.ticket_change(new_state, tid)
          else
            puts 'Invalid State - please choose from : ' + tic.tic_states.join(", ")
          end
        elsif args.size > 0
          # new state
          new_state = args[0].chomp

          if valid_state?(new_state)
            tic.ticket_change(new_state)
          else
            puts 'Invalid State - please choose from : ' + tic.tic_states.join(", ")
          end
        else
          puts 'You need to at least specify a new state for the current ticket'
        end
      end

      def valid_state?(state)
        tic.tic_states.include?(state)
      end
    end
  end
end
