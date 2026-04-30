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
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use ticgit_lib::{query, Filter, Ticket, TicketState};

use crate::commands::open_store;
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {}

pub fn run(_args: Args) -> Result<()> {
    let store = open_store()?;
    let tickets = query::apply(
        store.list()?,
        &Filter {
            state: Some(TicketState::Open),
            ..Default::default()
        },
    );

    let mut terminal = init_terminal()?;
    let mut guard = TerminalGuard { active: true };
    let result = App::new(tickets).run(&mut terminal);
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
    tickets: Vec<Ticket>,
    visible: Vec<usize>,
    list_state: ListState,
    filter: String,
    editing_filter: bool,
    detail: Option<usize>,
}

impl App {
    fn new(tickets: Vec<Ticket>) -> Self {
        let mut app = Self {
            tickets,
            visible: Vec::new(),
            list_state: ListState::default(),
            filter: String::new(),
            editing_filter: false,
            detail: None,
        };
        app.apply_filter();
        app
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
            if self.handle_key(key) {
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
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(outer[1]);

        self.draw_filter(frame, outer[0]);
        self.draw_list(frame, panes[0]);
        self.draw_detail(frame, panes[1]);
    }

    fn draw_filter(&self, frame: &mut Frame<'_>, area: Rect) {
        let mode = if self.editing_filter {
            "filter"
        } else {
            "normal"
        };
        let help = if self.editing_filter {
            "type to filter, Enter/Esc to finish"
        } else {
            "/ filter  Enter details  ↑/↓ or j/k move  q quit"
        };
        let prompt = Line::from(vec![
            Span::styled(format!("{mode} "), Style::default().fg(Color::Yellow)),
            Span::raw("/"),
            Span::styled(self.filter.as_str(), Style::default().fg(Color::Cyan)),
            Span::raw(format!("  {help}")),
        ]);
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
        let text = match self.detail {
            Some(idx) => render::ticket_detail(&self.tickets[idx]),
            None if self.visible.is_empty() => {
                "No open tickets match the current filter.".to_string()
            }
            None => "Press Enter on a ticket to show its details here.".to_string(),
        };
        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.editing_filter {
            self.handle_filter_key(key);
            return false;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => true,
            KeyCode::Char('/') => {
                self.editing_filter = true;
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
            _ => false,
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.editing_filter = false;
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
