module TicGit
  class CLI
    module New
      def parser
        OptionParser.new do |opts|
          opts.banner = "Usage: ti new [options]"
          opts.on("-t TITLE", "--title TITLE",
                  "Title to use for the name of the new ticket"){|v|
            options.title = v
          }
        end
      end

      def execute
        if title = options.title
          ticket_show(tic.ticket_new(title, options))
        else
          # interactive
          message_file = Tempfile.new('ticgit_message').path
          File.open(message_file, 'w') do |f|
            f.puts "\n# ---"
            f.puts "tags:"
            f.puts "# first line will be the title of the tic, the rest will be the first comment"
            f.puts "# if you would like to add initial tags, put them on the 'tags:' line, comma delim"
          end
          if message = get_editor_message(message_file)
            title = message.shift
            if title && title.chomp.length > 0
              title = title.chomp
              if message.last[0, 5] == 'tags:'
                tags = message.pop
                tags = tags.gsub('tags:', '')
                tags = tags.split(',').map { |t| t.strip }
              end
              if message.size > 0
                comment = message.join("")
              end
              ticket_show(tic.ticket_new(title, :comment => comment, :tags => tags))
            else
              puts "You need to at least enter a title"
            end
          else
            puts "It seems you wrote nothing"
          end
        end
      end
    end
  end
end
