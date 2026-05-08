use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use ticgit_lib::{query, Filter, NewTicketOpts, Ticket, TicketState, TicketStore};
use time::format_description::well_known::Rfc3339;

use crate::commands::open_store;

#[derive(Debug, Parser)]
pub struct Args {}

pub fn run(_args: Args) -> Result<()> {
    let store = open_store()?;

    let mut terminal = init_terminal()?;
    let mut guard = TerminalGuard { active: true };
    let result = App::new(store)?.run(&mut terminal);
    guard.restore(&mut terminal)?;
    result
}

struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn restore(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    Ok(terminal)
}

struct App {
    store: TicketStore,
    tickets: Vec<Ticket>,
    visible: Vec<usize>,
    list_state: ListState,
    filter: String,
    mode: Mode,
    input: String,
    new_ticket: NewTicketDraft,
    detail: Option<usize>,
    status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Filter,
    Input(InputKind),
    State,
    Create,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Title,
    Description,
    Comment,
    AddTags,
    RemoveTags,
}

#[derive(Debug, Default)]
struct NewTicketDraft {
    title: String,
    description: String,
    tags: String,
    assigned: String,
    field: NewTicketField,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum NewTicketField {
    #[default]
    Title,
    Description,
    Tags,
    Assigned,
}

impl App {
    fn new(store: TicketStore) -> Result<Self> {
        let mut app = Self {
            store,
            tickets: Vec::new(),
            visible: Vec::new(),
            list_state: ListState::default(),
            filter: String::new(),
            mode: Mode::Normal,
            input: String::new(),
            new_ticket: NewTicketDraft::default(),
            detail: None,
            status: None,
        };
        app.reload(None)?;
        Ok(app)
    }

