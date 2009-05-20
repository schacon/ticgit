require 'spec/helper'

describe TicGit::Base do
  behaves_like :clean_ticgit

  it 'has four ticket states' do
    @ticgit.tic_states.size.should == 4
  end

  reset

  it 'creates new tickets' do
    @ticgit.tickets.size.should == 0
    @ticgit.ticket_new('my new ticket').class.should == TicGit::Ticket
    @ticgit.tickets.size.should == 1
  end

  reset

  it 'lists existing tickets' do
    @ticgit.ticket_new('my new ticket')
    @ticgit.ticket_new('my second ticket')
    list = @ticgit.ticket_list
    list.first.class.should == TicGit::Ticket
    list.size.should == 2
  end

  describe 'assigning tickets' do
    behaves_like :clean_ticgit

    it 'reassigns tickets' do
      @ticgit.ticket_new('my new ticket')
      @ticgit.ticket_new('my second ticket')
      tic = @ticgit.ticket_list.first

      @ticgit.ticket_assign('pope', tic.ticket_id)
      tic = @ticgit.ticket_show(tic.ticket_id)
      tic.assigned.should == 'pope'
    end

    it "doesn't reassign ticket to the user it is already assigned to" do
      @ticgit.ticket_new('my new ticket')
      @ticgit.ticket_new('my second ticket')
      tic = @ticgit.ticket_list.first

      tic_id = tic.ticket_id
      lambda{
        @ticgit.ticket_assign(tic.assigned, tic_id)
        @ticgit.ticket_show(tic_id)
      }.should.not.change{ @ticgit.ticket_recent(tic_id).size }
    end

    it 'reassigns tickets to the current user by default' do
      @ticgit.ticket_new('my new ticket')
      @ticgit.ticket_new('my second ticket')
      tic = @ticgit.ticket_list.first

      @ticgit.ticket_checkout(tic.ticket_id)
      @ticgit.ticket_assign
      tic = @ticgit.ticket_show(tic.ticket_id)
      tic.assigned.should == tic.email
    end
  end

  describe 'ticket states' do
    behaves_like :clean_ticgit

    it "changes state of a ticket" do
      @ticgit.ticket_new('my new ticket')
      @ticgit.ticket_new('my second ticket')
      tic = @ticgit.ticket_list.first
      @ticgit.ticket_change('resolved', tic.ticket_id)
      tic = @ticgit.ticket_show(tic.ticket_id)
      tic.state.should == 'resolved'
    end

    it "won't change the state of a ticket to something invalid" do
      @ticgit.ticket_new('my new ticket')
      @ticgit.ticket_new('my second ticket')
      tic = @ticgit.ticket_list.first
      @ticgit.ticket_change('resolve', tic.ticket_id)
      tic = @ticgit.ticket_show(tic.ticket_id)
      tic.state.should.not == 'resolve'
    end

    it "only shows open tickets by default" do
      @ticgit.ticket_new('my third ticket')
      tics = @ticgit.ticket_list
      states = tics.map{ |t| t.state }.uniq
      states.should == ['open']
    end
  end

  describe "filtering tickets" do
    behaves_like :clean_ticgit

    it "filters tickets on state" do
      @ticgit.ticket_new('my new ticket')
      @ticgit.ticket_new('my second ticket')
      @ticgit.ticket_new('my third ticket')

      tic = @ticgit.ticket_list.first
      @ticgit.ticket_change('resolved', tic.ticket_id)

      tics = @ticgit.ticket_list(:state => 'resolved')
      tics.size.should == 1

      tics = @ticgit.ticket_list(:state => 'open')
      tics.size.should == 2
    end

    reset

    it "saves and recalls filtered ticket lists" do
      @ticgit.ticket_new('my new ticket')
      @ticgit.ticket_new('my second ticket')

      tic = @ticgit.ticket_list.first
      @ticgit.ticket_change('resolved', tic.ticket_id)

      tics = @ticgit.ticket_list(:state => 'resolved', :save => 'resolve')
      tics.size.should == 1

      rtics = @ticgit.ticket_list(:saved => 'resolve')
      tics.size.should == 1
    end
  end

  describe "commenting on tickets" do
    behaves_like :clean_ticgit

    it "comments on tickets" do
      t = @ticgit.ticket_new('my fourth ticket')
      t.comments.size.should == 0

      @ticgit.ticket_comment('my new comment', t.ticket_id)
      t = @ticgit.ticket_show(t.ticket_id)
      t.comments.size.should == 1
      t.comments.first.comment.should == 'my new comment'
    end
  end

  it "should retrieve specific tickets" do
    t = @ticgit.ticket_new('my fourth ticket')
    tid = @ticgit.ticket_list.last.ticket_id
    tic = @ticgit.ticket_show(tid)
    tic.ticket_id.should == tid
  end

  it "should be able to checkout a ticket" do
    t = @ticgit.ticket_new('my fourth ticket')
    tid = @ticgit.ticket_list.last.ticket_id
    @ticgit.ticket_checkout(tid)
    @ticgit.ticket_show.ticket_id.should == tid
  end

  it "should resolve partial shas into ticket" do
    t = @ticgit.ticket_new('my fourth ticket')
    tid = @ticgit.ticket_list.last.ticket_id
    @ticgit.ticket_checkout(tid[0, 5])
    @ticgit.ticket_show.ticket_id.should == tid

    @ticgit.ticket_checkout(tid[0, 20])
    @ticgit.ticket_show.ticket_id.should == tid
  end

  it "should resolve order number from most recent list into ticket" do
    @ticgit.ticket_new('my new ticket')
    @ticgit.ticket_new('my second ticket')
    tics = @ticgit.ticket_list(:state => 'open')
    @ticgit.ticket_show('1').ticket_id.should == tics[0].ticket_id
    @ticgit.ticket_show('2').ticket_id.should == tics[1].ticket_id
  end

  describe "tagging tickets" do
    behaves_like :clean_ticgit

    it "should be able to tag a ticket" do
      @ticgit.ticket_new('my new ticket')
      @ticgit.ticket_new('my second ticket')
      t = @ticgit.ticket_list.last
      t.tags.size.should == 0
      @ticgit.ticket_tag('newtag', t.ticket_id)
      t = @ticgit.ticket_show(t.ticket_id)
      t.tags.size.should == 1
      t.tags.first.should == 'newtag'
    end

    it "should not be able to tag a ticket with a blank tag" do
      t = @ticgit.ticket_new('my fourth ticket', :tags => [' '])
      t.tags.size.should == 0

      @ticgit.ticket_tag(' ', t.ticket_id)
      t = @ticgit.ticket_show(t.ticket_id)
      t.tags.size.should == 0

      @ticgit.ticket_tag('', t.ticket_id)
      t = @ticgit.ticket_show(t.ticket_id)
      t.tags.size.should == 0

      @ticgit.ticket_tag(',mytag', t.ticket_id)
      t = @ticgit.ticket_show(t.ticket_id)
      t.tags.size.should == 1
      t.tags.first.should == 'mytag'
    end

    it "should be able to remove a tag from a ticket" do
      t = @ticgit.ticket_new('my next ticket', :tags => ['scotty', 'chacony'])
      t.tags.size.should == 2

      @ticgit.ticket_tag('scotty', t.ticket_id, :remove => true)
      t.tags.size.should == 2
      t.tags.first.should == 'chacony'
    end
  end

  it "should save state to disk after a new ticket" do
    time = File.stat(@ticgit.state).size
    t = @ticgit.ticket_new('my next ticket', :tags => ['scotty', 'chacony'])
    File.stat(@ticgit.state).size.should.not == time
  end

  describe "estimating ticket points" do
    behaves_like :clean_ticgit

    it "should be able to change the points of a ticket" do
      @ticgit.ticket_new('my new ticket')
      tic = @ticgit.ticket_list.first
      tic.state.should.not == 3
      @ticgit.ticket_points(3, tic.ticket_id)
      tic = @ticgit.ticket_show(tic.ticket_id)
      tic.points.should == 3
    end
  end
end
