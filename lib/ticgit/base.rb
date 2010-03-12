module TicGit
  class NoRepoFound < StandardError;end
  class Base

    attr_reader :git, :logger
    attr_reader :tic_working, :tic_index
    attr_reader :tickets, :last_tickets, :current_ticket  # saved in state
    attr_reader :config
    attr_reader :state, :config_file

    def initialize(git_dir, opts = {})
      @git = Git.open(find_repo(git_dir))
      @logger = opts[:logger] || Logger.new(STDOUT)
      @last_tickets = []

      proj = Ticket.clean_string(@git.dir.path)

      @tic_dir = opts[:tic_dir] || '~/.ticgit'
      @tic_working = opts[:working_directory] || File.expand_path(File.join(@tic_dir, proj, 'working'))
      @tic_index = opts[:index_file] || File.expand_path(File.join(@tic_dir, proj, 'index'))

      # load config file
      @config_file = File.expand_path(File.join(@tic_dir, proj, 'config.yml'))
      if File.exists?(config_file)
        @config = YAML.load(File.read(config_file))
      else
        @config = {}
      end

      @state = File.expand_path(File.join(@tic_dir, proj, 'state'))

      if File.file?(@state)
        load_state
      else
        reset_ticgit
      end
    end

    def find_repo(dir)
      full = File.expand_path(dir)
      ENV["GIT_WORKING_DIR"] || loop do
        return full if File.directory?(File.join(full, ".git"))
        raise NoRepoFound if full == full=File.dirname(full)
      end
    end

    # marshal dump the internals
    # save config file
    def save_state
      state_list = [@tickets, @last_tickets, @current_ticket]
      File.open(@state, 'w+'){|io| Marshal.dump(state_list, io) }
      File.open(@config_file, 'w+'){|io| io.write(config.to_yaml) }
    end

    # read in the internals
    def load_state
      state_list = File.open(@state){|io| Marshal.load(io) }
      @tickets, @last_tickets, @current_ticket = state_list
    end

    # returns new Ticket
    def ticket_new(title, options = {})
      t = TicGit::Ticket.create(self, title, options)
      reset_ticgit
      TicGit::Ticket.open(self, t.ticket_name, @tickets[t.ticket_name])
    end

    def reset_ticgit
      load_tickets
      save_state
    end

    # returns new Ticket
    def ticket_comment(comment, ticket_id = nil)
      if t = ticket_revparse(ticket_id)
        ticket = TicGit::Ticket.open(self, t, @tickets[t])
        ticket.add_comment(comment)
        reset_ticgit
      end
    end

    # returns array of Tickets
    def ticket_list(options = {})
      reset_ticgit
      ts = []
      @last_tickets = []
      @config['list_options'] ||= {}

      @tickets.to_a.each do |name, t|
        ts << TicGit::Ticket.open(self, name, t)
      end

      if name = options[:saved]
         if c = config['list_options'][name]
           options = c.merge(options)
         end
      end

      if options[:list]
        # TODO : this is a hack and i need to fix it
        config['list_options'].each do |name, opts|
          puts name + "\t" + opts.inspect
        end
        return false
      end

      if options.size == 0
        # default list
        options[:state] = 'open'
      end

      # :tag, :state, :assigned
      if t = options[:tags]
        t = {false => Set.new, true => Set.new}.merge t.classify { |x| x[0,1] != "-" }
        t[false].map! { |x| x[1..-1] }
        ts = ts.reject { |tic| t[true].intersection(tic.tags).empty? } unless t[true].empty?
        ts = ts.select { |tic| t[false].intersection(tic.tags).empty? } unless t[false].empty?
      end
      if s = options[:states]
        s = {false => Set.new, true => Set.new}.merge s.classify { |x| x[0,1] != "-" }
        s[true].map! { |x| Regexp.new(x, Regexp::IGNORECASE) }
        s[false].map! { |x| Regexp.new(x[1..-1], Regexp::IGNORECASE) }
        ts = ts.select { |tic| s[true].any? { |st| tic.state =~ st } } unless s[true].empty?
        ts = ts.reject { |tic| s[false].any? { |st| tic.state =~ st } } unless s[false].empty?
      end
      if a = options[:assigned]
        ts = ts.select { |tic| tic.assigned =~ Regexp.new(a, Regexp::IGNORECASE) }
      end

      # SORTING
      if field = options[:order]
        field, type = field.split('.')

        case field
        when 'assigned'; ts = ts.sort_by{|a| a.assigned }
        when 'state';    ts = ts.sort_by{|a| a.state }
        when 'date';     ts = ts.sort_by{|a| a.opened }
        when 'title';    ts = ts.sort_by{|a| a.title }
        end

        ts = ts.reverse if type == 'desc'
      else
        # default list
        ts = ts.sort_by{|a| a.opened }
      end

      if options.size == 0
        # default list
        options[:state] = 'open'
      end

      # :tag, :state, :assigned
      if t = options[:tag]
        ts = ts.select { |tag| tag.tags.include?(t) }
      end
      if s = options[:state]
        ts = ts.select { |tag| tag.state =~ /#{s}/ }
      end
      if a = options[:assigned]
        ts = ts.select { |tag| tag.assigned =~ /#{a}/ }
      end

      if save = options[:save]
        options.delete(:save)
        @config['list_options'][save] = options
      end

      @last_tickets = ts.map{|t| t.ticket_name }
      # :save

      save_state
      ts
    end

    # returns single Ticket
    def ticket_show(ticket_id = nil)
      # ticket_id can be index of last_tickets, partial sha or nil => last ticket
      reset_ticgit
      if t = ticket_revparse(ticket_id)
        return TicGit::Ticket.open(self, t, @tickets[t])
      end
    end

    # returns recent ticgit activity
    # uses the git logs for this
    def ticket_recent(ticket_id = nil)
      if ticket_id
        t = ticket_revparse(ticket_id)
        return git.log.object('ticgit').path(t)
      else
        return git.log.object('ticgit')
      end
    end

    def ticket_revparse(ticket_id)
      if ticket_id
        ticket_id = ticket_id.strip

        if /^[0-9]*$/ =~ ticket_id
          if t = @last_tickets[ticket_id.to_i - 1]
            return t
          end
        else # partial or full sha
          regex = /^#{Regexp.escape(ticket_id)}/
          ch = @tickets.select{|name, t|
            t['files'].assoc('TICKET_ID')[1] =~ regex }
          ch.first[0] if ch.first
        end
      elsif(@current_ticket)
        return @current_ticket
      end
    end

    def ticket_tag(tag, ticket_id = nil, options = {})
      if t = ticket_revparse(ticket_id)
        ticket = TicGit::Ticket.open(self, t, @tickets[t])
        if options.remove
          ticket.remove_tag(tag)
        else
          ticket.add_tag(tag)
        end
        reset_ticgit
      end
    end

    def ticket_change(new_state, ticket_id = nil)
      if t = ticket_revparse(ticket_id)
        if tic_states.include?(new_state)
          ticket = TicGit::Ticket.open(self, t, @tickets[t])
          ticket.change_state(new_state)
          reset_ticgit
        end
      end
    end

    def ticket_assign(new_assigned = nil, ticket_id = nil)
      if t = ticket_revparse(ticket_id)
        ticket = TicGit::Ticket.open(self, t, @tickets[t])
        ticket.change_assigned(new_assigned)
        reset_ticgit
      end
    end

    def ticket_points(new_points = nil, ticket_id = nil)
      if t = ticket_revparse(ticket_id)
        ticket = TicGit::Ticket.open(self, t, @tickets[t])
        ticket.change_points(new_points)
        reset_ticgit
      end
    end

    def ticket_checkout(ticket_id)
      if t = ticket_revparse(ticket_id)
        ticket = TicGit::Ticket.open(self, t, @tickets[t])
        @current_ticket = ticket.ticket_name
        save_state
      end
    end

    def comment_add(ticket_id, comment, options = {})
    end

    def comment_list(ticket_id)
    end

    def tic_states
      ['open', 'resolved', 'invalid', 'hold']
    end

    def sync_tickets
      Dir.chdir "../tidyapp_bugs" 
      #bs = git.lib.branches_all.map{|b| b.first }

      #unless bs.include?('ticgit') && File.directory?(@tic_working)
      #  init_ticgit_branch(bs.include?('ticgit'))
      #end
      
      puts "checking out ticgit"
      #in_branch(bs.include?('ticgit'))  do
      #puts git.branch('ticgit').checkout()
      #   puts git.pull('origin','origin/ticgit')
      #puts git.branch('master').checkout()
      #end
      
      in_branch('ticgit') do 
         #puts git.add('.')
         #puts git.commit('tickets update')
         puts git.pull('origin','origin/ticgit')
         #puts git.push('origin','origin/ticgit')
         puts
         puts "Tickets synchronized."
      end
       
    end

    def load_tickets
      @tickets = {}

      bs = git.lib.branches_all.map{|b| b.first }

      unless bs.include?('ticgit') && File.directory?(@tic_working)
        init_ticgit_branch(bs.include?('ticgit'))
      end

      tree = git.lib.full_tree('ticgit')
      tree.each do |t|
        data, file = t.split("\t")
        mode, type, sha = data.split(" ")
        tic = file.split('/')
        if tic.size == 2  # directory depth
          ticket, info = tic
          @tickets[ticket] ||= { 'files' => [] }
          @tickets[ticket]['files'] << [info, sha]
        end
      end
    end

    def init_ticgit_branch(ticgit_branch = false)
      @logger.info 'creating ticgit repo branch'

      in_branch(ticgit_branch) do
        new_file('.hold', 'hold')

        unless ticgit_branch
          git.add
          git.commit('creating the ticgit branch')
        end
      end
    end

    # temporarlily switches to ticgit branch for tic work
    def in_branch(branch_exists = true)
      needs_checkout = false

      unless File.directory?(@tic_working)
        FileUtils.mkdir_p(@tic_working)
        needs_checkout = true
      end

      needs_checkout = true unless File.file?('.hold')

      old_current = git.lib.branch_current
      begin
        git.lib.change_head_branch('ticgit')
        git.with_index(@tic_index) do
          git.with_working(@tic_working) do |wd|
            git.lib.checkout('ticgit') if needs_checkout && branch_exists
            yield wd
          end
        end
      ensure
        git.lib.change_head_branch(old_current)
      end
    end

    def new_file(name, contents)
      File.open(name, 'w+'){|f| f.puts(contents) }
    end

  end
end