    fn reload(&mut self, preferred_id: Option<uuid::Uuid>) -> Result<()> {
        self.tickets = query::apply(
            self.store.list()?,
            &Filter {
                state: Some(TicketState::Open),
                ..Default::default()
            },
        );
        self.apply_filter();

        if let Some(id) = preferred_id {
            if let Some(visible_pos) = self
                .visible
                .iter()
                .position(|idx| self.tickets[*idx].id == id)
            {
                self.list_state.select(Some(visible_pos));
                if self.detail.is_some() {
                    self.detail = self.visible.get(visible_pos).copied();
                }
            } else if self.detail.is_some() {
                self.detail = None;
            }
        }

        Ok(())
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;

            if !event::poll(Duration::from_millis(250))? {
                continue;
            }

            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if self.handle_key(key)? {
                return Ok(());
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(area);

        self.draw_filter(frame, outer[0]);
        if self.detail.is_some() {
            let panes = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(outer[1]);
            self.draw_list(frame, panes[0]);
            self.draw_detail(frame, panes[1]);
        } else {
            self.draw_list(frame, outer[1]);
        }

        match self.mode {
            Mode::Input(kind) => self.draw_input_modal(frame, kind),
            Mode::State => self.draw_state_modal(frame),
            Mode::Create => self.draw_create_modal(frame),
            _ => {}
        }
    }

    fn draw_filter(&self, frame: &mut Frame<'_>, area: Rect) {
        let prompt = match self.mode {
            Mode::Filter => Line::from(vec![
                Span::styled("filter ", Style::default().fg(Color::Yellow)),
                Span::raw("/"),
                Span::styled(self.filter.as_str(), Style::default().fg(Color::Cyan)),
                Span::raw("  type to filter, Enter/Esc to finish"),
            ]),
            Mode::Input(kind) => Line::from(vec![
                Span::styled("editing ", Style::default().fg(Color::Yellow)),
                Span::raw(kind.label()),
                Span::raw("  Enter apply, Esc cancel"),
            ]),
            Mode::State => Line::from(vec![
                Span::styled("editing ", Style::default().fg(Color::Yellow)),
                Span::raw("state  choose in modal, Esc cancel"),
            ]),
            Mode::Create => Line::from(vec![
                Span::styled("new ", Style::default().fg(Color::Yellow)),
                Span::raw("Tab/↑/↓ fields  Enter create  Esc cancel"),
            ]),
            Mode::Normal => {
                let status = self.status.as_deref().unwrap_or(
                    "n new  / filter  Enter details  t title  d desc  c comment  s state  +/- tags  q quit",
                );
                Line::from(vec![
                    Span::styled("normal ", Style::default().fg(Color::Yellow)),
                    Span::raw(status),
                ])
            }
        };
        let paragraph =
            Paragraph::new(prompt).block(Block::default().borders(Borders::ALL).title("ti tui"));
        frame.render_widget(paragraph, area);
    }

    fn draw_list(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let count = self.visible.len();
        let title = if self.filter.is_empty() {
            format!("Open tickets ({count})")
        } else {
            format!("Open tickets matching \"{}\" ({count})", self.filter)
        };

        let items: Vec<ListItem<'_>> = self
            .visible
            .iter()
            .map(|&idx| {
                let ticket = &self.tickets[idx];
                let tags = if ticket.tags.is_empty() {
                    String::new()
                } else {
                    format!(
                        " [{}]",
                        ticket.tags.iter().cloned().collect::<Vec<_>>().join(",")
                    )
                };
                ListItem::new(Line::from(vec![
                    Span::styled(ticket.short_id(), Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::raw(ticket.title.as_str()),
                    Span::styled(tags, Style::default().fg(Color::Blue)),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn draw_detail(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(idx) = self.detail else {
            return;
        };
        let ticket = &self.tickets[idx];
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        let mut detail_lines = vec![
            field_line("Title", ticket.title.as_str()),
            field_line("Id", &ticket.id.to_string()),
            field_line(
                "Created",
                &format!(
                    "{} by {}",
                    ticket.created_at.format(&Rfc3339).unwrap_or_default(),
                    created_by_display(ticket)
                ),
            ),
            field_line("State", ticket.state.as_str()),
        ];
        if let Some(assigned) = &ticket.assigned {
            detail_lines.push(field_line("Assigned", assigned));
        }
        if let Some(points) = ticket.points {
            detail_lines.push(field_line("Points", &points.to_string()));
        }
        if let Some(milestone) = &ticket.milestone {
            detail_lines.push(field_line("Milestone", milestone));
        }
        if !ticket.tags.is_empty() {
            detail_lines.push(field_line(
                "Tags",
                &ticket.tags.iter().cloned().collect::<Vec<_>>().join(", "),
            ));
        }
        detail_lines.push(Line::raw(""));
        detail_lines.push(Line::from(Span::styled(
            "Description",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        if let Some(description) = &ticket.description {
            for line in description.lines() {
                detail_lines.push(Line::from(Span::raw(line.to_string())));
            }
        } else {
            detail_lines.push(Line::from(Span::styled(
                "(none)",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let detail = Paragraph::new(detail_lines)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        frame.render_widget(detail, chunks[0]);

        let comment_lines = if ticket.comments.is_empty() {
            vec![Line::from(Span::styled(
                "(no comments)",
                Style::default().fg(Color::DarkGray),
            ))]
        } else {
            ticket
                .comments
                .iter()
                .flat_map(|comment| {
                    let header = Line::from(vec![
                        Span::styled(comment.author.clone(), Style::default().fg(Color::Cyan)),
                        Span::raw("  "),
                        Span::styled(
                            comment.at.format(&Rfc3339).unwrap_or_default(),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]);
                    let mut lines = vec![header];
                    lines.extend(comment.body.lines().map(|line| Line::raw(line.to_string())));
                    lines.push(Line::raw(""));
                    lines
                })
                .collect()
        };
        let comments = Paragraph::new(comment_lines)
            .block(Block::default().borders(Borders::ALL).title("Comments"))
            .wrap(Wrap { trim: false });
        frame.render_widget(comments, chunks[1]);
    }

    fn draw_input_modal(&self, frame: &mut Frame<'_>, kind: InputKind) {
        let area = centered_rect(70, kind.modal_height(), frame.area());
        let title = format!("Edit {}", kind.label());
        let help = match kind {
            InputKind::Title => "Enter a new ticket title.",
            InputKind::Description => "Enter a new description. Empty clears it.",
            InputKind::Comment => "Enter a comment to append.",
            InputKind::AddTags => "Enter comma- or space-separated tags to add.",
            InputKind::RemoveTags => "Enter comma- or space-separated tags to remove.",
        };
        let lines = vec![
            Line::from(Span::styled(help, Style::default().fg(Color::DarkGray))),
            Line::raw(""),
            Line::from(self.input.as_str()),
            Line::raw(""),
            Line::from(Span::styled(
                "Enter apply  Esc cancel",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let modal = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, area);
        frame.render_widget(modal, area);
    }

    fn draw_state_modal(&self, frame: &mut Frame<'_>) {
        let area = centered_rect(42, 9, frame.area());
        let lines = vec![
            Line::from(vec![
                Span::styled("o", Style::default().fg(Color::Yellow)),
                Span::raw(" open"),
            ]),
            Line::from(vec![
                Span::styled("r", Style::default().fg(Color::Yellow)),
                Span::raw(" resolved"),
            ]),
            Line::from(vec![
                Span::styled("i", Style::default().fg(Color::Yellow)),
                Span::raw(" invalid"),
            ]),
            Line::from(vec![
                Span::styled("h", Style::default().fg(Color::Yellow)),
                Span::raw(" hold"),
            ]),
            Line::raw(""),
            Line::from(Span::styled(
                "Esc cancel",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let modal = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Change state"));
        frame.render_widget(Clear, area);
        frame.render_widget(modal, area);
    }

    fn draw_create_modal(&self, frame: &mut Frame<'_>) {
        let area = centered_rect(72, 15, frame.area());
        let lines = vec![
            new_ticket_field_line(
                NewTicketField::Title,
                self.new_ticket.field,
                "Title",
                self.new_ticket.title.as_str(),
                true,
            ),
            new_ticket_field_line(
                NewTicketField::Description,
                self.new_ticket.field,
                "Description",
                self.new_ticket.description.as_str(),
                false,
            ),
            new_ticket_field_line(
                NewTicketField::Tags,
                self.new_ticket.field,
                "Tags",
                self.new_ticket.tags.as_str(),
                false,
            ),
            new_ticket_field_line(
                NewTicketField::Assigned,
                self.new_ticket.field,
                "Assigned",
                self.new_ticket.assigned.as_str(),
                false,
            ),
            Line::raw(""),
            Line::from(Span::styled(
                "Tab/Up/Down switch fields  Enter create  Esc cancel",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let modal = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("New ticket"))
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, area);
        frame.render_widget(modal, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        let quit = match self.mode {
            Mode::Filter => {
                self.handle_filter_key(key);
                false
            }
            Mode::Input(_) => {
                self.handle_input_key(key)?;
                false
            }
            Mode::State => {
                self.handle_state_key(key)?;
                false
            }
            Mode::Create => {
                self.handle_create_key(key)?;
                false
            }
            Mode::Normal => self.handle_normal_key(key),
        };
        Ok(quit)
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> bool {
        self.status = None;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => true,
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                false
            }
            KeyCode::Char('n') => {
                self.begin_create();
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.next();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.previous();
                false
            }
            KeyCode::Enter => {
                self.open_selected();
                false
            }
            KeyCode::Char('t') => {
                self.begin_input(InputKind::Title);
                false
            }
            KeyCode::Char('d') => {
                self.begin_input(InputKind::Description);
                false
            }
            KeyCode::Char('c') => {
                self.begin_input(InputKind::Comment);
                false
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.begin_input(InputKind::AddTags);
                false
            }
            KeyCode::Char('-') => {
                self.begin_input(InputKind::RemoveTags);
                false
            }
            KeyCode::Char('s') => {
                if self.selected_ticket().is_some() {
                    self.mode = Mode::State;
                } else {
                    self.status = Some("Select a ticket first.".to_string());
                }
                false
            }
            _ => false,
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.apply_filter();
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.apply_filter();
            }
            _ => {}
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.input.clear();
                self.status = Some("Cancelled.".to_string());
            }
            KeyCode::Enter => {
                if self.submit_input()? {
                    self.mode = Mode::Normal;
                    self.input.clear();
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_state_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = Some("Cancelled.".to_string());
            }
            KeyCode::Char('o') | KeyCode::Char('1') => self.set_state(TicketState::Open)?,
            KeyCode::Char('r') | KeyCode::Char('2') => self.set_state(TicketState::Resolved)?,
            KeyCode::Char('i') | KeyCode::Char('3') => self.set_state(TicketState::Invalid)?,
            KeyCode::Char('h') | KeyCode::Char('4') => self.set_state(TicketState::Hold)?,
            _ => {}
        }
        Ok(())
    }

    fn handle_create_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.new_ticket = NewTicketDraft::default();
                self.status = Some("Cancelled.".to_string());
            }
            KeyCode::Tab | KeyCode::Down => self.new_ticket.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.new_ticket.previous_field(),
            KeyCode::Enter => {
                if self.create_ticket()? {
                    self.mode = Mode::Normal;
                    self.new_ticket = NewTicketDraft::default();
                }
            }
            KeyCode::Backspace => {
                self.new_ticket.current_value_mut().pop();
            }
            KeyCode::Char(c) => {
                self.new_ticket.current_value_mut().push(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn begin_create(&mut self) {
        self.new_ticket = NewTicketDraft::default();
        self.mode = Mode::Create;
    }

    fn begin_input(&mut self, kind: InputKind) {
        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            return;
        };

        self.input = match kind {
            InputKind::Title => ticket.title.clone(),
            InputKind::Description => ticket.description.clone().unwrap_or_default(),
            InputKind::Comment | InputKind::AddTags | InputKind::RemoveTags => String::new(),
        };
        self.mode = Mode::Input(kind);
    }

    fn submit_input(&mut self) -> Result<bool> {
        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            return Ok(false);
        };
        let id = ticket.id;
        let Mode::Input(kind) = self.mode else {
            return Ok(false);
        };

        match kind {
            InputKind::Title => {
                let title = self.input.trim();
                if title.is_empty() {
                    self.status = Some("Title cannot be empty.".to_string());
                    return Ok(false);
                }
                self.store.set_title(&id, title)?;
                self.status = Some("Updated title.".to_string());
            }
            InputKind::Description => {
                let description = self.input.trim();
                self.store.set_description(
                    &id,
                    if description.is_empty() {
                        None
                    } else {
                        Some(description)
                    },
                )?;
                self.status = Some("Updated description.".to_string());
            }
            InputKind::Comment => {
                let body = self.input.trim();
                if body.is_empty() {
                    self.status = Some("Comment cannot be empty.".to_string());
                    return Ok(false);
                }
                self.store.add_comment(&id, body)?;
                self.status = Some("Added comment.".to_string());
            }
            InputKind::AddTags => {
                let tags = split_tags(&self.input);
                if tags.is_empty() {
                    self.status = Some("Enter at least one tag.".to_string());
                    return Ok(false);
                }
                for tag in tags {
                    self.store.add_tag(&id, &tag)?;
                }
                self.status = Some("Added tag(s).".to_string());
            }
            InputKind::RemoveTags => {
                let tags = split_tags(&self.input);
                if tags.is_empty() {
                    self.status = Some("Enter at least one tag.".to_string());
                    return Ok(false);
                }
                for tag in tags {
                    self.store.remove_tag(&id, &tag)?;
                }
                self.status = Some("Removed tag(s).".to_string());
            }
        }

        self.reload(Some(id))?;
        Ok(true)
    }

    fn set_state(&mut self, state: TicketState) -> Result<()> {
        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            self.mode = Mode::Normal;
            return Ok(());
        };
        let id = ticket.id;
        self.store.set_state(&id, state)?;
        self.status = Some(format!("Changed state to {state}."));
        self.mode = Mode::Normal;
        self.reload(Some(id))?;
        Ok(())
    }

    fn create_ticket(&mut self) -> Result<bool> {
        let title = self.new_ticket.title.trim();
        if title.is_empty() {
            self.status = Some("Title cannot be empty.".to_string());
            return Ok(false);
        }

        let ticket = self.store.create(
            title,
            NewTicketOpts {
                comment: None,
                tags: split_tags(&self.new_ticket.tags),
                assigned: optional_trimmed(&self.new_ticket.assigned).map(ToString::to_string),
            },
        )?;
        let id = ticket.id;
        if let Some(description) = optional_trimmed(&self.new_ticket.description) {
            self.store.set_description(&id, Some(description))?;
        }

        self.filter.clear();
        self.detail = Some(0);
        self.reload(Some(id))?;
        self.open_ticket_by_id(id);
        self.status = Some(format!("Created {}.", ticket.short_id()));
        Ok(true)
    }

    fn apply_filter(&mut self) {
        let needle = self.filter.to_ascii_lowercase();
        self.visible = self
            .tickets
            .iter()
            .enumerate()
            .filter_map(|(idx, ticket)| {
                if needle.is_empty() || ticket_matches(ticket, &needle) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();

        self.detail = self.detail.filter(|idx| self.visible.contains(idx));
        if self.visible.is_empty() {
            self.list_state.select(None);
        } else {
            let selected = self
                .list_state
                .selected()
                .unwrap_or(0)
                .min(self.visible.len() - 1);
            self.list_state.select(Some(selected));
        }
    }

    fn next(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let selected = self.list_state.selected().unwrap_or(0);
        let next = (selected + 1) % self.visible.len();
        self.list_state.select(Some(next));
    }

    fn previous(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let selected = self.list_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| self.visible.len().saturating_sub(1));
        self.list_state.select(Some(previous));
    }

    fn open_selected(&mut self) {
        if let Some(selected) = self.list_state.selected() {
            self.detail = self.visible.get(selected).copied();
        }
    }

    fn open_ticket_by_id(&mut self, id: uuid::Uuid) {
        if let Some(visible_pos) = self
            .visible
            .iter()
            .position(|idx| self.tickets[*idx].id == id)
        {
            self.list_state.select(Some(visible_pos));
            self.detail = self.visible.get(visible_pos).copied();
        }
    }

    fn selected_ticket(&self) -> Option<&Ticket> {
        self.list_state
            .selected()
            .and_then(|selected| self.visible.get(selected))
            .map(|idx| &self.tickets[*idx])
    }
}

impl NewTicketDraft {
    fn current_value_mut(&mut self) -> &mut String {
        match self.field {
            NewTicketField::Title => &mut self.title,
            NewTicketField::Description => &mut self.description,
            NewTicketField::Tags => &mut self.tags,
            NewTicketField::Assigned => &mut self.assigned,
        }
    }

    fn next_field(&mut self) {
        self.field = match self.field {
            NewTicketField::Title => NewTicketField::Description,
            NewTicketField::Description => NewTicketField::Tags,
            NewTicketField::Tags => NewTicketField::Assigned,
            NewTicketField::Assigned => NewTicketField::Title,
        };
    }

    fn previous_field(&mut self) {
        self.field = match self.field {
            NewTicketField::Title => NewTicketField::Assigned,
            NewTicketField::Description => NewTicketField::Title,
            NewTicketField::Tags => NewTicketField::Description,
            NewTicketField::Assigned => NewTicketField::Tags,
        };
    }
}

impl InputKind {
    fn label(self) -> &'static str {
        match self {
            InputKind::Title => "title",
            InputKind::Description => "description",
            InputKind::Comment => "comment",
            InputKind::AddTags => "add tags",
            InputKind::RemoveTags => "remove tags",
        }
    }

    fn modal_height(self) -> u16 {
        match self {
            InputKind::Description | InputKind::Comment => 14,
            InputKind::Title | InputKind::AddTags | InputKind::RemoveTags => 9,
        }
    }
}

fn ticket_matches(ticket: &Ticket, needle: &str) -> bool {
    ticket.title.to_ascii_lowercase().contains(needle)
        || ticket
            .description
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains(needle)
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn optional_trimmed(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn created_by_display(ticket: &Ticket) -> &str {
    ticket
        .description
        .as_deref()
        .and_then(github_author)
        .unwrap_or(&ticket.created_by)
}

fn github_author(description: &str) -> Option<&str> {
    description.lines().find_map(|line| {
        line.strip_prefix("GitHub author:")
            .map(str::trim)
            .filter(|author| !author.is_empty())
    })
}

fn field_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<10}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" : ", Style::default().fg(Color::DarkGray)),
        Span::styled(value.to_string(), Style::default().fg(Color::Cyan)),
    ])
}

fn new_ticket_field_line(
    field: NewTicketField,
    active: NewTicketField,
    label: &str,
    value: &str,
    required: bool,
) -> Line<'static> {
    let marker = if field == active { ">" } else { " " };
    let label_style = if field == active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let value_style = if field == active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let suffix = if required { " *" } else { "" };
    Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        Span::styled(format!("{label:<12}{suffix}"), label_style),
        Span::styled(" : ", Style::default().fg(Color::DarkGray)),
        Span::styled(value.to_string(), value_style),
    ])
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical_margin = area.height.saturating_sub(height) / 2;
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(vertical_margin),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1]);
    horizontal[1]
}
