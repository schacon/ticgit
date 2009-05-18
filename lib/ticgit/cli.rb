require 'ticgit'

# used Cap as a model for this - thanks Jamis

module TicGit
  class CLI
    COMMANDS = {}

    def self.register(mod_name, *commands)
      autoload(mod_name, "ticgit/command/#{mod_name.downcase}")
      commands.each{|cmd| COMMANDS[cmd] = mod_name }
    end

    register 'Assign', 'assign'
    register 'Attach', 'attach'
    register 'Checkout', 'checkout', 'co'
    register 'Comment', 'comment'
    register 'List', 'list'
    register 'Milestone','milestone'
    register 'New', 'new'
    register 'Points', 'points'
    register 'Recent', 'recent'
    register 'Show', 'show'
    register 'State', 'state'
    register 'Tag', 'tag'

    def self.execute
      parse(ARGV).execute!
    end

    def self.parse(args)
      cli = new(args)
      cli.parse_options!
      cli
    end

    attr_reader :action, :options, :args, :tic

    def initialize(args)
      @args = args.dup
      @tic = TicGit.open('.', :keep_state => true)
      @options = OpenStruct.new
      $stdout.sync = true # so that Net::SSH prompts show up
    rescue NoRepoFound
      puts "No repo found"
      exit
    end

    def execute!
      COMMANDS.each do |name, mod_name|
        if name === action
          mod = self.class.const_get(mod_name)
          extend(mod)

          if respond_to?(:parser)
            option_parser = parser
            option_parser.on('-h', '--help', 'Show this message'){
              puts option_parser
              exit
            }

            option_parser.parse!(args)
          end

          execute if respond_to?(:execute)

          exit
        end
      end

      puts 'not a command'
      usage
      exit
    end

    def parse_options! #:nodoc:
      if args.empty?
        warn "Please specify at least one action to execute."
        usage
        exit
      end

      @action = args.shift
    end

    def usage
      puts COMMANDS.keys.sort.join(' ')
    end

    def get_editor_message(message_file = nil)
      message_file = Tempfile.new('ticgit_message').path if !message_file

      editor = ENV["EDITOR"] || 'vim'
      system("#{editor} #{message_file}");
      message = File.readlines(message_file)
      message = message.select { |line| line[0, 1] != '#' } # removing comments
      if message.empty?
        return false
      else
        return message
      end
    end

    def ticket_show(t)
      days_ago = ((Time.now - t.opened) / (60 * 60 * 24)).round.to_s
      puts
      puts just('Title', 10) + ': ' + t.title
      puts just('TicId', 10) + ': ' + t.ticket_id
      puts
      puts just('Assigned', 10) + ': ' + t.assigned.to_s
      puts just('Opened', 10) + ': ' + t.opened.to_s + ' (' + days_ago + ' days)'
      puts just('State', 10) + ': ' + t.state.upcase
      if t.points == nil
        puts just('Points', 10) + ': no estimate'
      else
        puts just('Points', 10) + ': ' + t.points.to_s
      end
      if !t.tags.empty?
        puts just('Tags', 10) + ': ' + t.tags.join(', ')
      end
      puts
      if !t.comments.empty?
        puts 'Comments (' + t.comments.size.to_s + '):'
        t.comments.reverse.each do |c|
          puts '  * Added ' + c.added.strftime("%m/%d %H:%M") + ' by ' + c.user

          wrapped = c.comment.split("\n").collect do |line|
            line.length > 80 ? line.gsub(/(.{1,80})(\s+|$)/, "\\1\n").strip : line
          end * "\n"

          wrapped = wrapped.split("\n").map { |line| "\t" + line }
          if wrapped.size > 6
            puts wrapped[0, 6].join("\n")
            puts "\t** more... **"
          else
            puts wrapped.join("\n")
          end
          puts
        end
      end
    end

    class << self
      attr_accessor :window_lines, :window_cols

      TIOCGWINSZ_INTEL = 0x5413     # For an Intel processor
      TIOCGWINSZ_PPC   = 0x40087468 # For a PowerPC processor

      def reset_window_width
        try_using(TIOCGWINSZ_PPC) ||
        try_using(TIOCGWINSZ_INTEL) ||
          try_windows ||
          use_fallback
      end

      def try_using(mask)
        buf = [0,0,0,0].pack("S*")

        if $stdout.ioctl(mask, buf) >= 0
          self.window_lines, self.window_cols = buf.unpack("S2")
          true
        end
      rescue Errno::EINVAL
      end

      def try_windows
        lines, cols = windows_terminal_size
        self.window_lines, self.window_cols = lines, cols if lines and cols
      end

      STDOUT_HANDLE = 0xFFFFFFF5
      def windows_terminal_size
        m_GetStdHandle = Win32API.new(
          'kernel32', 'GetStdHandle', ['L'], 'L')
        m_GetConsoleScreenBufferInfo = Win32API.new(
          'kernel32', 'GetConsoleScreenBufferInfo', ['L', 'P'], 'L' )
        format = 'SSSSSssssSS'
        buf = ([0] * format.size).pack(format)
        stdout_handle = m_GetStdHandle.call(STDOUT_HANDLE)

        m_GetConsoleScreenBufferInfo.call(stdout_handle, buf)
        (bufx, bufy, curx, cury, wattr,
         left, top, right, bottom, maxx, maxy) = buf.unpack(format)
        return bottom - top + 1, right - left + 1
      end

      def use_fallback
        self.window_lines, self.window_cols = 25, 80
      end
    end

    def window_lines
      TicGit::CLI.window_lines
    end

    def window_cols
      TicGit::CLI.window_cols
    end

    def just(value, size, side = 'l')
      value = value.to_s
      if value.size > size
        value = value[0, size-1] + "\xe2\x80\xa6"
      end
      if side == 'r'
        return value.rjust(size)
      else
        return value.ljust(size)
      end
    end
  end
end

TicGit::CLI.reset_window_width
Signal.trap("SIGWINCH") { TicGit::CLI.reset_window_width }
