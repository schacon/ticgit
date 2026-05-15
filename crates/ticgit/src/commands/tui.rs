use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Stdout};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
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
use ratatui::widgets::{
    Block, Borders, Clear, Gauge, HighlightSpacing, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::{Frame, Terminal};
use ticgit_lib::{
    query, Comment, Filter, NewTicketOpts, NewWriteupOpts, SortKey, SortOrder, Ticket,
    TicketLifecycle, TicketState, TicketStatus, TicketStore, Writeup, WriteupStatus,
};
use time::OffsetDateTime;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const HIGHLIGHT_SYMBOL: &str = "> ";
const LIST_ID_WIDTH: usize = 3;
const LIST_STATE_WIDTH: usize = 2;
const LIST_AGE_WIDTH: usize = 3;
const LIST_PRIORITY_WIDTH: usize = 3;
const COMPACT_LIST_MIN_TITLE_WIDTH: usize = 24;
const ISSUE_TABLE_MIN_TITLE_WIDTH: usize = 30;
const ANSI_TAG_COLORS: [Color; 12] = [
    Color::Blue,
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::LightBlue,
    Color::LightCyan,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightMagenta,
    Color::LightRed,
    Color::Gray,
];
const BOARD_STATES: [TicketState; 5] = [
    TicketState::New,
    TicketState::Assigned,
    TicketState::InProgress,
    TicketState::Blocked,
    TicketState::Review,
];
const DETAIL_WIDTH_PERCENT_DEFAULT: u16 = 58;
const DETAIL_WIDTH_PERCENT_MIN: u16 = 35;
const DETAIL_WIDTH_PERCENT_MAX: u16 = 80;
const DETAIL_WIDTH_PERCENT_STEP: u16 = 5;

use crate::commands::{open_store, SessionGitDir};
use crate::editor;
use crate::session_state::{SavedView, State};
use crate::timefmt::relative_time;

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

fn suspend_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn resume_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.hide_cursor()?;
    terminal.clear()?;
    Ok(())
}

fn load_closed_times(
    git_dir: &std::path::Path,
    tickets: &[Ticket],
) -> HashMap<uuid::Uuid, OffsetDateTime> {
    let db_path = git_dir.join("git-meta.sqlite");
    let Ok(conn) =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return HashMap::new();
    };

    tickets
        .iter()
        .filter(|ticket| ticket.status == TicketStatus::Closed)
        .filter_map(|ticket| {
            query_closed_at(&conn, ticket.id).map(|closed_at| (ticket.id, closed_at))
        })
        .collect()
}

fn query_closed_at(conn: &rusqlite::Connection, id: uuid::Uuid) -> Option<OffsetDateTime> {
    let status_key = format!("ticgit:tickets:{id}:status");
    let closed_by_key = format!("ticgit:tickets:{id}:closed-by");
    let timestamp_ms: i64 = conn
        .query_row(
            "SELECT timestamp \
             FROM metadata_log \
             WHERE target_type = 'project' \
               AND operation != 'remove' \
               AND ((key = ?1 AND value IN ('\"closed\"', 'closed')) OR key = ?2) \
             ORDER BY timestamp DESC \
             LIMIT 1",
            rusqlite::params![status_key, closed_by_key],
            |row| row.get(0),
        )
        .ok()?;
    OffsetDateTime::from_unix_timestamp(timestamp_ms / 1000).ok()
}

struct App {
    store: TicketStore,
    all_tickets: Vec<Ticket>,
    tickets: Vec<Ticket>,
    visible: Vec<usize>,
    writeups: Vec<Writeup>,
    visible_writeups: Vec<usize>,
    list_state: ListState,
    writeup_state: ListState,
    board_column: usize,
    board_rows: [usize; BOARD_STATES.len()],
    view: ViewMode,
    active_tab: TuiTab,
    show_all_writeups: bool,
    active_view_name: Option<String>,
    saved_view_state: ListState,
    pending_delete_view: Option<String>,
    base_status: Option<TicketStatus>,
    base_state: Option<TicketState>,
    assigned_filter: Option<String>,
    only_tagged: bool,
    hide_subissues: bool,
    show_subissues_preference: bool,
    sort_order: Option<SortOrder>,
    sort_closed_desc: bool,
    closed_at: HashMap<uuid::Uuid, OffsetDateTime>,
    issue_columns: Vec<IssueColumn>,
    filter: String,
    tag_filter: BTreeSet<String>,
    tag_filter_match_all: bool,
    tag_picker_state: ListState,
    manage_tag_state: ListState,
    link_issue_state: ListState,
    writeup_toc_state: ListState,
    version_state: ListState,
    order_state: ListState,
    column_state: ListState,
    mode: Mode,
    input: String,
    new_ticket: NewTicketDraft,
    detail: Option<usize>,
    writeup_detail: Option<usize>,
    writeup_detail_focus: WriteupPaneFocus,
    writeup_detail_scroll: u16,
    writeup_toc_open: bool,
    detail_width_percent: u16,
    comments_mode: bool,
    comment_state: ListState,
    show_help: bool,
    show_quit_hint: bool,
    status: Option<String>,
    sync: Option<SyncState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    List,
    Board,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiTab {
    Issues,
    Writeups,
    Dashboard,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum WriteupPaneFocus {
    #[default]
    List,
    Detail,
    Toc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagTarget {
    Ticket(uuid::Uuid),
    Writeup(uuid::Uuid),
}

struct SyncState {
    receiver: Receiver<Result<SyncResult>>,
    selected_id: Option<uuid::Uuid>,
    started_at: Instant,
}

struct SyncResult {
    summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewKind {
    BuiltIn,
    Saved,
}

#[derive(Debug, Clone)]
struct ViewEntry {
    name: String,
    view: SavedView,
    kind: ViewKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownHeading {
    level: usize,
    title: String,
    line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WriteupBodyStats {
    words: usize,
    read_minutes: usize,
    headings: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Filter,
    Tags,
    ManageTags,
    Order,
    Columns,
    SavedViews,
    ConfirmDeleteView,
    SaveView,
    LinkIssueSearch,
    UnlinkIssueSelect,
    Versions,
    Input(InputKind),
    State,
    Create,
}

#[derive(Debug, Clone, Copy)]
struct MenuHint {
    key: &'static str,
    desc: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrderChoice {
    Priority,
    DateAsc,
    DateDesc,
    State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueColumn {
    Id,
    Date,
    Closed,
    Priority,
    State,
    Title,
    Assignee,
    Points,
    Milestone,
    Tags,
}

const ORDER_CHOICES: [OrderChoice; 4] = [
    OrderChoice::Priority,
    OrderChoice::DateAsc,
    OrderChoice::DateDesc,
    OrderChoice::State,
];
const ISSUE_COLUMN_CHOICES: [IssueColumn; 10] = [
    IssueColumn::Id,
    IssueColumn::Date,
    IssueColumn::Closed,
    IssueColumn::Priority,
    IssueColumn::State,
    IssueColumn::Title,
    IssueColumn::Assignee,
    IssueColumn::Points,
    IssueColumn::Milestone,
    IssueColumn::Tags,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Priority,
    Points,
    AddTags,
    RemoveTags,
}

#[derive(Debug, Default)]
struct NewTicketDraft {
    title: String,
    description: String,
    tags: String,
    assigned: String,
    parent: Option<uuid::Uuid>,
    field: NewTicketField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutlineRow {
    ticket_idx: usize,
    depth: usize,
    has_children: bool,
    collapsed: bool,
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
        let project_settings = State::load()
            .unwrap_or_default()
            .project_settings_for(&store.session().repo_git_dir());
        let detail_width_percent = project_settings
            .detail_width_percent
            .unwrap_or(DETAIL_WIDTH_PERCENT_DEFAULT)
            .clamp(DETAIL_WIDTH_PERCENT_MIN, DETAIL_WIDTH_PERCENT_MAX);
        let show_subissues_preference = project_settings.show_subissues.unwrap_or(false);
        let mut app = Self {
            store,
            all_tickets: Vec::new(),
            tickets: Vec::new(),
            visible: Vec::new(),
            writeups: Vec::new(),
            visible_writeups: Vec::new(),
            list_state: ListState::default(),
            writeup_state: ListState::default(),
            board_column: 0,
            board_rows: [0; BOARD_STATES.len()],
            view: ViewMode::List,
            active_tab: TuiTab::Issues,
            show_all_writeups: false,
            active_view_name: None,
            saved_view_state: ListState::default(),
            pending_delete_view: None,
            base_status: Some(TicketStatus::Open),
            base_state: None,
            assigned_filter: None,
            only_tagged: false,
            hide_subissues: !show_subissues_preference,
            show_subissues_preference,
            sort_order: None,
            sort_closed_desc: false,
            closed_at: HashMap::new(),
            issue_columns: default_issue_columns(),
            filter: String::new(),
            tag_filter: BTreeSet::new(),
            tag_filter_match_all: true,
            tag_picker_state: ListState::default(),
            manage_tag_state: ListState::default(),
            link_issue_state: ListState::default(),
            writeup_toc_state: ListState::default(),
            version_state: ListState::default(),
            order_state: ListState::default(),
            column_state: ListState::default(),
            mode: Mode::Normal,
            input: String::new(),
            new_ticket: NewTicketDraft::default(),
            detail: None,
            writeup_detail: None,
            writeup_detail_focus: WriteupPaneFocus::List,
            writeup_detail_scroll: 0,
            writeup_toc_open: false,
            detail_width_percent,
            comments_mode: false,
            comment_state: ListState::default(),
            show_help: false,
            show_quit_hint: false,
            status: None,
            sync: None,
        };
        app.reload_all(None, None)?;
        Ok(app)
    }

    fn reload(&mut self, preferred_id: Option<uuid::Uuid>) -> Result<()> {
        let tickets = self.store.list()?;
        self.closed_at = load_closed_times(&self.store.session().repo_git_dir(), &tickets);
        self.all_tickets = tickets.clone();
        self.tickets = query::apply(
            tickets,
            &Filter {
                status: self.base_status,
                state: self.base_state,
                assigned: self.assigned_filter.clone(),
                only_tagged: self.only_tagged,
                hide_subissues: self.hide_subissues,
                order: self.sort_order,
                ..Default::default()
            },
        );
        if self.sort_closed_desc {
            let closed_at = &self.closed_at;
            self.tickets.sort_by(|a, b| {
                closed_at_for(closed_at, b)
                    .cmp(&closed_at_for(closed_at, a))
                    .then_with(|| b.created_at.cmp(&a.created_at))
                    .then_with(|| a.id.cmp(&b.id))
            });
        } else if self.sort_order.is_none() {
            self.tickets.sort_by(compare_tui_tickets);
        }
        self.apply_filter();

        if let Some(id) = preferred_id {
            let list_indices = self.list_ticket_indices();
            if let Some(list_pos) = list_indices
                .iter()
                .position(|idx| self.tickets[*idx].id == id)
            {
                self.list_state.select(Some(list_pos));
                if self.detail.is_some() {
                    self.detail = list_indices.get(list_pos).copied();
                }
            } else if self.detail.is_some() {
                self.detail = None;
                self.comments_mode = false;
            }
        }
        self.sync_comment_selection();

        Ok(())
    }

    fn reload_all(
        &mut self,
        preferred_ticket_id: Option<uuid::Uuid>,
        preferred_writeup_id: Option<uuid::Uuid>,
    ) -> Result<()> {
        self.reload(preferred_ticket_id)?;
        self.reload_writeups(preferred_writeup_id)?;
        Ok(())
    }

    fn reload_writeups(&mut self, preferred_id: Option<uuid::Uuid>) -> Result<()> {
        self.writeups = self.store.list_writeups()?;
        self.writeups.sort_by(|a, b| {
            writeup_recent_at(b)
                .cmp(&writeup_recent_at(a))
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.id.cmp(&b.id))
        });
        self.apply_writeup_filter();

        if let Some(id) = preferred_id {
            if let Some(visible_pos) = self
                .visible_writeups
                .iter()
                .position(|idx| self.writeups[*idx].id == id)
            {
                self.writeup_state.select(Some(visible_pos));
                if self.writeup_detail.is_some() {
                    self.writeup_detail = self.visible_writeups.get(visible_pos).copied();
                }
            } else if self.writeup_detail.is_some() {
                self.writeup_detail = None;
            }
        }

        Ok(())
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        loop {
            self.poll_sync()?;
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
            if self.handle_key(key, terminal)? {
                return Ok(());
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let constraints = if self.sync.is_some() {
            vec![
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ]
        } else {
            vec![Constraint::Min(1), Constraint::Length(1)]
        };
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        if self.active_tab == TuiTab::Issues && self.detail.is_some() {
            let list_width = 100_u16.saturating_sub(self.detail_width_percent);
            let panes = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(list_width),
                    Constraint::Percentage(self.detail_width_percent),
                ])
                .split(outer[0]);
            if self.comments_mode {
                self.draw_comments_list(frame, panes[0]);
                self.draw_comment_detail(frame, panes[1]);
            } else {
                self.draw_issue_view(frame, panes[0]);
                self.draw_detail(frame, panes[1]);
            }
        } else if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() {
            let list_width = 100_u16.saturating_sub(self.detail_width_percent);
            let panes = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(list_width),
                    Constraint::Percentage(self.detail_width_percent),
                ])
                .split(outer[0]);
            self.draw_writeup_list(frame, panes[0]);
            self.draw_writeup_detail(frame, panes[1]);
        } else {
            match self.active_tab {
                TuiTab::Issues => match self.view {
                    ViewMode::List => self.draw_list(frame, outer[0]),
                    ViewMode::Board => self.draw_board(frame, outer[0]),
                },
                TuiTab::Writeups => self.draw_writeup_list(frame, outer[0]),
                TuiTab::Dashboard => self.draw_dashboard(frame, outer[0]),
            }
        }
        if self.sync.is_some() {
            self.draw_sync_progress(frame, outer[1]);
            self.draw_menu_bar(frame, outer[2]);
        } else {
            self.draw_menu_bar(frame, outer[1]);
        }

        match self.mode {
            Mode::Tags => self.draw_tags_modal(frame),
            Mode::ManageTags => self.draw_manage_tags_modal(frame),
            Mode::Order => self.draw_order_modal(frame),
            Mode::Columns => self.draw_columns_modal(frame),
            Mode::SavedViews => self.draw_saved_views_modal(frame),
            Mode::ConfirmDeleteView => self.draw_delete_view_confirm_modal(frame),
            Mode::SaveView => self.draw_save_view_modal(frame),
            Mode::LinkIssueSearch => self.draw_link_issue_search_modal(frame),
            Mode::UnlinkIssueSelect => self.draw_unlink_issue_select_modal(frame),
            Mode::Versions => self.draw_versions_modal(frame),
            Mode::Input(kind) => self.draw_input_modal(frame, kind),
            Mode::State => self.draw_state_modal(frame),
            Mode::Create => self.draw_create_modal(frame),
            _ => {}
        }
        if self.show_help {
            self.draw_help_modal(frame);
        }
        if self.show_quit_hint {
            self.draw_quit_hint_modal(frame);
        }
    }

    fn draw_menu_bar(&self, frame: &mut Frame<'_>, area: Rect) {
        let (mode, detail, hints) = self.menu_bar_content();
        let prompt = menu_bar_line(usize::from(area.width), mode, detail.as_deref(), &hints);
        let paragraph = Paragraph::new(prompt).style(Style::default().bg(Color::DarkGray));
        frame.render_widget(paragraph, area);
    }

    fn menu_bar_content(&self) -> (&'static str, Option<String>, Vec<MenuHint>) {
        match self.mode {
            Mode::Filter => (
                "filter",
                Some(format!("/{}", self.filter)),
                vec![
                    MenuHint {
                        key: "type",
                        desc: "filter",
                    },
                    MenuHint {
                        key: "Backspace",
                        desc: "delete",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "apply",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "finish",
                    },
                ],
            ),
            Mode::Tags => (
                "tags",
                None,
                vec![
                    MenuHint {
                        key: "j/k",
                        desc: "move",
                    },
                    MenuHint {
                        key: "Space",
                        desc: "toggle",
                    },
                    MenuHint {
                        key: "a",
                        desc: "all/any",
                    },
                    MenuHint {
                        key: "c",
                        desc: "clear",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "apply",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "finish",
                    },
                ],
            ),
            Mode::ManageTags => (
                "tags",
                None,
                vec![
                    MenuHint {
                        key: "j/k",
                        desc: "move",
                    },
                    MenuHint {
                        key: "Space",
                        desc: "toggle",
                    },
                    MenuHint {
                        key: "n",
                        desc: "new",
                    },
                    MenuHint {
                        key: "r",
                        desc: "remove",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "finish",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "finish",
                    },
                ],
            ),
            Mode::Order => (
                "order",
                None,
                vec![
                    MenuHint {
                        key: "j/k",
                        desc: "move",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "apply",
                    },
                    MenuHint {
                        key: "1-4",
                        desc: "choose",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "cancel",
                    },
                ],
            ),
            Mode::Columns => (
                "columns",
                Some(issue_columns_label(&self.issue_columns)),
                vec![
                    MenuHint {
                        key: "j/k",
                        desc: "move",
                    },
                    MenuHint {
                        key: "Space",
                        desc: "toggle",
                    },
                    MenuHint {
                        key: "d",
                        desc: "default",
                    },
                    MenuHint {
                        key: "V",
                        desc: "save view",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "finish",
                    },
                ],
            ),
            Mode::SavedViews => (
                "views",
                None,
                vec![
                    MenuHint {
                        key: "j/k",
                        desc: "move",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "apply",
                    },
                    MenuHint {
                        key: "d",
                        desc: "default",
                    },
                    MenuHint {
                        key: "D",
                        desc: "delete",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "cancel",
                    },
                ],
            ),
            Mode::ConfirmDeleteView => (
                "delete view",
                self.pending_delete_view.clone(),
                vec![
                    MenuHint {
                        key: "y",
                        desc: "delete",
                    },
                    MenuHint {
                        key: "n/Esc",
                        desc: "cancel",
                    },
                ],
            ),
            Mode::SaveView => (
                "save view",
                (!self.input.is_empty()).then(|| self.input.clone()),
                vec![
                    MenuHint {
                        key: "type",
                        desc: "name",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "save",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "cancel",
                    },
                    MenuHint {
                        key: "Backspace",
                        desc: "delete",
                    },
                ],
            ),
            Mode::LinkIssueSearch => (
                "link issue",
                (!self.input.is_empty()).then(|| self.input.clone()),
                vec![
                    MenuHint {
                        key: "type",
                        desc: "search",
                    },
                    MenuHint {
                        key: "j/k",
                        desc: "move",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "link",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "cancel",
                    },
                    MenuHint {
                        key: "Backspace",
                        desc: "delete",
                    },
                ],
            ),
            Mode::UnlinkIssueSelect => (
                "unlink issue",
                None,
                vec![
                    MenuHint {
                        key: "j/k",
                        desc: "move",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "unlink",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "cancel",
                    },
                ],
            ),
            Mode::Versions => (
                "versions",
                None,
                vec![
                    MenuHint {
                        key: "j/k",
                        desc: "move",
                    },
                    MenuHint {
                        key: "Enter/Esc",
                        desc: "close",
                    },
                ],
            ),
            Mode::Input(kind) => (
                "editing",
                Some(kind.label().to_string()),
                vec![
                    MenuHint {
                        key: "Enter",
                        desc: "apply",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "cancel",
                    },
                    MenuHint {
                        key: "Backspace",
                        desc: "delete",
                    },
                ],
            ),
            Mode::State => (
                "state",
                None,
                vec![
                    MenuHint {
                        key: "n/a/p/b/v",
                        desc: "open",
                    },
                    MenuHint {
                        key: "r/w/u/i",
                        desc: "closed",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "cancel",
                    },
                ],
            ),
            Mode::Create => (
                "new",
                None,
                vec![
                    MenuHint {
                        key: "Tab",
                        desc: "fields",
                    },
                    MenuHint {
                        key: "Enter",
                        desc: "create",
                    },
                    MenuHint {
                        key: "Esc",
                        desc: "cancel",
                    },
                    MenuHint {
                        key: "Backspace",
                        desc: "delete",
                    },
                ],
            ),
            Mode::Normal => {
                let detail = self.status.clone();
                let hints = if self.comments_mode {
                    vec![
                        MenuHint {
                            key: "j/k",
                            desc: "comments",
                        },
                        MenuHint {
                            key: "c",
                            desc: "comment",
                        },
                        MenuHint {
                            key: "+/-",
                            desc: "resize",
                        },
                        MenuHint {
                            key: "Esc",
                            desc: "details",
                        },
                        MenuHint {
                            key: "r",
                            desc: "refresh",
                        },
                        MenuHint {
                            key: "q",
                            desc: "quit",
                        },
                    ]
                } else if self.active_tab == TuiTab::Dashboard {
                    vec![
                        MenuHint {
                            key: "Tab",
                            desc: "issues",
                        },
                        MenuHint {
                            key: "d",
                            desc: "issues",
                        },
                        MenuHint {
                            key: "r",
                            desc: "refresh",
                        },
                        MenuHint {
                            key: "S",
                            desc: "sync",
                        },
                        MenuHint {
                            key: "q",
                            desc: "quit",
                        },
                    ]
                } else if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() {
                    let move_hint = if self.writeup_detail_focus == WriteupPaneFocus::Detail {
                        MenuHint {
                            key: "j/k",
                            desc: "scroll",
                        }
                    } else if self.writeup_detail_focus == WriteupPaneFocus::Toc {
                        MenuHint {
                            key: "j/k",
                            desc: "contents",
                        }
                    } else {
                        MenuHint {
                            key: "j/k",
                            desc: "writeups",
                        }
                    };
                    vec![
                        MenuHint {
                            key: "Tab",
                            desc: "issues",
                        },
                        move_hint,
                        MenuHint {
                            key: "h/l",
                            desc: "pane",
                        },
                        MenuHint {
                            key: "Enter",
                            desc: "jump/open",
                        },
                        MenuHint {
                            key: "i/u",
                            desc: "link",
                        },
                        MenuHint {
                            key: "t",
                            desc: "contents",
                        },
                        MenuHint {
                            key: "v",
                            desc: "versions",
                        },
                        MenuHint {
                            key: "e",
                            desc: "edit",
                        },
                        MenuHint {
                            key: "+/-",
                            desc: "resize",
                        },
                        MenuHint {
                            key: "Esc",
                            desc: "close",
                        },
                        MenuHint {
                            key: "q",
                            desc: "quit",
                        },
                    ]
                } else if self.active_tab == TuiTab::Writeups {
                    vec![
                        MenuHint {
                            key: "Tab",
                            desc: "issues",
                        },
                        MenuHint {
                            key: "j/k",
                            desc: "writeups",
                        },
                        MenuHint {
                            key: "Enter",
                            desc: "details",
                        },
                        MenuHint {
                            key: "e",
                            desc: "edit",
                        },
                        MenuHint {
                            key: "t",
                            desc: "tags",
                        },
                        MenuHint {
                            key: "v",
                            desc: "versions",
                        },
                        MenuHint {
                            key: "n",
                            desc: "new",
                        },
                        MenuHint {
                            key: "p",
                            desc: "priority",
                        },
                        MenuHint {
                            key: "P",
                            desc: "promote",
                        },
                        MenuHint {
                            key: "a",
                            desc: "all/open",
                        },
                        MenuHint {
                            key: "d",
                            desc: "stats",
                        },
                        MenuHint {
                            key: "c/o",
                            desc: "close/open",
                        },
                        MenuHint {
                            key: "i/u",
                            desc: "link",
                        },
                        MenuHint {
                            key: "1-9",
                            desc: "jump",
                        },
                        MenuHint {
                            key: "r",
                            desc: "refresh",
                        },
                        MenuHint {
                            key: "q",
                            desc: "quit",
                        },
                    ]
                } else if self.view == ViewMode::Board && self.detail.is_none() {
                    vec![
                        MenuHint {
                            key: "Tab",
                            desc: "writeups",
                        },
                        MenuHint {
                            key: "j/k",
                            desc: "tickets",
                        },
                        MenuHint {
                            key: "h/l",
                            desc: "columns",
                        },
                        MenuHint {
                            key: "Enter",
                            desc: "details",
                        },
                        MenuHint {
                            key: "b",
                            desc: "list",
                        },
                        MenuHint {
                            key: "d",
                            desc: "stats",
                        },
                        MenuHint {
                            key: "U",
                            desc: "subissues",
                        },
                        MenuHint {
                            key: "s",
                            desc: "state",
                        },
                        MenuHint {
                            key: "t",
                            desc: "tags",
                        },
                        MenuHint {
                            key: "e",
                            desc: "edit",
                        },
                        MenuHint {
                            key: "c",
                            desc: "comment",
                        },
                        MenuHint {
                            key: "o",
                            desc: "order",
                        },
                        MenuHint {
                            key: "r",
                            desc: "refresh",
                        },
                        MenuHint {
                            key: "q",
                            desc: "quit",
                        },
                    ]
                } else if self.detail.is_some() {
                    vec![
                        MenuHint {
                            key: "Tab",
                            desc: "writeups",
                        },
                        MenuHint {
                            key: "j/k",
                            desc: "tickets",
                        },
                        MenuHint {
                            key: "b",
                            desc: "board",
                        },
                        MenuHint {
                            key: "d",
                            desc: "stats",
                        },
                        MenuHint {
                            key: "U",
                            desc: "subissues",
                        },
                        MenuHint {
                            key: "n",
                            desc: "subissue",
                        },
                        MenuHint {
                            key: "P",
                            desc: "parent",
                        },
                        MenuHint {
                            key: "+/-",
                            desc: "resize",
                        },
                        MenuHint {
                            key: "t",
                            desc: "tags",
                        },
                        MenuHint {
                            key: "c",
                            desc: "comment",
                        },
                        MenuHint {
                            key: "m",
                            desc: "comments",
                        },
                        MenuHint {
                            key: "i",
                            desc: "spec",
                        },
                        MenuHint {
                            key: "e",
                            desc: "edit",
                        },
                        MenuHint {
                            key: "s",
                            desc: "state",
                        },
                        MenuHint {
                            key: "o",
                            desc: "order",
                        },
                        MenuHint {
                            key: "x",
                            desc: "columns",
                        },
                        MenuHint {
                            key: "1-9",
                            desc: "writeup",
                        },
                        MenuHint {
                            key: "Esc",
                            desc: "close",
                        },
                        MenuHint {
                            key: "r",
                            desc: "refresh",
                        },
                        MenuHint {
                            key: "q",
                            desc: "quit",
                        },
                    ]
                } else {
                    vec![
                        MenuHint {
                            key: "Tab",
                            desc: "writeups",
                        },
                        MenuHint {
                            key: "j/k",
                            desc: "tickets",
                        },
                        MenuHint {
                            key: "Enter",
                            desc: "details",
                        },
                        MenuHint {
                            key: "t",
                            desc: "tags",
                        },
                        MenuHint {
                            key: "/",
                            desc: "search",
                        },
                        MenuHint {
                            key: "g",
                            desc: "filter tags",
                        },
                        MenuHint {
                            key: "o",
                            desc: "order",
                        },
                        MenuHint {
                            key: "b",
                            desc: "board",
                        },
                        MenuHint {
                            key: "d",
                            desc: "stats",
                        },
                        MenuHint {
                            key: "U",
                            desc: "subissues",
                        },
                        MenuHint {
                            key: "n",
                            desc: "new",
                        },
                        MenuHint {
                            key: "v",
                            desc: "views",
                        },
                        MenuHint {
                            key: "S",
                            desc: "sync",
                        },
                        MenuHint {
                            key: "r",
                            desc: "refresh",
                        },
                        MenuHint {
                            key: "q",
                            desc: "quit",
                        },
                    ]
                };
                ("normal", detail, hints)
            }
        }
    }

    fn draw_sync_progress(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(sync) = &self.sync else {
            return;
        };
        let elapsed = sync.started_at.elapsed().as_millis() as u64;
        let ratio = ((elapsed / 80) % 100) as f64 / 100.0;
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
            .label("syncing tickets")
            .ratio(ratio);
        frame.render_widget(gauge, area);
    }

    fn draw_list(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let count = self.visible.len();
        let filter = self.active_filter_display();
        let scope = if self.base_status == Some(TicketStatus::Open) && self.base_state.is_none() {
            "Open tickets"
        } else {
            "Tickets"
        };
        let title = if filter.is_empty() {
            format!("{scope} ({count})")
        } else {
            format!("{scope} matching {filter} ({count})")
        };
        let title = tabs_title(self.active_tab, &title);

        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        let row_width =
            usize::from(inner.width).saturating_sub(UnicodeWidthStr::width(HIGHLIGHT_SYMBOL));
        let compact = self.detail.is_some();
        let columns = issue_columns_for_width(&self.issue_columns, row_width);
        let widths = issue_column_widths(&columns, row_width);
        let ticket_by_id = self
            .all_tickets
            .iter()
            .map(|ticket| (ticket.id, ticket))
            .collect::<HashMap<_, _>>();

        let items: Vec<ListItem<'_>> = self
            .list_ticket_indices()
            .iter()
            .map(|&idx| {
                let ticket = &self.tickets[idx];
                let title_prefix = issue_title_prefix(ticket, &ticket_by_id, !self.hide_subissues);
                ListItem::new(ticket_table_line(
                    ticket,
                    &columns,
                    &widths,
                    row_width,
                    &title_prefix,
                    compact,
                    self.store.email(),
                    self.closed_at.get(&ticket.id).copied(),
                    !self.linked_writeups(ticket.id).is_empty(),
                ))
            })
            .collect();

        frame.render_widget(block, area);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(inner);
        frame.render_widget(
            Paragraph::new(issue_table_header(&columns, &widths, row_width)),
            chunks[0],
        );
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_stateful_widget(list, chunks[1], &mut self.list_state);
    }

    fn draw_issue_view(&mut self, frame: &mut Frame<'_>, area: Rect) {
        match self.view {
            ViewMode::List | ViewMode::Board => self.draw_list(frame, area),
        }
    }

    fn draw_writeup_list(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let count = self.visible_writeups.len();
        let scope = if self.show_all_writeups {
            "All writeups"
        } else {
            "Open writeups"
        };
        let title = if self.filter.is_empty() {
            format!("{scope} by recency ({count})")
        } else {
            format!("{scope} matching \"{}\" ({count})", self.filter)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(tabs_title(self.active_tab, &title))
            .border_style(
                if self.writeup_detail.is_some()
                    && self.writeup_detail_focus == WriteupPaneFocus::List
                {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                },
            );
        let row_width = usize::from(block.inner(area).width)
            .saturating_sub(UnicodeWidthStr::width(HIGHLIGHT_SYMBOL));
        let compact = self.writeup_detail.is_some();

        let items: Vec<ListItem<'_>> = if self.visible_writeups.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                if self.show_all_writeups {
                    "No writeups yet."
                } else {
                    "No open writeups. Press a to show all."
                },
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            self.visible_writeups
                .iter()
                .map(|&idx| {
                    let writeup = &self.writeups[idx];
                    ListItem::new(writeup_list_line(writeup, row_width, compact))
                })
                .collect()
        };

        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_stateful_widget(list, area, &mut self.writeup_state);
    }

    fn draw_dashboard(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(tabs_title(self.active_tab, "Project stats"));
        let inner = block.inner(area);
        let width = usize::from(inner.width);
        let height = usize::from(inner.height);
        let stats = DashboardStats::from_tickets(&self.all_tickets);
        let lines = dashboard_lines(&stats, &self.closed_at, width, height);
        let dashboard = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false });
        frame.render_widget(dashboard, area);
    }

    fn draw_board(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let constraints = vec![Constraint::Percentage(20); BOARD_STATES.len()];
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(area);

        for (column_idx, state) in BOARD_STATES.iter().enumerate() {
            let tickets = self.board_column_tickets(column_idx);
            let title = format!("{} ({})", state.as_str(), tickets.len());
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(if column_idx == self.board_column {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                });
            let row_width = usize::from(block.inner(columns[column_idx]).width)
                .saturating_sub(UnicodeWidthStr::width(HIGHLIGHT_SYMBOL));
            let items: Vec<ListItem<'_>> = tickets
                .iter()
                .map(|&&idx| {
                    let ticket = &self.tickets[idx];
                    ListItem::new(board_ticket_line(ticket, row_width))
                })
                .collect();
            let mut state = ListState::default();
            if column_idx == self.board_column && !tickets.is_empty() {
                let selected = self.board_rows[column_idx].min(tickets.len() - 1);
                self.board_rows[column_idx] = selected;
                state.select(Some(selected));
            }
            let list = List::new(items)
                .block(block)
                .highlight_style(
                    Style::default()
                        .bg(Color::Rgb(0, 0, 95))
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(HIGHLIGHT_SYMBOL)
                .highlight_spacing(HighlightSpacing::Always);
            frame.render_stateful_widget(list, columns[column_idx], &mut state);
        }
    }

    fn draw_detail(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(idx) = self.detail else {
            return;
        };
        let ticket = &self.tickets[idx];
        let detail_width = usize::from(
            Block::default()
                .borders(Borders::ALL)
                .title("Details")
                .inner(area)
                .width,
        );
        let mut detail_lines = vec![
            Line::from(Span::styled(
                ticket.id.to_string(),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                ticket.title.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            field_line(
                "Created",
                &format!(
                    "{} ago by {}",
                    relative_time(ticket.created_at, OffsetDateTime::now_utc()),
                    created_by_display(ticket)
                ),
            ),
            status_state_line(ticket),
        ];
        if !ticket.tags.is_empty() {
            detail_lines.push(tags_field_line(&ticket.tags));
        }
        if let Some(assigned) = &ticket.assigned {
            detail_lines.push(field_line("Assigned", assigned));
        }
        if let Some(closed_by) = &ticket.closed_by {
            detail_lines.push(field_line("Closed by", closed_by));
        }
        if let Some(parent) = ticket.parent {
            detail_lines.push(field_line("Parent", &self.issue_label(parent)));
        }
        if !ticket.children.is_empty() {
            detail_lines.push(field_line(
                "Sub-issues",
                &format!("{} (press U to show in list)", ticket.children.len()),
            ));
            for child in ticket.children.iter().take(5) {
                detail_lines.push(detail_child_issue_line(&self.issue_label(*child)));
            }
        }
        if let Some(priority) = ticket.priority {
            detail_lines.push(field_line("Priority", &priority.to_string()));
        }
        if let Some(points) = ticket.points {
            detail_lines.push(field_line("Points", &points.to_string()));
        }
        if let Some(milestone) = &ticket.milestone {
            detail_lines.push(field_line("Milestone", milestone));
        }
        if let Some(spec) = &ticket.spec {
            detail_lines.push(spec_field_line(spec, detail_width));
        }
        let linked_writeups = self.linked_writeups(ticket.id);
        if !linked_writeups.is_empty() {
            detail_lines.push(Line::raw(""));
            detail_lines.push(Line::from(Span::styled(
                "Writeups",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            for (idx, writeup) in linked_writeups.iter().take(9).enumerate() {
                detail_lines.push(Line::from(vec![
                    Span::styled(
                        format!("{}", idx + 1),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(writeup.short_id(), Style::default().fg(Color::DarkGray)),
                    Span::raw(" "),
                    Span::raw(writeup.title.clone()),
                ]));
            }
        }
        if let Some(description) = &ticket.description {
            detail_lines.push(Line::raw(""));
            for line in description.lines() {
                detail_lines.push(Line::from(Span::raw(line.to_string())));
            }
        }
        if !ticket.comments.is_empty() {
            detail_lines.push(Line::raw(""));
            detail_lines.push(Line::from(Span::styled(
                "Comments",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            let width = usize::from(area.width).saturating_sub(2);
            for comment in &ticket.comments {
                detail_lines.push(comment_summary_line(comment, width));
            }
        }

        let detail_block = Block::default().borders(Borders::ALL).title("Details");
        let detail = Paragraph::new(detail_lines)
            .block(detail_block)
            .wrap(Wrap { trim: false });
        frame.render_widget(detail, area);
    }

    fn draw_writeup_detail(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Some(idx) = self.writeup_detail else {
            return;
        };
        let writeup = &self.writeups[idx];
        let toc_visible = self.writeup_toc_open;
        let (detail_area, toc_area) = if toc_visible {
            let toc_width = (area.width / 3).clamp(16, 36);
            let panes = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(12), Constraint::Length(toc_width)])
                .split(area);
            (panes[0], Some(panes[1]))
        } else {
            (area, None)
        };
        let (lines, headings) =
            writeup_detail_lines(writeup, &self.tickets, usize::from(detail_area.width));
        self.sync_writeup_toc_selection(&headings);

        let detail = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Writeup")
                    .border_style(if self.writeup_detail_focus == WriteupPaneFocus::Detail {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default()
                    }),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.writeup_detail_scroll, 0));
        frame.render_widget(detail, detail_area);

        if let Some(toc_area) = toc_area {
            self.draw_writeup_toc(frame, toc_area, &headings);
        }
    }

    fn draw_writeup_toc(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        headings: &[MarkdownHeading],
    ) {
        let items: Vec<ListItem<'_>> = if headings.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No headings",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            headings
                .iter()
                .map(|heading| {
                    let indent = " ".repeat(heading.level.saturating_sub(1).min(5));
                    ListItem::new(Line::from(vec![
                        Span::raw(indent),
                        Span::raw(truncate_display(
                            &heading.title,
                            usize::from(area.width).saturating_sub(4),
                        )),
                    ]))
                })
                .collect()
        };
        let toc = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Contents")
                    .border_style(if self.writeup_detail_focus == WriteupPaneFocus::Toc {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default()
                    }),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_stateful_widget(toc, area, &mut self.writeup_toc_state);
    }

    fn draw_comments_list(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Some(idx) = self.detail else {
            return;
        };
        let ticket = &self.tickets[idx];
        let title = truncate_display(&ticket.title, usize::from(area.width).saturating_sub(14));
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Comments: {title}"));
        let width = usize::from(block.inner(area).width)
            .saturating_sub(UnicodeWidthStr::width(HIGHLIGHT_SYMBOL));
        let items: Vec<ListItem<'_>> = if ticket.comments.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No comments. Press c to add one.",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            ticket
                .comments
                .iter()
                .map(|comment| ListItem::new(comment_summary_line(comment, width)))
                .collect()
        };

        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_stateful_widget(list, area, &mut self.comment_state);
    }

    fn draw_comment_detail(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(ticket) = self.detail.map(|idx| &self.tickets[idx]) else {
            return;
        };
        let lines = if let Some(comment) = self.selected_comment(ticket) {
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        relative_time(comment.at, OffsetDateTime::now_utc()),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        comment_author_display(&comment.author),
                        Style::default().fg(Color::Cyan),
                    ),
                ]),
                Line::raw(""),
            ];
            lines.extend(comment.body.lines().map(|line| Line::raw(line.to_string())));
            lines
        } else {
            vec![Line::from(Span::styled(
                "No comments yet. Press c to add one.",
                Style::default().fg(Color::DarkGray),
            ))]
        };
        let comments = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Comment"))
            .wrap(Wrap { trim: false });
        frame.render_widget(comments, area);
    }

    fn draw_input_modal(&self, frame: &mut Frame<'_>, kind: InputKind) {
        let area = centered_rect(70, kind.modal_height(), frame.area());
        let title = format!("Edit {}", kind.label());
        let help = match kind {
            InputKind::Priority => format!(
                "Enter priority. Lower is more important. Empty clears it. {}",
                self.priority_range_display()
            ),
            InputKind::Points => "Enter points estimate. Empty clears it.".to_string(),
            InputKind::AddTags => "Enter comma- or space-separated tags to add.".to_string(),
            InputKind::RemoveTags => "Enter comma- or space-separated tags to remove.".to_string(),
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

    fn draw_link_issue_search_modal(&mut self, frame: &mut Frame<'_>) {
        let area = centered_rect(74, 22, frame.area());
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
        let results = self.link_issue_search_results();
        let selected = self
            .link_issue_state
            .selected()
            .unwrap_or(0)
            .min(results.len().saturating_sub(1));
        if results.is_empty() {
            self.link_issue_state.select(None);
        } else {
            self.link_issue_state.select(Some(selected));
        }

        let search = Paragraph::new(Line::from(self.input.as_str())).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Search issues"),
        );
        let row_width = usize::from(chunks[1].width)
            .saturating_sub(2)
            .saturating_sub(UnicodeWidthStr::width(HIGHLIGHT_SYMBOL));
        let items: Vec<ListItem<'_>> = if results.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No matching unlinked issues.",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            results
                .iter()
                .map(|idx| {
                    ListItem::new(ticket_list_line(
                        &self.tickets[*idx],
                        row_width,
                        false,
                        self.store.email(),
                        false,
                    ))
                })
                .collect()
        };
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Link Issue"))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        let help = Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" link  "),
            Span::styled("j/k", Style::default().fg(Color::Yellow)),
            Span::raw(" move  "),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" cancel"),
        ]))
        .style(Style::default().bg(Color::DarkGray));

        frame.render_widget(Clear, area);
        frame.render_widget(search, chunks[0]);
        frame.render_stateful_widget(list, chunks[1], &mut self.link_issue_state);
        frame.render_widget(help, chunks[2]);
    }

    fn draw_unlink_issue_select_modal(&mut self, frame: &mut Frame<'_>) {
        let area = centered_rect(74, 20, frame.area());
        let linked = self.linked_issue_ids_for_selected_writeup();
        let selected = self
            .link_issue_state
            .selected()
            .unwrap_or(0)
            .min(linked.len().saturating_sub(1));
        if linked.is_empty() {
            self.link_issue_state.select(None);
        } else {
            self.link_issue_state.select(Some(selected));
        }

        let row_width = usize::from(area.width)
            .saturating_sub(2)
            .saturating_sub(UnicodeWidthStr::width(HIGHLIGHT_SYMBOL));
        let items: Vec<ListItem<'_>> = if linked.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No linked issues.",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            linked
                .iter()
                .map(|ticket_id| self.linked_issue_line(*ticket_id, row_width))
                .map(ListItem::new)
                .collect()
        };
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Unlink Issue"))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);

        frame.render_widget(Clear, area);
        frame.render_stateful_widget(list, area, &mut self.link_issue_state);
    }

    fn draw_versions_modal(&mut self, frame: &mut Frame<'_>) {
        let Some(writeup) = self.selected_writeup().cloned() else {
            return;
        };
        let title = writeup.title.clone();
        let versions = writeup.versions;
        let area = centered_rect(86, 28, frame.area());
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
            .split(area);
        let selected = self
            .version_state
            .selected()
            .unwrap_or_else(|| versions.len().saturating_sub(1))
            .min(versions.len().saturating_sub(1));
        if versions.is_empty() {
            self.version_state.select(None);
        } else {
            self.version_state.select(Some(selected));
        }

        let items: Vec<ListItem<'_>> = if versions.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No versions yet.",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            versions
                .iter()
                .enumerate()
                .map(|(idx, version)| {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("v{}", idx + 1),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            relative_time(version.at, OffsetDateTime::now_utc()),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]))
                })
                .collect()
        };
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Versions"))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);

        let mut preview_lines = Vec::new();
        if let Some(version) = self
            .version_state
            .selected()
            .and_then(|idx| versions.get(idx))
        {
            preview_lines.push(Line::from(Span::styled(
                title,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            preview_lines.push(Line::raw(""));
            preview_lines.push(Line::from(vec![
                Span::styled(
                    version
                        .at
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| relative_time(version.at, OffsetDateTime::now_utc())),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("  "),
                Span::styled(&version.author, Style::default().fg(Color::Cyan)),
            ]));
            preview_lines.push(Line::raw(""));
            preview_lines.extend(version.body.lines().map(|line| Line::raw(line.to_string())));
        } else {
            preview_lines.push(Line::from(Span::styled(
                "No version selected.",
                Style::default().fg(Color::DarkGray),
            )));
        }
        let preview = Paragraph::new(preview_lines)
            .block(Block::default().borders(Borders::ALL).title("Preview"))
            .wrap(Wrap { trim: false });

        frame.render_widget(Clear, area);
        frame.render_stateful_widget(list, panes[0], &mut self.version_state);
        frame.render_widget(preview, panes[1]);
    }

    fn draw_tags_modal(&mut self, frame: &mut Frame<'_>) {
        let area = centered_rect(64, 20, frame.area());
        let tags = self.available_tags();
        let selected = self
            .tag_picker_state
            .selected()
            .unwrap_or(0)
            .min(tags.len().saturating_sub(1));
        if tags.is_empty() {
            self.tag_picker_state.select(None);
        } else {
            self.tag_picker_state.select(Some(selected));
        }

        let items: Vec<ListItem<'_>> = if tags.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No tags on open tickets.",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            tags.iter()
                .map(|(tag, count)| {
                    let checked = if self.tag_filter.contains(tag) {
                        "[x]"
                    } else {
                        "[ ]"
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(checked, Style::default().fg(Color::Yellow)),
                        Span::raw(" "),
                        Span::styled(tag.clone(), Style::default().fg(tag_color(tag))),
                        Span::styled(format!(" ({count})"), Style::default().fg(Color::DarkGray)),
                    ]))
                })
                .collect()
        };
        let mode = if self.tag_filter_match_all {
            "all selected tags"
        } else {
            "any selected tag"
        };
        let title = format!("Tag Filter: {mode}");
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_widget(Clear, area);
        frame.render_stateful_widget(list, area, &mut self.tag_picker_state);
    }

    fn draw_manage_tags_modal(&mut self, frame: &mut Frame<'_>) {
        let area = centered_rect(64, 20, frame.area());
        let Some((_, target_label, target_tags)) = self.selected_tag_target() else {
            return;
        };
        let title = format!("Manage Tags: {target_label}");
        let tags = self.manageable_tags(&target_tags);
        let selected = self
            .manage_tag_state
            .selected()
            .unwrap_or(0)
            .min(tags.len().saturating_sub(1));
        if tags.is_empty() {
            self.manage_tag_state.select(None);
        } else {
            self.manage_tag_state.select(Some(selected));
        }

        let items: Vec<ListItem<'_>> = if tags.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No known tags. Press n to create one here.",
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            tags.iter()
                .map(|tag| {
                    let checked = if target_tags.contains(tag) {
                        "[x]"
                    } else {
                        "[ ]"
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(checked, Style::default().fg(Color::Yellow)),
                        Span::raw(" "),
                        Span::styled(tag.clone(), Style::default().fg(tag_color(tag))),
                    ]))
                })
                .collect()
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        let help = Paragraph::new(Line::from(vec![
            Span::styled("Space", Style::default().fg(Color::Yellow)),
            Span::raw(" add/remove  "),
            Span::styled("n", Style::default().fg(Color::Yellow)),
            Span::raw(" new tags  "),
            Span::styled("r", Style::default().fg(Color::Yellow)),
            Span::raw(" remove by name  "),
            Span::styled("Enter/Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" finish"),
        ]))
        .style(Style::default().bg(Color::DarkGray));
        frame.render_widget(Clear, area);
        frame.render_stateful_widget(list, chunks[0], &mut self.manage_tag_state);
        frame.render_widget(help, chunks[1]);
    }

    fn draw_order_modal(&mut self, frame: &mut Frame<'_>) {
        let area = centered_rect(52, 10, frame.area());
        let current = self.current_order_choice();
        let selected = self
            .order_state
            .selected()
            .unwrap_or_else(|| order_choice_index(current))
            .min(ORDER_CHOICES.len() - 1);
        self.order_state.select(Some(selected));

        let items: Vec<ListItem<'_>> = ORDER_CHOICES
            .iter()
            .enumerate()
            .map(|(idx, choice)| {
                let active = *choice == current;
                let marker = if active { "*" } else { " " };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Yellow)),
                    Span::raw(" "),
                    Span::styled(
                        format!("{}", idx + 1),
                        Style::default()
                            .fg(Color::LightYellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(choice.label(), Style::default().fg(Color::Cyan)),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("List Order"))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_widget(Clear, area);
        frame.render_stateful_widget(list, area, &mut self.order_state);
    }

    fn draw_columns_modal(&mut self, frame: &mut Frame<'_>) {
        let area = centered_rect(64, 17, frame.area());
        let selected = self
            .column_state
            .selected()
            .unwrap_or(0)
            .min(ISSUE_COLUMN_CHOICES.len().saturating_sub(1));
        self.column_state.select(Some(selected));

        let items = ISSUE_COLUMN_CHOICES
            .iter()
            .map(|column| {
                let checked = if self.issue_columns.contains(column) {
                    "[x]"
                } else {
                    "[ ]"
                };
                let locked = if *column == IssueColumn::Title {
                    " required"
                } else {
                    ""
                };
                ListItem::new(Line::from(vec![
                    Span::styled(checked, Style::default().fg(Color::Yellow)),
                    Span::raw(" "),
                    Span::styled(
                        fit_display(column.label(), 10),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(column.description(), Style::default().fg(Color::Gray)),
                    Span::styled(locked, Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect::<Vec<_>>();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        let title = format!("Columns: {}", issue_columns_label(&self.issue_columns));
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        let help = Paragraph::new(Line::from(vec![
            Span::styled("Space", Style::default().fg(Color::Yellow)),
            Span::raw(" show/hide  "),
            Span::styled("d", Style::default().fg(Color::Yellow)),
            Span::raw(" default  "),
            Span::styled("V", Style::default().fg(Color::Yellow)),
            Span::raw(" save view  "),
            Span::styled("Enter/Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" finish"),
        ]))
        .style(Style::default().bg(Color::DarkGray));
        frame.render_widget(Clear, area);
        frame.render_stateful_widget(list, chunks[0], &mut self.column_state);
        frame.render_widget(help, chunks[1]);
    }

    fn draw_saved_views_modal(&mut self, frame: &mut Frame<'_>) {
        let area = centered_rect(78, 20, frame.area());
        let views = self.view_entries();
        let selected = self
            .saved_view_state
            .selected()
            .unwrap_or(0)
            .min(views.len().saturating_sub(1));
        if views.is_empty() {
            self.saved_view_state.select(None);
        } else {
            self.saved_view_state.select(Some(selected));
        }

        let width = usize::from(area.width).saturating_sub(6);
        let items: Vec<ListItem<'_>> = if views.is_empty() {
            Vec::new()
        } else {
            views
                .iter()
                .map(|entry| {
                    let active = self.active_view_name.as_deref() == Some(entry.name.as_str());
                    let marker = if active { "*" } else { " " };
                    let desc = crate::commands::view::describe_view(&entry.view);
                    let kind = match entry.kind {
                        ViewKind::BuiltIn => "built-in",
                        ViewKind::Saved => "saved",
                    };
                    let name_width = 18;
                    let kind_width = 8;
                    let desc_width = width.saturating_sub(name_width + kind_width + 5);
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(Color::Yellow)),
                        Span::raw(" "),
                        Span::styled(
                            fit_display(&entry.name, name_width),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            fit_display(kind, kind_width),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            truncate_display(&desc, desc_width),
                            Style::default().fg(Color::Cyan),
                        ),
                    ]))
                })
                .collect()
        };
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Views"))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(0, 0, 95))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL)
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_widget(Clear, area);
        frame.render_stateful_widget(list, area, &mut self.saved_view_state);
    }

    fn draw_delete_view_confirm_modal(&self, frame: &mut Frame<'_>) {
        let area = centered_rect(56, 7, frame.area());
        let name = self
            .pending_delete_view
            .as_deref()
            .unwrap_or("<unknown view>");
        let name = truncate_display(name, usize::from(area.width).saturating_sub(24));
        let lines = vec![
            Line::from(Span::styled(
                format!("Delete saved view `{name}`?"),
                Style::default().fg(Color::LightRed),
            )),
            Line::raw(""),
            Line::from(Span::styled(
                "y delete   n/Esc cancel",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let modal = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Delete View"))
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, area);
        frame.render_widget(modal, area);
    }

    fn draw_save_view_modal(&self, frame: &mut Frame<'_>) {
        let area = centered_rect(64, 9, frame.area());
        let filter = self.active_filter_display();
        let lines = vec![
            Line::from(Span::styled(
                if filter.is_empty() {
                    "Saving current open-ticket view with no extra filters.".to_string()
                } else {
                    format!("Saving current view: {filter}")
                },
                Style::default().fg(Color::DarkGray),
            )),
            Line::raw(""),
            Line::from(self.input.as_str()),
            Line::raw(""),
            Line::from(Span::styled(
                "Enter save  Esc cancel",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let modal = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Save View"))
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, area);
        frame.render_widget(modal, area);
    }

    fn draw_quit_hint_modal(&self, frame: &mut Frame<'_>) {
        let area = centered_rect(42, 7, frame.area());
        let lines = vec![
            Line::from(Span::styled(
                "Esc backs out of views and filters.",
                Style::default().fg(Color::Cyan),
            )),
            Line::raw(""),
            Line::from(vec![
                Span::raw("Hit "),
                Span::styled(
                    "q",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" to quit."),
            ]),
        ];
        let modal = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Quit"))
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, area);
        frame.render_widget(modal, area);
    }

    fn draw_state_modal(&self, frame: &mut Frame<'_>) {
        let area = centered_rect(48, 15, frame.area());
        let lines = vec![
            Line::from(vec![
                Span::styled("n", Style::default().fg(Color::Yellow)),
                Span::raw(" open:new"),
            ]),
            Line::from(vec![
                Span::styled("a", Style::default().fg(Color::Yellow)),
                Span::raw(" open:assigned"),
            ]),
            Line::from(vec![
                Span::styled("p", Style::default().fg(Color::Yellow)),
                Span::raw(" open:in-progress"),
            ]),
            Line::from(vec![
                Span::styled("b", Style::default().fg(Color::Yellow)),
                Span::raw(" open:blocked"),
            ]),
            Line::from(vec![
                Span::styled("v", Style::default().fg(Color::Yellow)),
                Span::raw(" open:review"),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::styled("r", Style::default().fg(Color::Yellow)),
                Span::raw(" closed:resolved"),
            ]),
            Line::from(vec![
                Span::styled("w", Style::default().fg(Color::Yellow)),
                Span::raw(" closed:wontfix"),
            ]),
            Line::from(vec![
                Span::styled("u", Style::default().fg(Color::Yellow)),
                Span::raw(" closed:duplicate"),
            ]),
            Line::from(vec![
                Span::styled("i", Style::default().fg(Color::Yellow)),
                Span::raw(" closed:invalid"),
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
        let height = if self.new_ticket.parent.is_some() {
            17
        } else {
            15
        };
        let area = centered_rect(72, height, frame.area());
        let mut lines = Vec::new();
        if let Some(parent_id) = self.new_ticket.parent {
            lines.push(field_line("Parent", &self.issue_label(parent_id)));
            lines.push(Line::raw(""));
        }
        lines.extend([
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
        ]);
        let title = if self.new_ticket.parent.is_some() {
            "New sub-issue"
        } else {
            "New ticket"
        };
        let modal = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, area);
        frame.render_widget(modal, area);
    }

    fn draw_help_modal(&self, frame: &mut Frame<'_>) {
        let area = centered_rect(88, 22, frame.area());
        let mut lines = Vec::new();

        help_section(&mut lines, "General");
        lines.push(help_columns(
            ("?", "toggle help"),
            Some(("Esc", "back / cancel")),
        ));
        lines.push(help_columns(("q", "quit"), Some(("S", "sync tickets"))));

        match self.mode {
            Mode::Filter => {
                help_section(&mut lines, "Search Filter");
                lines.push(help_columns(
                    ("type", "search text"),
                    Some(("Backspace", "delete char")),
                ));
                lines.push(help_columns(("Enter", "apply"), Some(("Esc", "finish"))));
            }
            Mode::Tags => {
                help_section(&mut lines, "Tag Filter");
                lines.push(help_columns(
                    ("j/k", "move tag"),
                    Some(("Space", "check tag")),
                ));
                lines.push(help_columns(
                    ("a", "all / any mode"),
                    Some(("c", "clear tags")),
                ));
                lines.push(help_columns(("Enter", "apply"), Some(("Esc", "finish"))));
            }
            Mode::ManageTags => {
                help_section(&mut lines, "Tags");
                lines.push(help_columns(
                    ("j/k", "move tag"),
                    Some(("Space", "add / remove")),
                ));
                lines.push(help_columns(
                    ("n", "new tags"),
                    Some(("r", "remove by name")),
                ));
                lines.push(help_columns(("Enter", "finish"), None));
                lines.push(help_columns(("Esc", "finish"), None));
            }
            Mode::Order => {
                help_section(&mut lines, "List Order");
                lines.push(help_columns(
                    ("j/k", "move order"),
                    Some(("Enter", "apply")),
                ));
                lines.push(help_columns(("1", "prio"), Some(("2", "date asc"))));
                lines.push(help_columns(("3", "date desc"), Some(("4", "state"))));
                lines.push(help_columns(("Esc", "cancel"), None));
            }
            Mode::Columns => {
                help_section(&mut lines, "Columns");
                lines.push(help_columns(
                    ("j/k", "move column"),
                    Some(("Space", "show / hide")),
                ));
                lines.push(help_columns(("d", "default"), Some(("V", "save view"))));
                lines.push(help_columns(("Enter", "finish"), Some(("Esc", "finish"))));
            }
            Mode::SavedViews => {
                help_section(&mut lines, "Views");
                lines.push(help_columns(
                    ("j/k", "move view"),
                    Some(("Enter", "apply view")),
                ));
                lines.push(help_columns(
                    ("d", "default view"),
                    Some(("D", "delete saved view")),
                ));
                lines.push(help_columns(("Esc", "cancel"), None));
            }
            Mode::ConfirmDeleteView => {
                help_section(&mut lines, "Delete View");
                lines.push(help_columns(("y", "delete"), Some(("n/Esc", "cancel"))));
            }
            Mode::SaveView => {
                help_section(&mut lines, "Save View");
                lines.push(help_columns(
                    ("type", "view name"),
                    Some(("Backspace", "delete char")),
                ));
                lines.push(help_columns(("Enter", "save"), Some(("Esc", "cancel"))));
            }
            Mode::LinkIssueSearch => {
                help_section(&mut lines, "Link Issue");
                lines.push(help_columns(
                    ("type", "search title/description"),
                    Some(("Backspace", "delete char")),
                ));
                lines.push(help_columns(
                    ("j/k", "move issue"),
                    Some(("Enter", "link selected")),
                ));
                lines.push(help_columns(("Esc", "cancel"), None));
            }
            Mode::UnlinkIssueSelect => {
                help_section(&mut lines, "Unlink Issue");
                lines.push(help_columns(
                    ("j/k", "move issue"),
                    Some(("Enter", "unlink selected")),
                ));
                lines.push(help_columns(("Esc", "cancel"), None));
            }
            Mode::Versions => {
                help_section(&mut lines, "Versions");
                lines.push(help_columns(
                    ("j/k", "move version"),
                    Some(("Enter/Esc", "close")),
                ));
            }
            Mode::Input(kind) => {
                help_section(&mut lines, &format!("Editing {}", kind.label()));
                lines.push(help_columns(
                    ("type", "new value"),
                    Some(("Backspace", "delete char")),
                ));
                lines.push(help_columns(("Enter", "apply"), Some(("Esc", "cancel"))));
            }
            Mode::State => {
                help_section(&mut lines, "Open States");
                lines.push(help_columns(("n", "new"), Some(("a", "assigned"))));
                lines.push(help_columns(("p", "in progress"), Some(("b", "blocked"))));
                lines.push(help_columns(("v", "review"), None));

                help_section(&mut lines, "Closed States");
                lines.push(help_columns(("r", "resolved"), Some(("w", "wontfix"))));
                lines.push(help_columns(("u", "duplicate"), Some(("i", "invalid"))));
            }
            Mode::Create => {
                help_section(&mut lines, "New Ticket");
                lines.push(help_columns(
                    ("Tab/Down", "next field"),
                    Some(("Shift-Tab/Up", "previous field")),
                ));
                lines.push(help_columns(("Enter", "create"), Some(("Esc", "cancel"))));
            }
            Mode::Normal if self.comments_mode => {
                help_section(&mut lines, "Comments");
                lines.push(help_columns(
                    ("j/k", "move comment"),
                    Some(("c", "add comment")),
                ));
                lines.push(help_columns(
                    ("+/-", "resize detail"),
                    Some(("Esc", "detail view")),
                ));
                lines.push(help_columns(("r", "refresh"), None));
            }
            Mode::Normal
                if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() =>
            {
                help_section(&mut lines, "Writeup Detail");
                lines.push(help_columns(
                    ("h/l", "switch pane"),
                    Some(("Esc", "close detail")),
                ));
                lines.push(help_columns(
                    ("j/k", "move or scroll"),
                    Some(("Up/Down", "move or scroll")),
                ));
                lines.push(help_columns(
                    ("t", "contents"),
                    Some(("Enter", "jump heading")),
                ));
                lines.push(help_columns(
                    ("i", "link issue"),
                    Some(("u", "unlink issue")),
                ));
                lines.push(help_columns(("p", "priority"), Some(("P", "promote"))));
                lines.push(help_columns(("e", "edit latest"), Some(("v", "versions"))));
                lines.push(help_columns(
                    ("+/-", "resize detail"),
                    Some(("1-9", "jump issue")),
                ));
            }
            Mode::Normal if self.active_tab == TuiTab::Writeups => {
                help_section(&mut lines, "Writeups");
                lines.push(help_columns(
                    ("Tab", "issues tab"),
                    Some(("j/k", "move writeups")),
                ));
                lines.push(help_columns(
                    ("Enter", "details"),
                    Some(("e", "edit latest")),
                ));
                lines.push(help_columns(
                    ("n", "new writeup"),
                    Some(("a", "show all/open")),
                ));
                lines.push(help_columns(("c", "close"), Some(("o", "reopen"))));
                lines.push(help_columns(("p", "priority"), Some(("P", "promote"))));
                lines.push(help_columns(
                    ("i", "link issue"),
                    Some(("u", "unlink issue")),
                ));
                lines.push(help_columns(("v", "versions"), Some(("t", "manage tags"))));
                lines.push(help_columns(("1-9", "jump issue"), None));
                lines.push(help_columns(
                    ("+/-", "resize detail"),
                    Some(("r", "refresh")),
                ));

                help_section(&mut lines, "Views");
                lines.push(help_columns(("d", "stats view"), None));
            }
            Mode::Normal if self.active_tab == TuiTab::Dashboard => {
                help_section(&mut lines, "Dashboard");
                lines.push(help_columns(("Tab", "issues tab"), Some(("r", "refresh"))));
                lines.push(help_columns(("S", "sync tickets"), Some(("q", "quit"))));

                help_section(&mut lines, "Views");
                lines.push(help_columns(("d", "issues view"), None));
            }
            Mode::Normal if self.view == ViewMode::Board && self.detail.is_none() => {
                help_section(&mut lines, "Navigation");
                lines.push(help_columns(
                    ("h/l", "move columns"),
                    Some(("j/k", "move tickets")),
                ));
                lines.push(help_columns(
                    ("Left/Right", "move columns"),
                    Some(("Up/Down", "move tickets")),
                ));
                lines.push(help_columns(("Enter", "details"), Some(("r", "refresh"))));

                help_section(&mut lines, "Edit Ticket");
                lines.push(help_columns(("C", "claim"), Some(("s", "state"))));
                lines.push(help_columns(
                    ("e", "edit title/body"),
                    Some(("i", "edit spec")),
                ));
                lines.push(help_columns(("c", "comment"), Some(("t", "manage tags"))));
                lines.push(help_columns(("p", "priority"), Some(("o", "order"))));

                help_section(&mut lines, "Views");
                lines.push(help_columns(("b", "list view"), Some(("d", "stats view"))));
                lines.push(help_columns(("U", "subissues"), None));
            }
            Mode::Normal => {
                help_section(&mut lines, "Navigation");
                lines.push(help_columns(
                    ("Tab", "writeups tab"),
                    Some(("j/k", "move tickets")),
                ));
                lines.push(help_columns(
                    ("Up/Down", "move tickets"),
                    Some(("1-9", "jump writeup")),
                ));
                lines.push(help_columns(
                    ("Enter", "details"),
                    Some(("n", "new/subissue")),
                ));
                lines.push(help_columns(("P", "jump parent"), Some(("m", "comments"))));
                lines.push(help_columns(("+/-", "resize detail"), None));
                lines.push(help_columns(("r", "refresh"), None));

                help_section(&mut lines, "Filters");
                lines.push(help_columns(
                    ("/", "search text"),
                    Some(("g", "tag picker")),
                ));
                lines.push(help_columns(("o", "order"), Some(("v", "saved views"))));
                lines.push(help_columns(("x", "columns"), Some(("V", "save view"))));

                help_section(&mut lines, "Edit Ticket");
                lines.push(help_columns(("C", "claim"), Some(("s", "state"))));
                lines.push(help_columns(
                    ("e", "edit title/body"),
                    Some(("i", "edit spec")),
                ));
                lines.push(help_columns(("c", "comment"), Some(("t", "manage tags"))));
                lines.push(help_columns(("p", "priority"), None));

                help_section(&mut lines, "Views");
                lines.push(help_columns(("b", "board view"), Some(("d", "stats view"))));
                lines.push(help_columns(("U", "subissues"), None));
            }
        }

        lines.push(Line::raw(""));
        lines.push(help_note("Close help with Esc, ?, Enter, or q."));

        let modal = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Help"))
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, area);
        frame.render_widget(modal, area);
    }

    fn handle_key(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<bool> {
        if self.show_quit_hint {
            return match key.code {
                KeyCode::Char('q') => Ok(true),
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => {
                    self.show_quit_hint = false;
                    Ok(false)
                }
                _ => Ok(false),
            };
        }
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::Char('q') => {
                    self.show_help = false;
                }
                _ => {}
            }
            return Ok(false);
        }
        if key.code == KeyCode::Char('?') {
            self.show_help = true;
            return Ok(false);
        }

        let quit = match self.mode {
            Mode::Filter => {
                self.handle_filter_key(key);
                false
            }
            Mode::Tags => {
                self.handle_tags_key(key);
                false
            }
            Mode::ManageTags => {
                self.handle_manage_tags_key(key)?;
                false
            }
            Mode::Order => {
                self.handle_order_key(key)?;
                false
            }
            Mode::Columns => {
                self.handle_columns_key(key)?;
                false
            }
            Mode::SavedViews => {
                self.handle_saved_views_key(key)?;
                false
            }
            Mode::ConfirmDeleteView => {
                self.handle_delete_view_confirm_key(key)?;
                false
            }
            Mode::SaveView => {
                self.handle_save_view_key(key)?;
                false
            }
            Mode::LinkIssueSearch => {
                self.handle_link_issue_search_key(key)?;
                false
            }
            Mode::UnlinkIssueSelect => {
                self.handle_unlink_issue_select_key(key)?;
                false
            }
            Mode::Versions => {
                self.handle_versions_key(key);
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
            Mode::Normal => self.handle_normal_key(key, terminal)?,
        };
        Ok(quit)
    }

    fn handle_normal_key(
        &mut self,
        key: KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<bool> {
        self.status = None;
        let quit = match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Tab | KeyCode::BackTab => {
                self.toggle_tab();
                false
            }
            KeyCode::Esc => {
                if self.comments_mode {
                    self.comments_mode = false;
                    false
                } else if self.active_tab == TuiTab::Writeups
                    && self.writeup_detail_focus == WriteupPaneFocus::Toc
                {
                    self.writeup_toc_open = false;
                    self.writeup_detail_focus = WriteupPaneFocus::Detail;
                    false
                } else if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() {
                    self.writeup_detail = None;
                    self.writeup_detail_focus = WriteupPaneFocus::List;
                    self.writeup_detail_scroll = 0;
                    self.writeup_toc_open = false;
                    false
                } else if self.detail.is_some() {
                    self.detail = None;
                    self.comments_mode = false;
                    false
                } else if self.view == ViewMode::Board {
                    self.view = ViewMode::List;
                    false
                } else if self.has_active_view_filters() {
                    self.clear_view_filters()?;
                    false
                } else {
                    self.show_quit_hint = true;
                    false
                }
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                false
            }
            KeyCode::Char('g') => {
                if self.active_tab == TuiTab::Issues {
                    self.begin_tag_filter();
                }
                false
            }
            KeyCode::Char('t') => {
                if self.active_tab == TuiTab::Writeups
                    && self.writeup_detail.is_some()
                    && self.writeup_detail_focus != WriteupPaneFocus::List
                {
                    self.toggle_writeup_toc();
                } else {
                    self.begin_manage_tags();
                }
                false
            }
            KeyCode::Char('v') => {
                if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() {
                    self.begin_versions();
                } else if self.active_tab == TuiTab::Issues {
                    self.begin_saved_views();
                }
                false
            }
            KeyCode::Char('V') => {
                if self.active_tab == TuiTab::Issues {
                    self.begin_save_view();
                }
                false
            }
            KeyCode::Char('x') => {
                if self.active_tab == TuiTab::Issues && self.view == ViewMode::List {
                    self.begin_columns();
                }
                false
            }
            KeyCode::Char('b') => {
                if self.active_tab == TuiTab::Issues {
                    self.handle_board_key()?;
                }
                false
            }
            KeyCode::Char('d') => {
                self.handle_dashboard_key();
                false
            }
            KeyCode::Char('u') => {
                if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() {
                    self.begin_unlink_issue_select();
                }
                false
            }
            KeyCode::Char('U') => {
                if self.active_tab == TuiTab::Issues {
                    self.toggle_subissue_visibility()?;
                }
                false
            }
            KeyCode::Char('r') => {
                self.refresh_data()?;
                false
            }
            KeyCode::Char('n') => {
                if self.active_tab == TuiTab::Issues {
                    if let Some(parent_id) = self.detail.map(|idx| self.tickets[idx].id) {
                        self.begin_create_subissue(parent_id);
                    } else {
                        self.begin_create();
                    }
                } else {
                    self.create_writeup_in_editor(terminal)?;
                }
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.comments_mode {
                    self.next_comment();
                } else if self.active_tab == TuiTab::Writeups
                    && self.writeup_detail_focus == WriteupPaneFocus::Detail
                {
                    self.scroll_writeup_detail(1);
                } else if self.active_tab == TuiTab::Writeups
                    && self.writeup_detail_focus == WriteupPaneFocus::Toc
                {
                    self.next_writeup_heading();
                } else if self.active_tab == TuiTab::Issues
                    && self.view == ViewMode::Board
                    && self.detail.is_none()
                {
                    self.next_board_ticket();
                } else {
                    self.next();
                }
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.comments_mode {
                    self.previous_comment();
                } else if self.active_tab == TuiTab::Writeups
                    && self.writeup_detail_focus == WriteupPaneFocus::Detail
                {
                    self.scroll_writeup_detail(-1);
                } else if self.active_tab == TuiTab::Writeups
                    && self.writeup_detail_focus == WriteupPaneFocus::Toc
                {
                    self.previous_writeup_heading();
                } else if self.active_tab == TuiTab::Issues
                    && self.view == ViewMode::Board
                    && self.detail.is_none()
                {
                    self.previous_board_ticket();
                } else {
                    self.previous();
                }
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() {
                    self.focus_next_writeup_pane();
                } else if self.active_tab == TuiTab::Issues
                    && self.view == ViewMode::Board
                    && self.detail.is_none()
                {
                    self.next_board_column();
                }
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() {
                    self.focus_previous_writeup_pane();
                } else if self.active_tab == TuiTab::Issues
                    && self.view == ViewMode::Board
                    && self.detail.is_none()
                {
                    self.previous_board_column();
                }
                false
            }
            KeyCode::Enter => {
                if self.active_tab == TuiTab::Writeups
                    && self.writeup_detail_focus == WriteupPaneFocus::Toc
                {
                    self.jump_to_selected_writeup_heading();
                } else {
                    self.open_selected();
                }
                false
            }
            KeyCode::Char('e') => {
                if self.active_tab == TuiTab::Writeups {
                    self.edit_writeup_in_editor(terminal)?;
                } else {
                    self.edit_ticket_in_editor(terminal)?;
                }
                false
            }
            KeyCode::Char('i') => {
                if self.active_tab == TuiTab::Writeups && self.writeup_detail.is_some() {
                    self.begin_link_issue_search();
                } else if self.active_tab == TuiTab::Issues {
                    self.edit_spec_in_editor(terminal)?;
                }
                false
            }
            KeyCode::Char('c') => {
                if self.active_tab == TuiTab::Issues {
                    self.add_comment_in_editor(terminal)?;
                } else {
                    self.set_selected_writeup_status(WriteupStatus::Closed)?;
                }
                false
            }
            KeyCode::Char('C') => {
                if self.active_tab == TuiTab::Issues {
                    self.claim_selected()?;
                }
                false
            }
            KeyCode::Char('m') => {
                if self.active_tab == TuiTab::Issues {
                    self.enter_comments_mode();
                }
                false
            }
            KeyCode::Char('p') => {
                if self.active_tab == TuiTab::Writeups {
                    self.begin_input(InputKind::Priority);
                } else {
                    self.begin_input(InputKind::Priority);
                }
                false
            }
            KeyCode::Char('P') => {
                if self.active_tab == TuiTab::Writeups {
                    self.promote_selected_writeup()?;
                } else if self.active_tab == TuiTab::Issues {
                    self.jump_to_parent_issue();
                }
                false
            }
            KeyCode::Char('o') => {
                if self.active_tab == TuiTab::Issues {
                    self.begin_order();
                } else {
                    self.set_selected_writeup_status(WriteupStatus::Open)?;
                }
                false
            }
            KeyCode::Char('a') => {
                if self.active_tab == TuiTab::Writeups {
                    self.toggle_writeup_scope();
                }
                false
            }
            KeyCode::Char('O') => {
                if self.active_tab == TuiTab::Issues {
                    self.begin_input(InputKind::Points);
                }
                false
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.resize_detail(DETAIL_WIDTH_PERCENT_STEP as i16);
                false
            }
            KeyCode::Char('-') => {
                self.resize_detail(-(DETAIL_WIDTH_PERCENT_STEP as i16));
                false
            }
            KeyCode::Char('s') => {
                if self.active_tab == TuiTab::Issues && self.selected_ticket().is_some() {
                    self.mode = Mode::State;
                } else {
                    self.status = Some("Select a ticket first.".to_string());
                }
                false
            }
            KeyCode::Char('S') => {
                self.start_sync();
                false
            }
            KeyCode::Char(c) if ('1'..='9').contains(&c) => {
                self.jump_linked_item(usize::from(c as u8 - b'1'));
                false
            }
            _ => false,
        };
        Ok(quit)
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

    fn handle_tags_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') => self.next_tag_filter(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_tag_filter(),
            KeyCode::Char(' ') => self.toggle_selected_tag_filter(),
            KeyCode::Char('a') => {
                self.tag_filter_match_all = !self.tag_filter_match_all;
                self.apply_filter();
            }
            KeyCode::Char('c') => {
                self.tag_filter.clear();
                self.apply_filter();
            }
            _ => {}
        }
    }

    fn handle_manage_tags_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') => self.next_manage_tag(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_manage_tag(),
            KeyCode::Char(' ') => self.toggle_selected_target_tag()?,
            KeyCode::Char('n') => self.begin_input(InputKind::AddTags),
            KeyCode::Char('r') => self.begin_input(InputKind::RemoveTags),
            _ => {}
        }
        Ok(())
    }

    fn handle_order_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = Some("Cancelled.".to_string());
            }
            KeyCode::Enter => self.apply_selected_order()?,
            KeyCode::Down | KeyCode::Char('j') => self.next_order(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_order(),
            KeyCode::Char('1') => self.apply_order_choice(OrderChoice::Priority)?,
            KeyCode::Char('2') => self.apply_order_choice(OrderChoice::DateAsc)?,
            KeyCode::Char('3') => self.apply_order_choice(OrderChoice::DateDesc)?,
            KeyCode::Char('4') => self.apply_order_choice(OrderChoice::State)?,
            _ => {}
        }
        Ok(())
    }

    fn handle_columns_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') => self.next_column_choice(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_column_choice(),
            KeyCode::Char(' ') => self.toggle_selected_column(),
            KeyCode::Char('d') => {
                self.issue_columns = default_issue_columns();
                self.status = Some("Restored default columns.".to_string());
            }
            KeyCode::Char('V') => self.begin_save_view(),
            _ => {}
        }
        Ok(())
    }

    fn handle_saved_views_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                self.apply_selected_saved_view()?;
                self.mode = Mode::Normal;
            }
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') => self.next_saved_view(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_saved_view(),
            KeyCode::Char('d') => {
                self.clear_view_filters()?;
                self.mode = Mode::Normal;
            }
            KeyCode::Char('D') => self.begin_delete_saved_view(),
            _ => {}
        }
        Ok(())
    }

    fn handle_delete_view_confirm_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.delete_pending_saved_view()?,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.pending_delete_view = None;
                self.mode = Mode::SavedViews;
                self.status = Some("Cancelled.".to_string());
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_save_view_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.input.clear();
                self.status = Some("Cancelled.".to_string());
            }
            KeyCode::Enter => {
                let name = self.input.trim().to_string();
                if name.is_empty() {
                    self.status = Some("View name cannot be empty.".to_string());
                    return Ok(());
                }
                self.save_current_view(&name)?;
                self.mode = Mode::Normal;
                self.input.clear();
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

    fn handle_link_issue_search_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.input.clear();
                self.status = Some("Cancelled.".to_string());
            }
            KeyCode::Enter => {
                if self.link_selected_issue()? {
                    self.mode = Mode::Normal;
                    self.input.clear();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.next_link_issue_result(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_link_issue_result(),
            KeyCode::Backspace => {
                self.input.pop();
                self.reset_link_issue_selection();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.reset_link_issue_selection();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_unlink_issue_select_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = Some("Cancelled.".to_string());
            }
            KeyCode::Enter => {
                if self.unlink_selected_issue()? {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.next_unlink_issue(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_unlink_issue(),
            _ => {}
        }
        Ok(())
    }

    fn handle_versions_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') => self.next_version(),
            KeyCode::Up | KeyCode::Char('k') => self.previous_version(),
            _ => {}
        }
    }

    fn handle_state_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status = Some("Cancelled.".to_string());
            }
            KeyCode::Char('n') | KeyCode::Char('1') => {
                self.set_lifecycle(TicketStatus::Open, TicketState::New)?
            }
            KeyCode::Char('a') | KeyCode::Char('2') => {
                self.set_lifecycle(TicketStatus::Open, TicketState::Assigned)?
            }
            KeyCode::Char('p') | KeyCode::Char('3') => {
                self.set_lifecycle(TicketStatus::Open, TicketState::InProgress)?
            }
            KeyCode::Char('b') | KeyCode::Char('4') => {
                self.set_lifecycle(TicketStatus::Open, TicketState::Blocked)?
            }
            KeyCode::Char('v') | KeyCode::Char('5') => {
                self.set_lifecycle(TicketStatus::Open, TicketState::Review)?
            }
            KeyCode::Char('r') | KeyCode::Char('6') => {
                self.set_lifecycle(TicketStatus::Closed, TicketState::Resolved)?
            }
            KeyCode::Char('w') | KeyCode::Char('7') => {
                self.set_lifecycle(TicketStatus::Closed, TicketState::Wontfix)?
            }
            KeyCode::Char('u') | KeyCode::Char('8') => {
                self.set_lifecycle(TicketStatus::Closed, TicketState::Duplicate)?
            }
            KeyCode::Char('i') | KeyCode::Char('9') => {
                self.set_lifecycle(TicketStatus::Closed, TicketState::Invalid)?
            }
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

    fn begin_create_subissue(&mut self, parent_id: uuid::Uuid) {
        self.new_ticket = NewTicketDraft {
            parent: Some(parent_id),
            ..Default::default()
        };
        self.mode = Mode::Create;
    }

    fn begin_tag_filter(&mut self) {
        let tags = self.available_tags();
        if tags.is_empty() {
            self.tag_picker_state.select(None);
        } else {
            let selected = self
                .tag_picker_state
                .selected()
                .unwrap_or(0)
                .min(tags.len() - 1);
            self.tag_picker_state.select(Some(selected));
        }
        self.mode = Mode::Tags;
    }

    fn begin_manage_tags(&mut self) {
        let Some((_, _, target_tags)) = self.selected_tag_target() else {
            self.status = Some("Select an issue or writeup first.".to_string());
            return;
        };
        let tags = self.manageable_tags(&target_tags);
        if tags.is_empty() {
            self.manage_tag_state.select(None);
        } else {
            let selected = self
                .manage_tag_state
                .selected()
                .unwrap_or(0)
                .min(tags.len() - 1);
            self.manage_tag_state.select(Some(selected));
        }
        self.mode = Mode::ManageTags;
    }

    fn begin_order(&mut self) {
        self.order_state
            .select(Some(order_choice_index(self.current_order_choice())));
        self.mode = Mode::Order;
    }

    fn begin_columns(&mut self) {
        let selected = self
            .column_state
            .selected()
            .unwrap_or(0)
            .min(ISSUE_COLUMN_CHOICES.len().saturating_sub(1));
        self.column_state.select(Some(selected));
        self.mode = Mode::Columns;
    }

    fn begin_saved_views(&mut self) {
        let views = self.view_entries();
        let selected = self
            .saved_view_state
            .selected()
            .unwrap_or(0)
            .min(views.len().saturating_sub(1));
        self.saved_view_state.select(Some(selected));
        self.mode = Mode::SavedViews;
    }

    fn begin_delete_saved_view(&mut self) {
        let views = self.view_entries();
        let Some(entry) = self
            .saved_view_state
            .selected()
            .and_then(|selected| views.get(selected))
        else {
            self.status = Some("No view selected.".to_string());
            return;
        };
        if entry.kind != ViewKind::Saved {
            self.status = Some("Built-in views cannot be deleted.".to_string());
            return;
        }
        self.pending_delete_view = Some(entry.name.clone());
        self.mode = Mode::ConfirmDeleteView;
    }

    fn begin_save_view(&mut self) {
        self.input.clear();
        self.mode = Mode::SaveView;
    }

    fn save_current_view(&mut self, name: &str) -> Result<()> {
        let git_dir = self.store.session().repo_git_dir();
        let mut state = State::load().unwrap_or_default();
        let view = self.current_saved_view();
        let desc = crate::commands::view::describe_view(&view);
        state.set_last_filters(&git_dir, view.clone());
        state.save_view(&git_dir, name, view);
        state.save()?;
        self.status = Some(format!("Saved view `{name}`: {desc}"));
        Ok(())
    }

    fn delete_pending_saved_view(&mut self) -> Result<()> {
        let Some(name) = self.pending_delete_view.take() else {
            self.status = Some("No view selected.".to_string());
            self.mode = Mode::SavedViews;
            return Ok(());
        };
        let git_dir = self.store.session().repo_git_dir();
        let mut state = State::load().unwrap_or_default();
        if state.delete_view(&git_dir, &name) {
            state.save()?;
            if self.active_view_name.as_deref() == Some(name.as_str()) {
                self.active_view_name = None;
            }
            let views = self.view_entries();
            if views.is_empty() {
                self.saved_view_state.select(None);
            } else {
                let selected = self
                    .saved_view_state
                    .selected()
                    .unwrap_or(0)
                    .min(views.len().saturating_sub(1));
                self.saved_view_state.select(Some(selected));
            }
            self.status = Some(format!("Deleted view `{name}`."));
        } else {
            self.status = Some(format!("No view named `{name}`."));
        }
        self.mode = Mode::SavedViews;
        Ok(())
    }

    fn current_saved_view(&self) -> SavedView {
        let tags = self.tag_filter.iter().cloned().collect::<Vec<_>>();
        SavedView {
            created_at: None,
            status: self.base_status.map(|status| status.as_str().to_string()),
            state: self.base_state.map(|state| state.as_str().to_string()),
            tag: (tags.len() == 1).then(|| tags[0].clone()),
            tags,
            tag_match_all: self.tag_filter_match_all,
            assigned: self.assigned_filter.clone(),
            only_tagged: self.only_tagged,
            search: optional_trimmed(&self.filter).map(ToString::to_string),
            order: Some(if self.sort_closed_desc {
                "closed.desc".to_string()
            } else {
                self.current_order_choice().spec().to_string()
            }),
            all: self.base_status.is_none() && self.base_state.is_none(),
            subissues: !self.hide_subissues,
            limit: 0,
            columns: self
                .issue_columns
                .iter()
                .map(|column| column.as_str().to_string())
                .collect(),
        }
    }

    fn view_entries(&self) -> Vec<ViewEntry> {
        let builtins = builtin_views(self.store.email());
        let git_dir = self.store.session().repo_git_dir();
        let mut saved: Vec<_> = State::load()
            .map(|state| state.list_views(&git_dir))
            .unwrap_or_default()
            .into_iter()
            .map(|(name, view)| ViewEntry {
                name,
                view,
                kind: ViewKind::Saved,
            })
            .collect();
        if saved.is_empty() {
            builtins
        } else {
            saved.extend(builtins);
            saved
        }
    }

    fn next_saved_view(&mut self) {
        let views = self.view_entries();
        let selected = self.saved_view_state.selected().unwrap_or(0);
        self.saved_view_state
            .select(Some((selected + 1) % views.len()));
    }

    fn previous_saved_view(&mut self) {
        let views = self.view_entries();
        let selected = self.saved_view_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| views.len().saturating_sub(1));
        self.saved_view_state.select(Some(previous));
    }

    fn apply_selected_saved_view(&mut self) -> Result<()> {
        let views = self.view_entries();
        let Some(entry) = self
            .saved_view_state
            .selected()
            .and_then(|selected| views.get(selected))
        else {
            self.status = Some("No view selected.".to_string());
            return Ok(());
        };
        self.apply_saved_view(&entry.name, &entry.view)
    }

    fn apply_saved_view(&mut self, name: &str, view: &SavedView) -> Result<()> {
        self.active_view_name = Some(name.to_string());
        self.base_status = if view.all || view.state.is_some() {
            None
        } else if let Some(status) = view.status.as_deref() {
            Some(TicketStatus::parse(status)?)
        } else {
            Some(TicketStatus::Open)
        };
        self.base_state = None;
        if let Some(state) = view.state.as_deref() {
            let lifecycle = TicketLifecycle::parse(state)?;
            self.base_status = Some(lifecycle.status);
            if TicketStatus::parse(state).is_err() {
                self.base_state = Some(lifecycle.state);
            }
        }
        self.assigned_filter = view.assigned.clone();
        self.only_tagged = view.only_tagged;
        self.hide_subissues = !view.subissues;
        self.filter = view.search.clone().unwrap_or_default();
        self.tag_filter = saved_view_tags(view).into_iter().collect();
        self.tag_filter_match_all = view.tag_match_all;
        self.sort_closed_desc = matches!(view.order.as_deref(), Some("closed.desc" | "closed"));
        self.sort_order = match view.order.as_deref() {
            Some("closed.desc" | "closed") => None,
            Some(spec) => Some(
                SortOrder::parse(spec)
                    .ok_or_else(|| anyhow::anyhow!("unknown sort order `{spec}`"))?,
            ),
            None => None,
        };
        self.issue_columns = saved_issue_columns(view);
        self.detail = None;
        self.writeup_detail = None;
        self.comments_mode = false;
        self.active_tab = TuiTab::Issues;
        self.view = ViewMode::List;
        self.reload(None)?;
        self.status = Some(format!("Loaded view `{name}`."));
        Ok(())
    }

    fn clear_view_filters(&mut self) -> Result<()> {
        self.active_view_name = None;
        self.base_status = Some(TicketStatus::Open);
        self.base_state = None;
        self.assigned_filter = None;
        self.only_tagged = false;
        self.hide_subissues = !self.show_subissues_preference;
        self.sort_order = None;
        self.sort_closed_desc = false;
        self.issue_columns = default_issue_columns();
        self.filter.clear();
        self.tag_filter.clear();
        self.tag_filter_match_all = true;
        self.detail = None;
        self.writeup_detail = None;
        self.comments_mode = false;
        self.reload(None)?;
        self.status = Some("Cleared to default view.".to_string());
        Ok(())
    }

    fn has_active_view_filters(&self) -> bool {
        self.active_view_name.is_some()
            || self.base_status != Some(TicketStatus::Open)
            || self.base_state.is_some()
            || self.assigned_filter.is_some()
            || self.only_tagged
            || self.hide_subissues != !self.show_subissues_preference
            || self.sort_order.is_some()
            || !self.filter.is_empty()
            || !self.tag_filter.is_empty()
            || !self.tag_filter_match_all
    }

    fn edit_ticket_in_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            return Ok(());
        };
        let id = ticket.id;
        let initial = ticket_edit_body(ticket);

        suspend_terminal(terminal)?;
        let edited = editor::capture_with_initial(
            "Edit the title on the first line. Remaining non-comment lines become the description.",
            &initial,
        );
        resume_terminal(terminal)?;

        match edited? {
            Some(edited) => {
                let (title, description) = editor::parse_ticket_edit(&edited)?;
                self.store.set_title(&id, &title)?;
                self.store.set_description(&id, description.as_deref())?;
                self.status = Some("Updated ticket.".to_string());
            }
            _ => {
                self.status = Some("Cancelled.".to_string());
            }
        }

        self.reload(Some(id))?;
        Ok(())
    }

    fn edit_spec_in_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            return Ok(());
        };
        let id = ticket.id;
        let initial = ticket.spec.clone().unwrap_or_default();

        suspend_terminal(terminal)?;
        let edited = editor::capture_with_initial(
            "Write the implementation spec below. Lines starting with # are ignored.",
            &initial,
        );
        resume_terminal(terminal)?;

        match edited? {
            Some(spec) if !spec.trim().is_empty() => {
                self.store.set_spec(&id, Some(spec.trim()))?;
                self.status = Some("Updated spec.".to_string());
            }
            _ => {
                self.store.set_spec(&id, None)?;
                self.status = Some("Cleared spec.".to_string());
            }
        }

        self.reload(Some(id))?;
        Ok(())
    }

    fn edit_writeup_in_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return Ok(());
        };
        let id = writeup.id;
        let initial = writeup_edit_body(writeup);

        suspend_terminal(terminal)?;
        let edited = editor::capture_with_initial(
            "Edit the title on the first line. Remaining non-comment lines become the writeup body.",
            &initial,
        );
        resume_terminal(terminal)?;

        match edited? {
            Some(edited) => {
                let (title, body) = editor::parse_ticket_edit(&edited)?;
                self.store.set_writeup_title(&id, &title)?;
                if let Some(body) = body {
                    self.store.append_writeup_version(&id, &body)?;
                }
                self.status = Some("Appended writeup version.".to_string());
            }
            _ => {
                self.status = Some("Cancelled.".to_string());
            }
        }

        self.reload_writeups(Some(id))?;
        Ok(())
    }

    fn create_writeup_in_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        suspend_terminal(terminal)?;
        let edited = editor::capture(
            "Write the title on the first line. Remaining non-comment lines become the writeup body.",
        );
        resume_terminal(terminal)?;

        let Some(edited) = edited? else {
            self.status = Some("Cancelled.".to_string());
            return Ok(());
        };
        let (title, body) = editor::parse_ticket_edit(&edited)?;
        let writeup = self.store.create_writeup(
            &title,
            NewWriteupOpts {
                body,
                ..Default::default()
            },
        )?;
        self.active_tab = TuiTab::Writeups;
        self.show_all_writeups = false;
        self.reload_writeups(Some(writeup.id))?;
        self.jump_to_writeup(writeup.id);
        self.status = Some(format!("Created writeup {}.", writeup.short_id()));
        Ok(())
    }

    fn add_comment_in_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            return Ok(());
        };
        let id = ticket.id;
        let title = ticket.title.clone();

        suspend_terminal(terminal)?;
        let comment = editor::capture_comment(&title);
        resume_terminal(terminal)?;

        match comment? {
            Some(comment) if !comment.trim().is_empty() => {
                self.store.add_comment(&id, comment.trim())?;
                self.status = Some("Added comment.".to_string());
            }
            _ => {
                self.status = Some("Cancelled.".to_string());
            }
        }

        self.reload(Some(id))?;
        if let Some(ticket) = self.selected_ticket() {
            if !ticket.comments.is_empty() {
                self.comment_state.select(Some(ticket.comments.len() - 1));
            }
        }
        Ok(())
    }

    fn promote_selected_writeup(&mut self) -> Result<()> {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return Ok(());
        };
        let writeup_id = writeup.id;
        let ticket = self.store.promote_writeup(&writeup_id)?;
        self.reload_all(Some(ticket.id), Some(writeup_id))?;
        self.status = Some(format!("Promoted to issue {}.", ticket.short_id()));
        Ok(())
    }

    fn set_selected_writeup_status(&mut self, status: WriteupStatus) -> Result<()> {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return Ok(());
        };
        let id = writeup.id;
        self.store.set_writeup_status(&id, status)?;
        self.reload_writeups(Some(id))?;
        self.status = Some(format!(
            "Writeup {} -> {}.",
            &id.to_string()[..6],
            status.as_str()
        ));
        Ok(())
    }

    fn toggle_writeup_scope(&mut self) {
        let selected_id = self.selected_writeup().map(|writeup| writeup.id);
        self.show_all_writeups = !self.show_all_writeups;
        self.apply_writeup_filter();
        if let Some(id) = selected_id {
            if let Some(visible_pos) = self
                .visible_writeups
                .iter()
                .position(|idx| self.writeups[*idx].id == id)
            {
                self.writeup_state.select(Some(visible_pos));
                if self.writeup_detail.is_some() {
                    self.writeup_detail = self.visible_writeups.get(visible_pos).copied();
                }
            }
        }
        self.status = Some(if self.show_all_writeups {
            "Showing all writeups.".to_string()
        } else {
            "Showing open writeups.".to_string()
        });
    }

    fn linked_writeups(&self, ticket_id: uuid::Uuid) -> Vec<&Writeup> {
        self.writeups
            .iter()
            .filter(|writeup| writeup.tickets.contains(&ticket_id))
            .collect()
    }

    fn jump_linked_item(&mut self, index: usize) {
        match self.active_tab {
            TuiTab::Issues => {
                let Some(ticket) = self.selected_ticket() else {
                    return;
                };
                let ticket_id = ticket.id;
                let Some(writeup_id) = self
                    .linked_writeups(ticket_id)
                    .get(index)
                    .map(|writeup| writeup.id)
                else {
                    self.status = Some("No linked writeup at that number.".to_string());
                    return;
                };
                self.jump_to_writeup(writeup_id);
            }
            TuiTab::Writeups => {
                let Some(writeup) = self.selected_writeup() else {
                    return;
                };
                let Some(ticket_id) = writeup.tickets.iter().nth(index).copied() else {
                    self.status = Some("No linked issue at that number.".to_string());
                    return;
                };
                self.jump_to_ticket(ticket_id);
            }
            TuiTab::Dashboard => {}
        }
    }

    fn jump_to_writeup(&mut self, id: uuid::Uuid) {
        let Some(idx) = self.writeups.iter().position(|writeup| writeup.id == id) else {
            self.status = Some("Linked writeup is not loaded.".to_string());
            return;
        };
        self.active_tab = TuiTab::Writeups;
        self.view = ViewMode::List;
        self.comments_mode = false;
        self.writeup_detail = Some(idx);
        self.writeup_detail_focus = WriteupPaneFocus::Detail;
        self.writeup_detail_scroll = 0;
        self.writeup_toc_open = false;
        self.writeup_toc_state.select(None);
        if let Some(visible_pos) = self
            .visible_writeups
            .iter()
            .position(|visible| *visible == idx)
        {
            self.writeup_state.select(Some(visible_pos));
        }
    }

    fn jump_to_ticket(&mut self, id: uuid::Uuid) {
        self.active_tab = TuiTab::Issues;
        self.view = ViewMode::List;
        self.open_ticket_by_id(id);
        if self.detail.is_none() {
            self.status = Some("Linked issue is hidden by current filters.".to_string());
        }
    }

    fn jump_to_parent_issue(&mut self) {
        let Some(ticket) = self.detail.map(|idx| &self.tickets[idx]) else {
            self.status = Some("Open ticket details first.".to_string());
            return;
        };
        let Some(parent) = ticket.parent else {
            self.status = Some("This issue has no parent.".to_string());
            return;
        };
        self.open_ticket_by_id(parent);
        if self.detail.map(|idx| self.tickets[idx].id) == Some(parent) {
            self.status = Some(format!("Opened parent {}.", self.issue_label(parent)));
        } else {
            self.status = Some("Parent issue is hidden by current filters.".to_string());
        }
    }

    fn start_sync(&mut self) {
        if self.sync.is_some() {
            self.status = Some("Sync already running.".to_string());
            return;
        }

        let selected_id = self.selected_ticket().map(|ticket| ticket.id);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = run_ti_sync_command();
            let _ = sender.send(result);
        });

        self.sync = Some(SyncState {
            receiver,
            selected_id,
            started_at: Instant::now(),
        });
        self.status = Some("Syncing tickets...".to_string());
    }

    fn poll_sync(&mut self) -> Result<()> {
        let completed = match &self.sync {
            Some(sync) => match sync.receiver.try_recv() {
                Ok(result) => Some((result, sync.selected_id)),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some((
                    Err(anyhow::anyhow!(
                        "sync worker stopped before reporting a result"
                    )),
                    sync.selected_id,
                )),
            },
            None => None,
        };

        let Some((result, selected_id)) = completed else {
            return Ok(());
        };

        self.sync = None;
        match result {
            Ok(result) => {
                self.reload_all(
                    selected_id,
                    self.selected_writeup().map(|writeup| writeup.id),
                )?;
                self.status = Some(result.summary);
            }
            Err(err) => {
                self.status = Some(format!("Sync failed: {err}"));
            }
        }
        Ok(())
    }

    fn begin_input(&mut self, kind: InputKind) {
        let ticket = match kind {
            InputKind::AddTags | InputKind::RemoveTags => {
                if self.selected_tag_target().is_none() {
                    self.status = Some("Select an issue or writeup first.".to_string());
                    return;
                }
                None
            }
            InputKind::Priority if self.active_tab == TuiTab::Writeups => {
                let Some(writeup) = self.selected_writeup() else {
                    self.status = Some("Select a writeup first.".to_string());
                    return;
                };
                self.input = writeup
                    .priority
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                self.mode = Mode::Input(kind);
                return;
            }
            _ => {
                let Some(ticket) = self.selected_ticket() else {
                    self.status = Some("Select a ticket first.".to_string());
                    return;
                };
                Some(ticket)
            }
        };

        self.input = match kind {
            InputKind::Priority => ticket
                .and_then(|ticket| ticket.priority)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            InputKind::Points => ticket
                .and_then(|ticket| ticket.points)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            InputKind::AddTags | InputKind::RemoveTags => String::new(),
        };
        self.mode = Mode::Input(kind);
    }

    fn begin_link_issue_search(&mut self) {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return;
        };
        let linked_tickets = writeup.tickets.clone();
        self.input.clear();
        let has_unlinked_issue = self
            .tickets
            .iter()
            .any(|ticket| !linked_tickets.contains(&ticket.id));
        if !has_unlinked_issue {
            self.status = Some("No unlinked issues available.".to_string());
            return;
        }
        self.link_issue_state.select(Some(0));
        self.mode = Mode::LinkIssueSearch;
    }

    fn begin_unlink_issue_select(&mut self) {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return;
        };
        if writeup.tickets.is_empty() {
            self.status = Some("No linked issues.".to_string());
            return;
        }
        self.link_issue_state.select(Some(0));
        self.mode = Mode::UnlinkIssueSelect;
    }

    fn begin_versions(&mut self) {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return;
        };
        if writeup.versions.is_empty() {
            self.version_state.select(None);
        } else {
            self.version_state
                .select(Some(writeup.versions.len().saturating_sub(1)));
        }
        self.mode = Mode::Versions;
    }

    fn priority_range_display(&self) -> String {
        let mut priorities: Box<dyn Iterator<Item = i64> + '_> =
            if self.active_tab == TuiTab::Writeups {
                Box::new(
                    self.visible_writeups
                        .iter()
                        .filter_map(|idx| self.writeups[*idx].priority),
                )
            } else {
                Box::new(
                    self.visible
                        .iter()
                        .filter_map(|idx| self.tickets[*idx].priority),
                )
            };
        let Some(first) = priorities.next() else {
            return "No priorities set.".to_string();
        };
        let (min, max) = priorities.fold((first, first), |(min, max), priority| {
            (min.min(priority), max.max(priority))
        });
        if min == max {
            format!("Current priority: {min}.")
        } else {
            format!("Current priority range: {min}-{max}.")
        }
    }

    fn submit_input(&mut self) -> Result<bool> {
        let Mode::Input(kind) = self.mode else {
            return Ok(false);
        };
        if matches!(kind, InputKind::AddTags | InputKind::RemoveTags) {
            return self.submit_tag_input(kind);
        }
        if kind == InputKind::Priority && self.active_tab == TuiTab::Writeups {
            return self.submit_writeup_priority_input();
        }

        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            return Ok(false);
        };
        let id = ticket.id;
        let preferred_after_reload = if kind == InputKind::Priority {
            self.adjacent_ticket_for_priority_triage(id)
        } else {
            Some(id)
        };

        match kind {
            InputKind::Priority => {
                let priority = match parse_optional_i64(&self.input, "priority") {
                    Ok(priority) => priority,
                    Err(err) => {
                        self.status = Some(err.to_string());
                        return Ok(false);
                    }
                };
                self.store.set_priority(&id, priority)?;
                self.status = Some(match priority {
                    Some(value) => format!("Set priority to {value}."),
                    None => "Cleared priority.".to_string(),
                });
            }
            InputKind::Points => {
                let points = match parse_optional_i64(&self.input, "points") {
                    Ok(points) => points,
                    Err(err) => {
                        self.status = Some(err.to_string());
                        return Ok(false);
                    }
                };
                self.store.set_points(&id, points)?;
                self.status = Some(match points {
                    Some(value) => format!("Set points to {value}."),
                    None => "Cleared points.".to_string(),
                });
            }
            InputKind::AddTags | InputKind::RemoveTags => unreachable!("handled above"),
        }

        self.reload(preferred_after_reload)?;
        Ok(true)
    }

    fn submit_writeup_priority_input(&mut self) -> Result<bool> {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return Ok(false);
        };
        let id = writeup.id;
        let priority = match parse_optional_i64(&self.input, "priority") {
            Ok(priority) => priority,
            Err(err) => {
                self.status = Some(err.to_string());
                return Ok(false);
            }
        };
        self.store.set_writeup_priority(&id, priority)?;
        self.status = Some(match priority {
            Some(value) => format!("Set writeup priority to {value}."),
            None => "Cleared writeup priority.".to_string(),
        });
        self.reload_writeups(Some(id))?;
        Ok(true)
    }

    fn submit_tag_input(&mut self, kind: InputKind) -> Result<bool> {
        let Some((target, _, _)) = self.selected_tag_target() else {
            self.status = Some("Select an issue or writeup first.".to_string());
            return Ok(false);
        };
        let tags = split_tags(&self.input);
        if tags.is_empty() {
            self.status = Some("Enter at least one tag.".to_string());
            return Ok(false);
        }

        match target {
            TagTarget::Ticket(id) => {
                for tag in tags {
                    match kind {
                        InputKind::AddTags => self.store.add_tag(&id, &tag)?,
                        InputKind::RemoveTags => self.store.remove_tag(&id, &tag)?,
                        _ => unreachable!("only tag inputs are submitted here"),
                    }
                }
                self.reload(Some(id))?;
            }
            TagTarget::Writeup(id) => {
                for tag in tags {
                    match kind {
                        InputKind::AddTags => self.store.add_writeup_tag(&id, &tag)?,
                        InputKind::RemoveTags => self.store.remove_writeup_tag(&id, &tag)?,
                        _ => unreachable!("only tag inputs are submitted here"),
                    }
                }
                self.reload_writeups(Some(id))?;
            }
        }
        self.status = Some(match kind {
            InputKind::AddTags => "Added tag(s).".to_string(),
            InputKind::RemoveTags => "Removed tag(s).".to_string(),
            _ => unreachable!("only tag inputs are submitted here"),
        });
        Ok(true)
    }

    fn link_selected_issue(&mut self) -> Result<bool> {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return Ok(false);
        };
        let writeup_id = writeup.id;
        let results = self.link_issue_search_results();
        let Some(ticket_id) = self
            .link_issue_state
            .selected()
            .and_then(|selected| results.get(selected))
            .map(|idx| self.tickets[*idx].id)
        else {
            self.status = Some("No issue selected.".to_string());
            return Ok(false);
        };
        self.store.link_writeup_ticket(&writeup_id, &ticket_id)?;
        self.reload_all(Some(ticket_id), Some(writeup_id))?;
        self.status = Some(format!("Linked issue {}.", &ticket_id.to_string()[..6]));
        Ok(true)
    }

    fn link_issue_search_results(&self) -> Vec<usize> {
        let Some(writeup) = self.selected_writeup() else {
            return Vec::new();
        };
        let needle = self.input.trim().to_ascii_lowercase();
        self.tickets
            .iter()
            .enumerate()
            .filter_map(|(idx, ticket)| {
                if writeup.tickets.contains(&ticket.id) {
                    return None;
                }
                if needle.is_empty() || ticket_matches(ticket, &needle) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    fn next_link_issue_result(&mut self) {
        let results = self.link_issue_search_results();
        if results.is_empty() {
            self.link_issue_state.select(None);
            return;
        }
        let selected = self.link_issue_state.selected().unwrap_or(0);
        self.link_issue_state
            .select(Some((selected + 1) % results.len()));
    }

    fn previous_link_issue_result(&mut self) {
        let results = self.link_issue_search_results();
        if results.is_empty() {
            self.link_issue_state.select(None);
            return;
        }
        let selected = self.link_issue_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| results.len().saturating_sub(1));
        self.link_issue_state.select(Some(previous));
    }

    fn reset_link_issue_selection(&mut self) {
        if self.link_issue_search_results().is_empty() {
            self.link_issue_state.select(None);
        } else {
            self.link_issue_state.select(Some(0));
        }
    }

    fn linked_issue_ids_for_selected_writeup(&self) -> Vec<uuid::Uuid> {
        self.selected_writeup()
            .map(|writeup| writeup.tickets.iter().copied().collect())
            .unwrap_or_default()
    }

    fn linked_issue_line(&self, ticket_id: uuid::Uuid, width: usize) -> Line<'static> {
        if let Some(ticket) = self.tickets.iter().find(|ticket| ticket.id == ticket_id) {
            return ticket_list_line(ticket, width, false, self.store.email(), false);
        }
        let short_id = ticket_id.to_string()[..6].to_string();
        ticket_list_line_from_parts(
            Some(&short_id),
            "missing issue",
            &[],
            None,
            false,
            width,
            None,
        )
    }

    fn issue_label(&self, id: uuid::Uuid) -> String {
        let short_id = id.to_string().chars().take(6).collect::<String>();
        let title = self
            .all_tickets
            .iter()
            .find(|ticket| ticket.id == id)
            .map(|ticket| flatten_display(&ticket.title))
            .unwrap_or_else(|| "missing issue".to_string());
        format!("{short_id} {title}")
    }

    fn unlink_selected_issue(&mut self) -> Result<bool> {
        let Some(writeup) = self.selected_writeup() else {
            self.status = Some("Select a writeup first.".to_string());
            return Ok(false);
        };
        let writeup_id = writeup.id;
        let linked = self.linked_issue_ids_for_selected_writeup();
        let Some(ticket_id) = self
            .link_issue_state
            .selected()
            .and_then(|selected| linked.get(selected))
            .copied()
        else {
            self.status = Some("No issue selected.".to_string());
            return Ok(false);
        };
        self.store.unlink_writeup_ticket(&writeup_id, &ticket_id)?;
        self.reload_all(None, Some(writeup_id))?;
        self.status = Some(format!("Unlinked issue {}.", &ticket_id.to_string()[..6]));
        Ok(true)
    }

    fn next_unlink_issue(&mut self) {
        let linked = self.linked_issue_ids_for_selected_writeup();
        if linked.is_empty() {
            self.link_issue_state.select(None);
            return;
        }
        let selected = self.link_issue_state.selected().unwrap_or(0);
        self.link_issue_state
            .select(Some((selected + 1) % linked.len()));
    }

    fn previous_unlink_issue(&mut self) {
        let linked = self.linked_issue_ids_for_selected_writeup();
        if linked.is_empty() {
            self.link_issue_state.select(None);
            return;
        }
        let selected = self.link_issue_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| linked.len().saturating_sub(1));
        self.link_issue_state.select(Some(previous));
    }

    fn next_version(&mut self) {
        let Some(writeup) = self.selected_writeup() else {
            self.version_state.select(None);
            return;
        };
        if writeup.versions.is_empty() {
            self.version_state.select(None);
            return;
        }
        let selected = self.version_state.selected().unwrap_or(0);
        self.version_state
            .select(Some((selected + 1) % writeup.versions.len()));
    }

    fn previous_version(&mut self) {
        let Some(writeup) = self.selected_writeup() else {
            self.version_state.select(None);
            return;
        };
        if writeup.versions.is_empty() {
            self.version_state.select(None);
            return;
        }
        let selected = self.version_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| writeup.versions.len().saturating_sub(1));
        self.version_state.select(Some(previous));
    }

    fn adjacent_ticket_for_priority_triage(&self, id: uuid::Uuid) -> Option<uuid::Uuid> {
        let selected = self
            .visible
            .iter()
            .position(|idx| self.tickets[*idx].id == id)?;
        self.visible
            .get(selected + 1)
            .or_else(|| {
                selected
                    .checked_sub(1)
                    .and_then(|previous| self.visible.get(previous))
            })
            .map(|idx| self.tickets[*idx].id)
    }

    fn set_lifecycle(&mut self, status: TicketStatus, state: TicketState) -> Result<()> {
        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            self.mode = Mode::Normal;
            return Ok(());
        };
        let id = ticket.id;
        self.store.set_lifecycle(&id, status, state)?;
        self.status = Some(format!("Changed lifecycle to {status}:{state}."));
        self.mode = Mode::Normal;
        self.reload(Some(id))?;
        Ok(())
    }

    fn claim_selected(&mut self) -> Result<()> {
        let Some(ticket) = self.selected_ticket() else {
            self.status = Some("Select a ticket first.".to_string());
            return Ok(());
        };
        let id = ticket.id;
        let email = self.store.email().to_string();
        self.store.set_assigned(&id, Some(&email))?;
        self.store
            .set_lifecycle(&id, TicketStatus::Open, TicketState::Assigned)?;
        self.status = Some(format!("Claimed ticket as {email}."));
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
                parent: self.new_ticket.parent,
                ..Default::default()
            },
        )?;
        let id = ticket.id;
        if let Some(description) = optional_trimmed(&self.new_ticket.description) {
            self.store.set_description(&id, Some(description))?;
        }

        self.filter.clear();
        if self.new_ticket.parent.is_some() {
            self.hide_subissues = false;
        }
        self.detail = Some(0);
        self.reload(Some(id))?;
        self.open_ticket_by_id(id);
        self.status = Some(format!("Created {}.", ticket.short_id()));
        Ok(true)
    }

    fn active_filter_display(&self) -> String {
        let mut parts = Vec::new();
        if let Some(name) = &self.active_view_name {
            parts.push(format!("view: {name}"));
        }
        if let Some(state) = self.base_state {
            parts.push(format!("state: {}", state.as_str()));
        } else if let Some(status) = self.base_status {
            if status != TicketStatus::Open {
                parts.push(format!("status: {}", status.as_str()));
            }
        } else {
            parts.push("all".to_string());
        }
        if let Some(assigned) = &self.assigned_filter {
            parts.push(format!("assigned: {assigned}"));
        }
        if self.only_tagged {
            parts.push("tagged only".to_string());
        }
        if !self.filter.is_empty() {
            parts.push(format!("\"{}\"", self.filter));
        }
        if !self.tag_filter.is_empty() {
            let mode = if self.tag_filter_match_all {
                "all"
            } else {
                "any"
            };
            let tags = self
                .tag_filter
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("{mode} tags: {tags}"));
        }
        parts.join("; ")
    }

    fn available_tags(&self) -> Vec<(String, usize)> {
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for ticket in &self.tickets {
            for tag in &ticket.tags {
                *counts.entry(tag.clone()).or_default() += 1;
            }
        }
        counts.into_iter().collect()
    }

    fn next_tag_filter(&mut self) {
        let tags = self.available_tags();
        if tags.is_empty() {
            self.tag_picker_state.select(None);
            return;
        }
        let selected = self.tag_picker_state.selected().unwrap_or(0);
        self.tag_picker_state
            .select(Some((selected + 1) % tags.len()));
    }

    fn previous_tag_filter(&mut self) {
        let tags = self.available_tags();
        if tags.is_empty() {
            self.tag_picker_state.select(None);
            return;
        }
        let selected = self.tag_picker_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| tags.len().saturating_sub(1));
        self.tag_picker_state.select(Some(previous));
    }

    fn toggle_selected_tag_filter(&mut self) {
        let tags = self.available_tags();
        let Some((tag, _)) = self
            .tag_picker_state
            .selected()
            .and_then(|selected| tags.get(selected))
        else {
            return;
        };
        if !self.tag_filter.remove(tag) {
            self.tag_filter.insert(tag.clone());
        }
        self.apply_filter();
    }

    fn current_order_choice(&self) -> OrderChoice {
        match self.sort_order {
            Some(order) if order.key == SortKey::Created && !order.desc => OrderChoice::DateAsc,
            Some(order) if order.key == SortKey::Created && order.desc => OrderChoice::DateDesc,
            Some(order) if order.key == SortKey::State => OrderChoice::State,
            Some(order) if order.key == SortKey::Priority => OrderChoice::Priority,
            _ => OrderChoice::Priority,
        }
    }

    fn next_order(&mut self) {
        let selected = self.order_state.selected().unwrap_or(0);
        self.order_state
            .select(Some((selected + 1) % ORDER_CHOICES.len()));
    }

    fn previous_order(&mut self) {
        let selected = self.order_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| ORDER_CHOICES.len() - 1);
        self.order_state.select(Some(previous));
    }

    fn next_column_choice(&mut self) {
        let selected = self.column_state.selected().unwrap_or(0);
        self.column_state
            .select(Some((selected + 1) % ISSUE_COLUMN_CHOICES.len()));
    }

    fn previous_column_choice(&mut self) {
        let selected = self.column_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| ISSUE_COLUMN_CHOICES.len() - 1);
        self.column_state.select(Some(previous));
    }

    fn toggle_selected_column(&mut self) {
        let selected = self.column_state.selected().unwrap_or(0);
        let Some(column) = ISSUE_COLUMN_CHOICES.get(selected).copied() else {
            return;
        };
        if column == IssueColumn::Title {
            self.status = Some("Title column is required.".to_string());
            return;
        }
        if let Some(idx) = self
            .issue_columns
            .iter()
            .position(|candidate| *candidate == column)
        {
            self.issue_columns.remove(idx);
        } else {
            self.issue_columns.push(column);
            self.issue_columns
                .sort_by_key(|column| issue_column_index(*column));
        }
        if !self.issue_columns.contains(&IssueColumn::Title) {
            self.issue_columns.push(IssueColumn::Title);
            self.issue_columns
                .sort_by_key(|column| issue_column_index(*column));
        }
    }

    fn apply_selected_order(&mut self) -> Result<()> {
        let selected = self.order_state.selected().unwrap_or(0);
        let choice = ORDER_CHOICES
            .get(selected)
            .copied()
            .unwrap_or(OrderChoice::Priority);
        self.apply_order_choice(choice)
    }

    fn apply_order_choice(&mut self, choice: OrderChoice) -> Result<()> {
        let selected_id = self.selected_ticket().map(|ticket| ticket.id);
        self.sort_order = choice.sort_order();
        self.mode = Mode::Normal;
        self.reload(selected_id)?;
        self.status = Some(format!("Ordered by {}.", choice.label()));
        Ok(())
    }

    fn manageable_tags(&self, current_tags: &BTreeSet<String>) -> Vec<String> {
        let mut tags = current_tags.clone();
        for ticket in &self.tickets {
            tags.extend(ticket.tags.iter().cloned());
        }
        for writeup in &self.writeups {
            tags.extend(writeup.tags.iter().cloned());
        }
        tags.into_iter().collect()
    }

    fn next_manage_tag(&mut self) {
        let Some((_, _, current_tags)) = self.selected_tag_target() else {
            self.manage_tag_state.select(None);
            return;
        };
        let tags = self.manageable_tags(&current_tags);
        if tags.is_empty() {
            self.manage_tag_state.select(None);
            return;
        }
        let selected = self.manage_tag_state.selected().unwrap_or(0);
        self.manage_tag_state
            .select(Some((selected + 1) % tags.len()));
    }

    fn previous_manage_tag(&mut self) {
        let Some((_, _, current_tags)) = self.selected_tag_target() else {
            self.manage_tag_state.select(None);
            return;
        };
        let tags = self.manageable_tags(&current_tags);
        if tags.is_empty() {
            self.manage_tag_state.select(None);
            return;
        }
        let selected = self.manage_tag_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| tags.len().saturating_sub(1));
        self.manage_tag_state.select(Some(previous));
    }

    fn toggle_selected_target_tag(&mut self) -> Result<()> {
        let Some((target, _, current_tags)) = self.selected_tag_target() else {
            self.status = Some("Select an issue or writeup first.".to_string());
            return Ok(());
        };
        let tags = self.manageable_tags(&current_tags);
        let Some(tag) = self
            .manage_tag_state
            .selected()
            .and_then(|selected| tags.get(selected))
            .cloned()
        else {
            self.status = Some("No tag selected.".to_string());
            return Ok(());
        };

        match target {
            TagTarget::Ticket(id) => {
                if current_tags.contains(&tag) {
                    self.store.remove_tag(&id, &tag)?;
                    self.status = Some(format!("Removed tag `{tag}`."));
                } else {
                    self.store.add_tag(&id, &tag)?;
                    self.status = Some(format!("Added tag `{tag}`."));
                }
                self.reload(Some(id))?;
                if !self.visible.iter().any(|idx| self.tickets[*idx].id == id) {
                    self.mode = Mode::Normal;
                }
            }
            TagTarget::Writeup(id) => {
                if current_tags.contains(&tag) {
                    self.store.remove_writeup_tag(&id, &tag)?;
                    self.status = Some(format!("Removed tag `{tag}`."));
                } else {
                    self.store.add_writeup_tag(&id, &tag)?;
                    self.status = Some(format!("Added tag `{tag}`."));
                }
                self.reload_writeups(Some(id))?;
                if !self
                    .visible_writeups
                    .iter()
                    .any(|idx| self.writeups[*idx].id == id)
                {
                    self.mode = Mode::Normal;
                }
            }
        }
        Ok(())
    }

    fn apply_filter(&mut self) {
        let needle = self.filter.to_ascii_lowercase();
        self.visible = self
            .tickets
            .iter()
            .enumerate()
            .filter_map(|(idx, ticket)| {
                if (needle.is_empty() || ticket_matches(ticket, &needle))
                    && ticket_matches_tag_filter(
                        ticket,
                        &self.tag_filter,
                        self.tag_filter_match_all,
                    )
                {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();

        self.detail = self.detail.filter(|idx| self.visible.contains(idx));
        self.sync_board_selection();
        let list_len = self.list_ticket_indices().len();
        if list_len == 0 {
            self.list_state.select(None);
        } else {
            let selected = self.list_state.selected().unwrap_or(0).min(list_len - 1);
            self.list_state.select(Some(selected));
        }
        self.apply_writeup_filter();
    }

    fn apply_writeup_filter(&mut self) {
        let needle = self.filter.to_ascii_lowercase();
        self.visible_writeups = self
            .writeups
            .iter()
            .enumerate()
            .filter_map(|(idx, writeup)| {
                if (self.show_all_writeups || writeup.status == WriteupStatus::Open)
                    && (needle.is_empty() || writeup_matches(writeup, &needle))
                {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();
        self.visible_writeups
            .sort_by(|a, b| compare_tui_writeups(&self.writeups[*a], &self.writeups[*b]));

        self.writeup_detail = self
            .writeup_detail
            .filter(|idx| self.visible_writeups.contains(idx));
        if self.writeup_detail.is_none() {
            self.writeup_detail_focus = WriteupPaneFocus::List;
            self.writeup_detail_scroll = 0;
            self.writeup_toc_open = false;
            self.writeup_toc_state.select(None);
        }
        if self.visible_writeups.is_empty() {
            self.writeup_state.select(None);
        } else {
            let selected = self
                .writeup_state
                .selected()
                .unwrap_or(0)
                .min(self.visible_writeups.len() - 1);
            self.writeup_state.select(Some(selected));
        }
    }

    fn next(&mut self) {
        if self.active_tab == TuiTab::Writeups {
            self.next_writeup();
            return;
        }
        if self.active_tab == TuiTab::Dashboard {
            return;
        }
        let list_len = self.list_ticket_indices().len();
        if list_len == 0 {
            return;
        }
        let selected = self.list_state.selected().unwrap_or(0);
        let next = (selected + 1) % list_len;
        self.list_state.select(Some(next));
        self.sync_open_detail();
    }

    fn previous(&mut self) {
        if self.active_tab == TuiTab::Writeups {
            self.previous_writeup();
            return;
        }
        if self.active_tab == TuiTab::Dashboard {
            return;
        }
        let list_len = self.list_ticket_indices().len();
        if list_len == 0 {
            return;
        }
        let selected = self.list_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| list_len.saturating_sub(1));
        self.list_state.select(Some(previous));
        self.sync_open_detail();
    }

    fn list_ticket_indices(&self) -> Vec<usize> {
        ordered_list_indices(&self.tickets, &self.visible, !self.hide_subissues)
    }

    fn next_writeup(&mut self) {
        if self.visible_writeups.is_empty() {
            return;
        }
        let selected = self.writeup_state.selected().unwrap_or(0);
        let next = (selected + 1) % self.visible_writeups.len();
        self.writeup_state.select(Some(next));
        self.sync_open_writeup_detail();
    }

    fn previous_writeup(&mut self) {
        if self.visible_writeups.is_empty() {
            return;
        }
        let selected = self.writeup_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| self.visible_writeups.len().saturating_sub(1));
        self.writeup_state.select(Some(previous));
        self.sync_open_writeup_detail();
    }

    fn focus_next_writeup_pane(&mut self) {
        self.writeup_detail_focus = match self.writeup_detail_focus {
            WriteupPaneFocus::List => WriteupPaneFocus::Detail,
            WriteupPaneFocus::Detail if self.writeup_toc_open => WriteupPaneFocus::Toc,
            WriteupPaneFocus::Detail | WriteupPaneFocus::Toc => WriteupPaneFocus::Detail,
        };
    }

    fn focus_previous_writeup_pane(&mut self) {
        self.writeup_detail_focus = match self.writeup_detail_focus {
            WriteupPaneFocus::Toc => WriteupPaneFocus::Detail,
            WriteupPaneFocus::Detail => WriteupPaneFocus::List,
            WriteupPaneFocus::List => WriteupPaneFocus::List,
        };
    }

    fn scroll_writeup_detail(&mut self, delta: i16) {
        self.writeup_detail_scroll = if delta.is_negative() {
            self.writeup_detail_scroll
                .saturating_sub(delta.unsigned_abs())
        } else {
            self.writeup_detail_scroll.saturating_add(delta as u16)
        };
        self.sync_writeup_toc_to_scroll();
    }

    fn toggle_writeup_toc(&mut self) {
        if self.writeup_toc_open {
            self.writeup_toc_open = false;
            self.writeup_detail_focus = WriteupPaneFocus::Detail;
            return;
        }
        let headings = self.current_writeup_headings();
        if headings.is_empty() {
            self.status = Some("No markdown headings in this writeup.".to_string());
            return;
        }
        self.writeup_toc_open = true;
        self.writeup_detail_focus = WriteupPaneFocus::Toc;
        self.sync_writeup_toc_selection(&headings);
    }

    fn current_writeup_headings(&self) -> Vec<MarkdownHeading> {
        let Some(writeup) = self.writeup_detail.map(|idx| &self.writeups[idx]) else {
            return Vec::new();
        };
        let (_, headings) = writeup_detail_lines(writeup, &[], usize::MAX);
        headings
    }

    fn next_writeup_heading(&mut self) {
        let headings = self.current_writeup_headings();
        if headings.is_empty() {
            self.writeup_toc_state.select(None);
            return;
        }
        let selected = self.writeup_toc_state.selected().unwrap_or(0);
        self.writeup_toc_state
            .select(Some((selected + 1) % headings.len()));
    }

    fn previous_writeup_heading(&mut self) {
        let headings = self.current_writeup_headings();
        if headings.is_empty() {
            self.writeup_toc_state.select(None);
            return;
        }
        let selected = self.writeup_toc_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| headings.len().saturating_sub(1));
        self.writeup_toc_state.select(Some(previous));
    }

    fn jump_to_selected_writeup_heading(&mut self) {
        let headings = self.current_writeup_headings();
        let Some(heading) = self
            .writeup_toc_state
            .selected()
            .and_then(|selected| headings.get(selected))
        else {
            self.status = Some("No heading selected.".to_string());
            return;
        };
        self.writeup_detail_scroll = heading.line.min(usize::from(u16::MAX)) as u16;
        self.writeup_detail_focus = WriteupPaneFocus::Detail;
    }

    fn sync_writeup_toc_selection(&mut self, headings: &[MarkdownHeading]) {
        if headings.is_empty() {
            self.writeup_toc_state.select(None);
            return;
        }
        let selected = self
            .writeup_toc_state
            .selected()
            .unwrap_or(0)
            .min(headings.len() - 1);
        self.writeup_toc_state.select(Some(selected));
    }

    fn sync_writeup_toc_to_scroll(&mut self) {
        if !self.writeup_toc_open {
            return;
        }
        let headings = self.current_writeup_headings();
        if headings.is_empty() {
            self.writeup_toc_state.select(None);
            return;
        }
        let scroll = usize::from(self.writeup_detail_scroll);
        let selected = headings
            .iter()
            .enumerate()
            .take_while(|(_, heading)| heading.line <= scroll)
            .map(|(idx, _)| idx)
            .last()
            .unwrap_or(0);
        self.writeup_toc_state.select(Some(selected));
    }

    fn resize_detail(&mut self, delta: i16) {
        if self.detail.is_none() && self.writeup_detail.is_none() {
            self.status = Some("Open details first.".to_string());
            return;
        }
        let next = if delta.is_negative() {
            self.detail_width_percent
                .saturating_sub(delta.unsigned_abs())
        } else {
            self.detail_width_percent.saturating_add(delta as u16)
        };
        let next = next.clamp(DETAIL_WIDTH_PERCENT_MIN, DETAIL_WIDTH_PERCENT_MAX);
        if next != self.detail_width_percent {
            self.detail_width_percent = next;
            if let Err(err) = self.save_project_settings() {
                self.status = Some(format!("Detail pane: {next}%. Settings not saved: {err}"));
                return;
            }
        }
        self.status = Some(format!("Detail pane: {}%.", self.detail_width_percent));
    }

    fn save_project_settings(&self) -> Result<()> {
        let git_dir = self.store.session().repo_git_dir();
        let mut state = State::load().unwrap_or_default();
        let mut settings = state.project_settings_for(&git_dir);
        settings.detail_width_percent = Some(self.detail_width_percent);
        settings.show_subissues = Some(self.show_subissues_preference);
        state.set_project_settings(&git_dir, settings);
        state.save()
    }

    fn refresh_data(&mut self) -> Result<()> {
        let selected_id = self.selected_ticket().map(|ticket| ticket.id);
        let selected_writeup_id = self.selected_writeup().map(|writeup| writeup.id);
        let was_board = self.view == ViewMode::Board && self.detail.is_none();
        self.reload_all(selected_id, selected_writeup_id)?;
        if was_board {
            if let Some(id) = selected_id {
                self.select_board_ticket_by_id(id);
            }
        }
        self.status = Some("Refreshed.".to_string());
        Ok(())
    }

    fn toggle_tab(&mut self) {
        self.active_tab = match self.active_tab {
            TuiTab::Issues => {
                self.comments_mode = false;
                self.view = ViewMode::List;
                TuiTab::Writeups
            }
            TuiTab::Writeups => TuiTab::Issues,
            TuiTab::Dashboard => TuiTab::Issues,
        };
    }

    fn select_board_ticket_by_id(&mut self, id: uuid::Uuid) {
        let Some(ticket_idx) = self.tickets.iter().position(|ticket| ticket.id == id) else {
            return;
        };
        let ticket_state = self.tickets[ticket_idx].state;
        let Some(column) = BOARD_STATES.iter().position(|state| *state == ticket_state) else {
            return;
        };
        let Some(row) = self
            .board_column_tickets(column)
            .iter()
            .position(|idx| **idx == ticket_idx)
        else {
            return;
        };
        self.board_column = column;
        self.board_rows[column] = row;
    }

    fn handle_board_key(&mut self) -> Result<()> {
        if self.detail.is_some() {
            self.open_board_for_detail_ticket();
        } else if self.view == ViewMode::Board {
            self.view = ViewMode::List;
        } else {
            let selected_id = self.selected_ticket().map(|ticket| ticket.id);
            self.view = ViewMode::Board;
            if let Some(id) = selected_id {
                self.select_board_ticket_by_id(id);
            }
        }
        Ok(())
    }

    fn handle_dashboard_key(&mut self) {
        if self.active_tab == TuiTab::Dashboard {
            self.active_tab = TuiTab::Issues;
            self.view = ViewMode::List;
            return;
        }
        self.active_tab = TuiTab::Dashboard;
        self.detail = None;
        self.writeup_detail = None;
        self.comments_mode = false;
    }

    fn toggle_subissue_visibility(&mut self) -> Result<()> {
        let selected_id = self.selected_ticket().map(|ticket| ticket.id);
        self.show_subissues_preference = self.hide_subissues;
        self.hide_subissues = !self.show_subissues_preference;
        self.save_project_settings()?;
        self.reload(selected_id)?;
        self.status = Some(if self.hide_subissues {
            "Hiding subissues.".to_string()
        } else {
            "Showing subissues.".to_string()
        });
        Ok(())
    }

    fn open_board_for_detail_ticket(&mut self) {
        let Some(idx) = self.detail else {
            return;
        };
        let ticket_state = self.tickets[idx].state;
        let Some(column) = BOARD_STATES.iter().position(|state| *state == ticket_state) else {
            self.status = Some("Selected ticket is not on the board.".to_string());
            return;
        };
        let Some(row) = self
            .board_column_tickets(column)
            .iter()
            .position(|ticket_idx| **ticket_idx == idx)
        else {
            self.status =
                Some("Selected ticket is hidden by the current board filters.".to_string());
            return;
        };

        self.view = ViewMode::Board;
        self.detail = None;
        self.comments_mode = false;
        self.board_column = column;
        self.board_rows[column] = row;
        self.sync_board_to_list_selection();
    }

    fn open_selected(&mut self) {
        if self.active_tab == TuiTab::Writeups {
            self.open_selected_writeup();
            return;
        }
        if let Some(idx) = self.selected_ticket_index() {
            self.detail = Some(idx);
            self.select_list_ticket_by_index(idx);
            self.comments_mode = false;
            self.sync_comment_selection();
        }
    }

    fn open_selected_writeup(&mut self) {
        if let Some(idx) = self.selected_writeup_index() {
            if self.writeup_detail != Some(idx) {
                self.writeup_detail_scroll = 0;
                self.writeup_toc_state.select(None);
            }
            self.writeup_detail = Some(idx);
            if self.writeup_detail_focus == WriteupPaneFocus::Toc && !self.writeup_toc_open {
                self.writeup_detail_focus = WriteupPaneFocus::Detail;
            }
            if let Some(visible_pos) = self
                .visible_writeups
                .iter()
                .position(|visible| *visible == idx)
            {
                self.writeup_state.select(Some(visible_pos));
            }
        }
    }

    fn open_ticket_by_id(&mut self, id: uuid::Uuid) {
        let list_indices = self.list_ticket_indices();
        if let Some(list_pos) = list_indices
            .iter()
            .position(|idx| self.tickets[*idx].id == id)
        {
            self.list_state.select(Some(list_pos));
            self.detail = list_indices.get(list_pos).copied();
            self.comments_mode = false;
            self.sync_comment_selection();
        }
    }

    fn sync_open_detail(&mut self) {
        if self.detail.is_some() {
            self.open_selected();
        }
    }

    fn sync_open_writeup_detail(&mut self) {
        if self.writeup_detail.is_some() {
            self.open_selected_writeup();
        }
    }

    fn board_column_tickets(&self, column: usize) -> Vec<&usize> {
        let state = BOARD_STATES[column];
        self.visible
            .iter()
            .filter(|idx| self.tickets[**idx].state == state)
            .collect()
    }

    fn next_board_column(&mut self) {
        self.board_column = (self.board_column + 1) % BOARD_STATES.len();
        self.sync_board_to_list_selection();
    }

    fn previous_board_column(&mut self) {
        self.board_column = self
            .board_column
            .checked_sub(1)
            .unwrap_or_else(|| BOARD_STATES.len() - 1);
        self.sync_board_to_list_selection();
    }

    fn next_board_ticket(&mut self) {
        let len = self.board_column_tickets(self.board_column).len();
        if len == 0 {
            return;
        }
        self.board_rows[self.board_column] = (self.board_rows[self.board_column] + 1) % len;
        self.sync_board_to_list_selection();
    }

    fn previous_board_ticket(&mut self) {
        let len = self.board_column_tickets(self.board_column).len();
        if len == 0 {
            return;
        }
        self.board_rows[self.board_column] = self.board_rows[self.board_column]
            .checked_sub(1)
            .unwrap_or_else(|| len.saturating_sub(1));
        self.sync_board_to_list_selection();
    }

    fn sync_board_selection(&mut self) {
        for column in 0..BOARD_STATES.len() {
            let len = self.board_column_tickets(column).len();
            if len == 0 {
                self.board_rows[column] = 0;
            } else {
                self.board_rows[column] = self.board_rows[column].min(len - 1);
            }
        }
    }

    fn sync_board_to_list_selection(&mut self) {
        if let Some(idx) = self.selected_ticket_index() {
            self.select_list_ticket_by_index(idx);
        }
    }

    fn select_list_ticket_by_index(&mut self, idx: usize) {
        if let Some(list_pos) = self
            .list_ticket_indices()
            .iter()
            .position(|candidate| *candidate == idx)
        {
            self.list_state.select(Some(list_pos));
        }
    }

    fn enter_comments_mode(&mut self) {
        if self.detail.is_none() {
            self.status = Some("Open ticket details first.".to_string());
            return;
        }
        self.comments_mode = true;
        self.sync_comment_selection();
    }

    fn next_comment(&mut self) {
        let Some(ticket) = self.detail.map(|idx| &self.tickets[idx]) else {
            return;
        };
        if ticket.comments.is_empty() {
            return;
        }
        let selected = self.comment_state.selected().unwrap_or(0);
        self.comment_state
            .select(Some((selected + 1) % ticket.comments.len()));
    }

    fn previous_comment(&mut self) {
        let Some(ticket) = self.detail.map(|idx| &self.tickets[idx]) else {
            return;
        };
        if ticket.comments.is_empty() {
            return;
        }
        let selected = self.comment_state.selected().unwrap_or(0);
        let previous = selected
            .checked_sub(1)
            .unwrap_or_else(|| ticket.comments.len().saturating_sub(1));
        self.comment_state.select(Some(previous));
    }

    fn sync_comment_selection(&mut self) {
        let Some(ticket) = self.detail.map(|idx| &self.tickets[idx]) else {
            self.comments_mode = false;
            self.comment_state.select(None);
            return;
        };
        if ticket.comments.is_empty() {
            self.comment_state.select(None);
            return;
        }
        let selected = self
            .comment_state
            .selected()
            .unwrap_or(0)
            .min(ticket.comments.len() - 1);
        self.comment_state.select(Some(selected));
    }

    fn selected_comment<'a>(&self, ticket: &'a Ticket) -> Option<&'a Comment> {
        self.comment_state
            .selected()
            .and_then(|idx| ticket.comments.get(idx))
    }

    fn selected_ticket(&self) -> Option<&Ticket> {
        self.selected_ticket_index().map(|idx| &self.tickets[idx])
    }

    fn selected_tag_target(&self) -> Option<(TagTarget, String, BTreeSet<String>)> {
        match self.active_tab {
            TuiTab::Issues => {
                let ticket = self.selected_ticket()?;
                Some((
                    TagTarget::Ticket(ticket.id),
                    ticket.short_id(),
                    ticket.tags.clone(),
                ))
            }
            TuiTab::Writeups => {
                let writeup = self.selected_writeup()?;
                Some((
                    TagTarget::Writeup(writeup.id),
                    format!("writeup {}", writeup.short_id()),
                    writeup.tags.clone(),
                ))
            }
            TuiTab::Dashboard => None,
        }
    }

    fn selected_ticket_index(&self) -> Option<usize> {
        if self.active_tab != TuiTab::Issues {
            return None;
        }
        if self.view == ViewMode::Board && self.detail.is_none() {
            let tickets = self.board_column_tickets(self.board_column);
            return tickets
                .get(self.board_rows[self.board_column])
                .map(|idx| **idx);
        }
        self.list_state
            .selected()
            .and_then(|selected| self.list_ticket_indices().get(selected).copied())
    }

    fn selected_writeup(&self) -> Option<&Writeup> {
        self.selected_writeup_index().map(|idx| &self.writeups[idx])
    }

    fn selected_writeup_index(&self) -> Option<usize> {
        if self.active_tab != TuiTab::Writeups {
            return None;
        }
        self.writeup_state
            .selected()
            .and_then(|selected| self.visible_writeups.get(selected))
            .copied()
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

impl OrderChoice {
    fn label(self) -> &'static str {
        match self {
            OrderChoice::Priority => "priority",
            OrderChoice::DateAsc => "date asc",
            OrderChoice::DateDesc => "date desc",
            OrderChoice::State => "state",
        }
    }

    fn spec(self) -> &'static str {
        match self {
            OrderChoice::Priority => "priority",
            OrderChoice::DateAsc => "created",
            OrderChoice::DateDesc => "created.desc",
            OrderChoice::State => "state",
        }
    }

    fn sort_order(self) -> Option<SortOrder> {
        Some(match self {
            OrderChoice::Priority => SortOrder {
                key: SortKey::Priority,
                desc: false,
            },
            OrderChoice::DateAsc => SortOrder {
                key: SortKey::Created,
                desc: false,
            },
            OrderChoice::DateDesc => SortOrder {
                key: SortKey::Created,
                desc: true,
            },
            OrderChoice::State => SortOrder {
                key: SortKey::State,
                desc: false,
            },
        })
    }
}

fn order_choice_index(choice: OrderChoice) -> usize {
    ORDER_CHOICES
        .iter()
        .position(|candidate| *candidate == choice)
        .unwrap_or(0)
}

impl InputKind {
    fn label(self) -> &'static str {
        match self {
            InputKind::Priority => "priority",
            InputKind::Points => "points",
            InputKind::AddTags => "add tags",
            InputKind::RemoveTags => "remove tags",
        }
    }

    fn modal_height(self) -> u16 {
        match self {
            InputKind::Priority
            | InputKind::Points
            | InputKind::AddTags
            | InputKind::RemoveTags => 9,
        }
    }
}

fn menu_bar_line(
    width: usize,
    mode: &str,
    detail: Option<&str>,
    hints: &[MenuHint],
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            " ti tui ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{mode} "),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    let help = MenuHint {
        key: "?",
        desc: "help",
    };
    let help_width = menu_hint_width(help);
    let separator_width = 2;

    if width <= help_width {
        return Line::from(menu_hint_spans(help));
    }

    let mut used = spans_width(&spans);
    let reserve_for_help = separator_width + help_width;

    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        if let Some(available) = width
            .checked_sub(used)
            .and_then(|available| available.checked_sub(separator_width + reserve_for_help))
        {
            let detail_width = available.min(36);
            if detail_width >= 4 {
                append_menu_separator(&mut spans);
                let value = truncate_display(detail, detail_width);
                used += separator_width + UnicodeWidthStr::width(value.as_str());
                spans.push(Span::styled(value, Style::default().fg(Color::Cyan)));
            }
        }
    }

    for hint in hints {
        let hint_width = menu_hint_width(*hint);
        if used + separator_width + hint_width + reserve_for_help > width {
            continue;
        }
        append_menu_separator(&mut spans);
        spans.extend(menu_hint_spans(*hint));
        used += separator_width + hint_width;
    }

    if used + reserve_for_help <= width {
        append_menu_separator(&mut spans);
        spans.extend(menu_hint_spans(help));
    } else {
        spans = menu_hint_spans(help);
    }

    Line::from(spans)
}

fn menu_hint_width(hint: MenuHint) -> usize {
    UnicodeWidthStr::width(hint.key) + 1 + UnicodeWidthStr::width(hint.desc)
}

fn menu_hint_spans(hint: MenuHint) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            hint.key,
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(hint.desc, Style::default().fg(Color::Gray)),
    ]
}

fn append_menu_separator(spans: &mut Vec<Span<'static>>) {
    spans.push(Span::raw("  "));
}

fn parse_optional_i64(raw: &str, label: &str) -> Result<Option<i64>> {
    let Some(value) = optional_trimmed(raw) else {
        return Ok(None);
    };
    value
        .parse::<i64>()
        .map(Some)
        .map_err(|_| anyhow::anyhow!("{label} must be an integer"))
}

fn compare_tui_tickets(a: &Ticket, b: &Ticket) -> std::cmp::Ordering {
    priority_sort_key(a.priority)
        .cmp(&priority_sort_key(b.priority))
        .then_with(|| b.created_at.cmp(&a.created_at))
        .then_with(|| a.id.cmp(&b.id))
}

fn compare_tui_writeups(a: &Writeup, b: &Writeup) -> std::cmp::Ordering {
    priority_sort_key(a.priority)
        .cmp(&priority_sort_key(b.priority))
        .then_with(|| writeup_recent_at(b).cmp(&writeup_recent_at(a)))
        .then_with(|| a.id.cmp(&b.id))
}

fn closed_at_for(
    closed_at: &HashMap<uuid::Uuid, OffsetDateTime>,
    ticket: &Ticket,
) -> OffsetDateTime {
    closed_at
        .get(&ticket.id)
        .copied()
        .unwrap_or(ticket.created_at)
}

fn priority_sort_key(priority: Option<i64>) -> (u8, i64) {
    match priority {
        Some(value) => (0, value),
        None => (1, 0),
    }
}

fn run_ti_sync_command() -> Result<SyncResult> {
    let exe = std::env::current_exe().context("locating current ti executable")?;
    let output = Command::new(exe)
        .arg("sync")
        .output()
        .context("running ti sync")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        let message = first_non_empty_line(&stderr)
            .or_else(|| first_non_empty_line(&stdout))
            .unwrap_or("ti sync failed");
        anyhow::bail!("{message}");
    }

    Ok(SyncResult {
        summary: sync_summary(&stdout),
    })
}

fn sync_summary(stdout: &str) -> String {
    let pull = stdout
        .lines()
        .find(|line| line.starts_with("Pull:"))
        .unwrap_or("Pull complete.");
    let push = stdout
        .lines()
        .find(|line| line.starts_with("Push:"))
        .unwrap_or("Push complete.");
    format!("{pull} {push}")
}

fn first_non_empty_line(value: &str) -> Option<&str> {
    value.lines().map(str::trim).find(|line| !line.is_empty())
}

fn tabs_title(active: TuiTab, title: &str) -> String {
    let issues = if active == TuiTab::Issues {
        "[issues]"
    } else {
        " issues "
    };
    let writeups = if active == TuiTab::Writeups {
        "[writeups]"
    } else {
        " writeups "
    };
    format!("{issues} {writeups}  {title}")
}

fn ticket_list_line(
    ticket: &Ticket,
    width: usize,
    compact: bool,
    current_user: &str,
    has_writeups: bool,
) -> Line<'static> {
    let short_id = ticket
        .short_id()
        .chars()
        .take(LIST_ID_WIDTH)
        .collect::<String>();
    let title = flatten_display(&ticket.title);
    if compact {
        return compact_ticket_list_line(
            &short_id,
            &title,
            &list_meta_display(ticket),
            ticket.assigned.as_deref() == Some(current_user),
            width,
            has_writeups.then(|| ("[w]".to_string(), Style::default().fg(Color::Yellow))),
        );
    }

    ticket_list_line_from_parts(
        Some(&short_id),
        &title,
        &list_meta_display(ticket),
        Some(&ticket.tags),
        ticket.assigned.as_deref() == Some(current_user),
        width,
        has_writeups.then(|| ("[w]".to_string(), Style::default().fg(Color::Yellow))),
    )
}

fn issue_title_prefix(
    ticket: &Ticket,
    ticket_by_id: &HashMap<uuid::Uuid, &Ticket>,
    show_subissue_graph: bool,
) -> String {
    let mut prefix = if show_subissue_graph {
        subissue_graph_prefix(ticket, ticket_by_id)
    } else {
        String::new()
    };
    if !ticket.children.is_empty() {
        prefix.push_str("[+] ");
    }
    prefix
}

fn subissue_graph_prefix(ticket: &Ticket, ticket_by_id: &HashMap<uuid::Uuid, &Ticket>) -> String {
    let depth = subissue_depth(ticket, ticket_by_id).min(5);
    if depth == 0 {
        return String::new();
    }

    format!("{}╰┄ ", " ".repeat(depth))
}

fn subissue_depth(ticket: &Ticket, ticket_by_id: &HashMap<uuid::Uuid, &Ticket>) -> usize {
    let mut depth = 0;
    let mut current = ticket;
    let mut seen = BTreeSet::new();
    while let Some(parent) = current.parent {
        if !seen.insert(parent) {
            break;
        }
        let Some(parent_ticket) = ticket_by_id.get(&parent).copied() else {
            break;
        };
        depth += 1;
        current = parent_ticket;
    }
    depth
}

fn ordered_list_indices(tickets: &[Ticket], visible: &[usize], show_subissues: bool) -> Vec<usize> {
    if !show_subissues {
        return visible.to_vec();
    }
    build_outline_rows(tickets, visible, &BTreeSet::new())
        .into_iter()
        .map(|row| row.ticket_idx)
        .collect()
}

fn build_outline_rows(
    tickets: &[Ticket],
    visible: &[usize],
    collapsed: &BTreeSet<uuid::Uuid>,
) -> Vec<OutlineRow> {
    let visible_set = visible.iter().copied().collect::<BTreeSet<_>>();
    let visible_order = visible
        .iter()
        .enumerate()
        .map(|(order, idx)| (*idx, order))
        .collect::<HashMap<_, _>>();
    let index_by_id = tickets
        .iter()
        .enumerate()
        .map(|(idx, ticket)| (ticket.id, idx))
        .collect::<HashMap<_, _>>();

    let mut roots = Vec::new();
    let mut children_by_parent = HashMap::<uuid::Uuid, Vec<usize>>::new();
    for &idx in visible {
        let Some(ticket) = tickets.get(idx) else {
            continue;
        };
        if let Some(parent) = ticket.parent {
            if index_by_id
                .get(&parent)
                .is_some_and(|parent_idx| visible_set.contains(parent_idx))
            {
                children_by_parent.entry(parent).or_default().push(idx);
                continue;
            }
        }
        roots.push(idx);
    }

    roots.sort_by_key(|idx| visible_order.get(idx).copied().unwrap_or(usize::MAX));
    for children in children_by_parent.values_mut() {
        children.sort_by_key(|idx| visible_order.get(idx).copied().unwrap_or(usize::MAX));
    }

    let mut rows = Vec::new();
    let mut visited = BTreeSet::<uuid::Uuid>::new();
    for idx in roots {
        push_outline_row(
            tickets,
            idx,
            0,
            collapsed,
            &children_by_parent,
            &mut visited,
            &mut rows,
        );
    }
    for &idx in visible {
        let Some(ticket) = tickets.get(idx) else {
            continue;
        };
        if visited.contains(&ticket.id) {
            continue;
        }
        push_outline_row(
            tickets,
            idx,
            0,
            collapsed,
            &children_by_parent,
            &mut visited,
            &mut rows,
        );
    }

    rows
}

fn push_outline_row(
    tickets: &[Ticket],
    idx: usize,
    depth: usize,
    collapsed: &BTreeSet<uuid::Uuid>,
    children_by_parent: &HashMap<uuid::Uuid, Vec<usize>>,
    visited: &mut BTreeSet<uuid::Uuid>,
    rows: &mut Vec<OutlineRow>,
) {
    let Some(ticket) = tickets.get(idx) else {
        return;
    };
    if !visited.insert(ticket.id) {
        return;
    }

    let children = children_by_parent.get(&ticket.id);
    let has_children = children.is_some_and(|children| !children.is_empty());
    let is_collapsed = collapsed.contains(&ticket.id);
    rows.push(OutlineRow {
        ticket_idx: idx,
        depth,
        has_children,
        collapsed: is_collapsed,
    });

    if is_collapsed {
        if let Some(children) = children {
            mark_outline_descendants_visited(tickets, children, children_by_parent, visited);
        }
        return;
    }
    if let Some(children) = children {
        for &child in children {
            push_outline_row(
                tickets,
                child,
                depth + 1,
                collapsed,
                children_by_parent,
                visited,
                rows,
            );
        }
    }
}

fn mark_outline_descendants_visited(
    tickets: &[Ticket],
    children: &[usize],
    children_by_parent: &HashMap<uuid::Uuid, Vec<usize>>,
    visited: &mut BTreeSet<uuid::Uuid>,
) {
    for &child in children {
        let Some(ticket) = tickets.get(child) else {
            continue;
        };
        if !visited.insert(ticket.id) {
            continue;
        }
        if let Some(grandchildren) = children_by_parent.get(&ticket.id) {
            mark_outline_descendants_visited(tickets, grandchildren, children_by_parent, visited);
        }
    }
}

fn issue_columns_for_width(columns: &[IssueColumn], width: usize) -> Vec<IssueColumn> {
    let mut columns = columns.to_vec();
    if !columns.contains(&IssueColumn::Title) {
        columns.push(IssueColumn::Title);
    }

    for removable in [
        IssueColumn::Tags,
        IssueColumn::Milestone,
        IssueColumn::Points,
        IssueColumn::Assignee,
        IssueColumn::Priority,
        IssueColumn::State,
        IssueColumn::Date,
        IssueColumn::Closed,
        IssueColumn::Id,
    ] {
        if issue_columns_min_width_with_title(&columns, ISSUE_TABLE_MIN_TITLE_WIDTH) <= width {
            break;
        }
        if let Some(idx) = columns.iter().position(|column| *column == removable) {
            columns.remove(idx);
        }
    }

    columns
}

fn issue_columns_min_width_with_title(columns: &[IssueColumn], title_width: usize) -> usize {
    let fixed = columns
        .iter()
        .map(|column| column.fixed_width().unwrap_or(title_width))
        .sum::<usize>();
    fixed + columns.len().saturating_sub(1)
}

fn issue_column_widths(columns: &[IssueColumn], width: usize) -> Vec<usize> {
    let fixed = columns
        .iter()
        .filter_map(|column| column.fixed_width())
        .sum::<usize>();
    let gaps = columns.len().saturating_sub(1);
    let title_width = width.saturating_sub(fixed + gaps).max(1);
    columns
        .iter()
        .map(|column| column.fixed_width().unwrap_or(title_width))
        .collect()
}

fn issue_table_header(columns: &[IssueColumn], widths: &[usize], width: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(
        " ".repeat(UnicodeWidthStr::width(HIGHLIGHT_SYMBOL)),
    )];
    for (idx, (column, column_width)) in columns.iter().zip(widths).enumerate() {
        if idx > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            fit_display(column.label(), *column_width),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let used = spans_width(&spans);
    if used < width {
        spans.push(Span::raw(" ".repeat(width - used)));
    }
    Line::from(spans)
}

fn ticket_table_line(
    ticket: &Ticket,
    columns: &[IssueColumn],
    widths: &[usize],
    width: usize,
    title_prefix: &str,
    _compact: bool,
    current_user: &str,
    closed_at: Option<OffsetDateTime>,
    has_writeups: bool,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (idx, (column, column_width)) in columns.iter().zip(widths).enumerate() {
        if idx > 0 {
            spans.push(Span::raw(" "));
        }
        match column {
            IssueColumn::Id => push_issue_id_column(
                &mut spans,
                ticket,
                *column_width,
                ticket.assigned.as_deref() == Some(current_user),
            ),
            IssueColumn::Tags => push_issue_tags_column(&mut spans, &ticket.tags, *column_width),
            IssueColumn::Title => {
                push_issue_title_column(&mut spans, ticket, *column_width, title_prefix);
            }
            _ => {
                let (value, style) = issue_column_value(ticket, *column, closed_at, current_user);
                spans.push(Span::styled(fit_display(&value, *column_width), style));
            }
        }
    }

    if has_writeups {
        push_right_indicator(
            &mut spans,
            Some(("[w]".to_string(), Style::default().fg(Color::Yellow))),
            width,
        );
    }

    Line::from(spans)
}

fn push_issue_id_column(
    spans: &mut Vec<Span<'static>>,
    ticket: &Ticket,
    width: usize,
    assigned_to_current_user: bool,
) {
    let short_id = ticket
        .short_id()
        .chars()
        .take(LIST_ID_WIDTH)
        .collect::<String>();
    let id = truncate_display(&short_id, width);
    let id_width = UnicodeWidthStr::width(id.as_str());
    let star_width = width.saturating_sub(id_width).min(1);
    let star = if assigned_to_current_user { "*" } else { " " };
    let padding_width = width.saturating_sub(id_width + star_width);

    spans.push(Span::styled(id, Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled(
        truncate_display(star, star_width),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ));
    if padding_width > 0 {
        spans.push(Span::raw(" ".repeat(padding_width)));
    }
}

fn push_issue_title_column(
    spans: &mut Vec<Span<'static>>,
    ticket: &Ticket,
    width: usize,
    title_prefix: &str,
) {
    if title_prefix.is_empty() {
        let (value, style) = issue_column_value(ticket, IssueColumn::Title, None, "");
        spans.push(Span::styled(fit_display(&value, width), style));
        return;
    }

    let prefix = truncate_display(title_prefix, width);
    let prefix_width = UnicodeWidthStr::width(prefix.as_str());
    spans.push(Span::styled(prefix, Style::default().fg(Color::DarkGray)));
    let title = flatten_display(&ticket.title);
    spans.push(Span::styled(
        fit_display(&title, width.saturating_sub(prefix_width)),
        Style::default().fg(Color::Reset),
    ));
}

fn push_issue_tags_column(spans: &mut Vec<Span<'static>>, tags: &BTreeSet<String>, width: usize) {
    let tag_spans = tag_spans(tags, width);
    let tag_width = spans_width(&tag_spans);
    spans.extend(tag_spans);
    if tag_width < width {
        spans.push(Span::raw(" ".repeat(width - tag_width)));
    }
}

fn issue_column_value(
    ticket: &Ticket,
    column: IssueColumn,
    closed_at: Option<OffsetDateTime>,
    current_user: &str,
) -> (String, Style) {
    match column {
        IssueColumn::Id => (
            ticket.short_id().chars().take(LIST_ID_WIDTH).collect(),
            Style::default().fg(Color::DarkGray),
        ),
        IssueColumn::Date => (
            relative_time(ticket.created_at, OffsetDateTime::now_utc()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        IssueColumn::Closed => (
            closed_at
                .map(|at| relative_time(at, OffsetDateTime::now_utc()))
                .unwrap_or_default(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        IssueColumn::Priority => (
            ticket
                .priority
                .map(|priority| format!("p{priority}"))
                .unwrap_or_default(),
            Style::default().fg(Color::Magenta),
        ),
        IssueColumn::State => (
            state_abbrev(ticket.state).to_string(),
            state_abbrev_style(ticket.state),
        ),
        IssueColumn::Title => (
            flatten_display(&ticket.title),
            Style::default().fg(Color::Reset),
        ),
        IssueColumn::Assignee => (
            ticket
                .assigned
                .as_deref()
                .map(short_assignee)
                .unwrap_or_default(),
            if ticket.assigned.as_deref() == Some(current_user) {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            },
        ),
        IssueColumn::Points => (
            ticket
                .points
                .map(|points| points.to_string())
                .unwrap_or_default(),
            Style::default().fg(Color::LightBlue),
        ),
        IssueColumn::Milestone => (
            ticket.milestone.clone().unwrap_or_default(),
            Style::default().fg(Color::LightCyan),
        ),
        IssueColumn::Tags => (
            ticket.tags.iter().cloned().collect::<Vec<_>>().join(","),
            Style::default().fg(Color::Yellow),
        ),
    }
}

fn short_assignee(assignee: &str) -> String {
    assignee
        .split_once('@')
        .map(|(local, _)| local)
        .unwrap_or(assignee)
        .to_string()
}

fn writeup_list_line(writeup: &Writeup, width: usize, compact: bool) -> Line<'static> {
    let short_id = writeup
        .short_id()
        .chars()
        .take(LIST_ID_WIDTH)
        .collect::<String>();
    let title = flatten_display(&writeup.title);
    let meta = vec![
        (
            fit_display(
                &relative_time(writeup_recent_at(writeup), OffsetDateTime::now_utc()),
                LIST_AGE_WIDTH,
            ),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        (
            fit_display(
                &writeup
                    .priority
                    .map(|priority| format!("p{priority}"))
                    .unwrap_or_else(|| "-".to_string()),
                LIST_PRIORITY_WIDTH,
            ),
            Style::default().fg(Color::LightMagenta),
        ),
        (
            fit_display(&format!("v{}", writeup.versions.len()), LIST_STATE_WIDTH),
            Style::default().fg(Color::LightBlue),
        ),
        (
            fit_display(writeup_status_abbrev(writeup.status), LIST_STATE_WIDTH),
            writeup_status_style(writeup.status),
        ),
    ];
    let issue_indicator = (!writeup.tickets.is_empty()).then(|| {
        (
            format!("[{}]", writeup.tickets.len()),
            Style::default().fg(Color::Magenta),
        )
    });

    if compact {
        return ticket_list_line_from_parts(
            Some(&short_id),
            &title,
            &meta,
            None,
            false,
            width,
            issue_indicator,
        );
    }

    ticket_list_line_from_parts(
        Some(&short_id),
        &title,
        &meta,
        Some(&writeup.tags),
        false,
        width,
        issue_indicator,
    )
}

struct DashboardStats {
    total: usize,
    open: usize,
    closed: usize,
    with_comments: usize,
    created_7d: usize,
    states: Vec<(String, usize)>,
    tags: Vec<(String, usize)>,
    assignees: Vec<(String, usize)>,
    recently_opened: Vec<(uuid::Uuid, String)>,
    closed_tickets: Vec<(uuid::Uuid, OffsetDateTime, String)>,
}

impl DashboardStats {
    fn from_tickets(tickets: &[Ticket]) -> Self {
        let now = OffsetDateTime::now_utc();
        let week_ago = now - time::Duration::days(7);
        let mut open = 0;
        let mut closed = 0;
        let mut with_comments = 0;
        let mut created_7d = 0;
        let mut states = BTreeMap::<String, usize>::new();
        let mut tags = BTreeMap::<String, usize>::new();
        let mut assignees = BTreeMap::<String, usize>::new();
        let mut recently_opened = Vec::new();
        let mut closed_tickets = Vec::new();

        for ticket in tickets {
            match ticket.status {
                TicketStatus::Open => open += 1,
                TicketStatus::Closed => {
                    closed += 1;
                    closed_tickets.push((ticket.id, ticket.created_at, ticket.title.clone()));
                }
            }
            if !ticket.comments.is_empty() {
                with_comments += 1;
            }
            if ticket.created_at >= week_ago {
                created_7d += 1;
            }
            *states.entry(ticket.state.as_str().to_string()).or_default() += 1;
            for tag in &ticket.tags {
                *tags.entry(tag.clone()).or_default() += 1;
            }
            if let Some(assigned) = &ticket.assigned {
                *assignees.entry(short_assignee(assigned)).or_default() += 1;
            }
            recently_opened.push((ticket.id, ticket.title.clone()));
        }

        let sort_counts = |map: BTreeMap<String, usize>| {
            let mut values = map.into_iter().collect::<Vec<_>>();
            values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            values
        };
        recently_opened.sort_by(|a, b| {
            ticket_created_at(tickets, b.0)
                .cmp(&ticket_created_at(tickets, a.0))
                .then_with(|| a.0.cmp(&b.0))
        });

        Self {
            total: tickets.len(),
            open,
            closed,
            with_comments,
            created_7d,
            states: sort_counts(states),
            tags: sort_counts(tags),
            assignees: sort_counts(assignees),
            recently_opened,
            closed_tickets,
        }
    }
}

fn ticket_created_at(tickets: &[Ticket], id: uuid::Uuid) -> OffsetDateTime {
    tickets
        .iter()
        .find(|ticket| ticket.id == id)
        .map(|ticket| ticket.created_at)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn dashboard_lines(
    stats: &DashboardStats,
    closed_at: &HashMap<uuid::Uuid, OffsetDateTime>,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    if stats.total == 0 {
        return vec![Line::from(Span::styled(
            "No tickets.",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let now = OffsetDateTime::now_utc();
    let week_ago = now - time::Duration::days(7);
    let mut recently_closed = stats
        .closed_tickets
        .iter()
        .map(|(id, fallback, title)| {
            let closed = closed_at.get(id).copied().unwrap_or(*fallback);
            (*id, closed, title.clone())
        })
        .collect::<Vec<_>>();
    recently_closed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let closed_7d = recently_closed
        .iter()
        .filter(|(_, closed, _)| *closed >= week_ago)
        .count();

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            stats.total.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" tickets  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            stats.open.to_string(),
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" open  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            stats.closed.to_string(),
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" closed", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("  {} with comments", stats.with_comments),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            format!("+{}", stats.created_7d),
            Style::default().fg(Color::LightGreen),
        ),
        Span::styled(" opened  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("-{closed_7d}"),
            Style::default().fg(Color::LightRed),
        ),
        Span::styled(
            " closed in the last 7d",
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::raw(""));

    push_count_section(
        &mut lines,
        "States",
        &stats.states,
        Color::Cyan,
        width,
        height,
    );
    push_count_section(
        &mut lines,
        "Top Tags",
        &stats.tags,
        Color::Magenta,
        width,
        height,
    );
    push_count_section(
        &mut lines,
        "Assignees",
        &stats.assignees,
        Color::LightGreen,
        width,
        height,
    );
    push_ticket_section(
        &mut lines,
        "Recently Opened",
        &stats
            .recently_opened
            .iter()
            .map(|(id, title)| (*id, title.clone()))
            .collect::<Vec<_>>(),
        Color::LightGreen,
        width,
        height,
    );
    push_ticket_section(
        &mut lines,
        "Recently Closed",
        &recently_closed
            .iter()
            .map(|(id, _, title)| (*id, title.clone()))
            .collect::<Vec<_>>(),
        Color::LightRed,
        width,
        height,
    );

    lines.truncate(height.max(1));
    lines
}

fn push_count_section(
    lines: &mut Vec<Line<'static>>,
    title: &'static str,
    rows: &[(String, usize)],
    color: Color,
    width: usize,
    height: usize,
) {
    if rows.is_empty() || lines.len() + 3 > height {
        return;
    }
    let remaining = height.saturating_sub(lines.len() + 2);
    if remaining == 0 {
        return;
    }
    let limit = rows.len().min(remaining.min(5));
    lines.push(Line::from(Span::styled(
        title,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
    let max = rows.iter().map(|(_, count)| *count).max().unwrap_or(1);
    let bar_width = width.saturating_sub(22).min(28);
    for (label, count) in rows.iter().take(limit) {
        lines.push(count_line(label, *count, max, color, bar_width));
    }
    if rows.len() > limit && lines.len() < height {
        lines.push(Line::from(Span::styled(
            format!("... and {} more", rows.len() - limit),
            Style::default().fg(Color::DarkGray),
        )));
    }
    if lines.len() < height {
        lines.push(Line::raw(""));
    }
}

fn count_line(
    label: &str,
    count: usize,
    max: usize,
    color: Color,
    bar_width: usize,
) -> Line<'static> {
    let filled = if max == 0 || bar_width == 0 {
        0
    } else {
        ((count as f64 / max as f64) * bar_width as f64)
            .round()
            .max(1.0) as usize
    }
    .min(bar_width);
    Line::from(vec![
        Span::styled(fit_display(label, 14), Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(format!("{count:>3}"), Style::default().fg(Color::Gray)),
        Span::raw(" "),
        Span::styled(" ".repeat(filled), Style::default().bg(color)),
    ])
}

fn push_ticket_section(
    lines: &mut Vec<Line<'static>>,
    title: &'static str,
    rows: &[(uuid::Uuid, String)],
    color: Color,
    width: usize,
    height: usize,
) {
    if rows.is_empty() || lines.len() + 3 > height {
        return;
    }
    let remaining = height.saturating_sub(lines.len() + 2);
    if remaining == 0 {
        return;
    }
    let limit = rows.len().min(remaining.min(5));
    lines.push(Line::from(Span::styled(
        title,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
    let title_width = width.saturating_sub(10);
    for (id, title) in rows.iter().take(limit) {
        let short = id.to_string().chars().take(6).collect::<String>();
        lines.push(Line::from(vec![
            Span::styled(short, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::raw(truncate_display(&flatten_display(title), title_width)),
        ]));
    }
    if rows.len() > limit && lines.len() < height {
        lines.push(Line::from(Span::styled(
            format!("... and {} more", rows.len() - limit),
            Style::default().fg(Color::DarkGray),
        )));
    }
    if lines.len() < height {
        lines.push(Line::raw(""));
    }
}

fn writeup_recent_at(writeup: &Writeup) -> OffsetDateTime {
    writeup
        .versions
        .last()
        .map(|version| version.at)
        .unwrap_or(writeup.created_at)
}

fn writeup_status_abbrev(status: WriteupStatus) -> &'static str {
    match status {
        WriteupStatus::Open => "op",
        WriteupStatus::Closed => "cl",
    }
}

fn writeup_status_style(status: WriteupStatus) -> Style {
    match status {
        WriteupStatus::Open => Style::default().fg(Color::LightGreen),
        WriteupStatus::Closed => Style::default().fg(Color::DarkGray),
    }
    .add_modifier(Modifier::BOLD)
}

fn board_ticket_line(ticket: &Ticket, width: usize) -> Line<'static> {
    let meta = priority_points_display(ticket);
    let meta_width = meta
        .as_deref()
        .map(UnicodeWidthStr::width)
        .unwrap_or_default();
    let gap_width = usize::from(meta_width > 0);
    let title_width = width.saturating_sub(meta_width + gap_width);
    let mut spans = Vec::new();
    if let Some(meta) = meta {
        spans.push(Span::styled(meta, Style::default().fg(Color::Magenta)));
        spans.push(Span::raw(" ".repeat(gap_width)));
    }
    spans.push(Span::raw(truncate_display(
        &flatten_display(&ticket.title),
        title_width,
    )));
    Line::from(spans)
}

fn priority_points_display(ticket: &Ticket) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(priority) = ticket.priority {
        parts.push(format!("p{priority}"));
    }
    if let Some(points) = ticket.points {
        parts.push(points.to_string());
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn ticket_list_line_from_parts(
    short_id: Option<&str>,
    title: &str,
    meta: &[(String, Style)],
    tags: Option<&BTreeSet<String>>,
    assigned_to_current_user: bool,
    width: usize,
    right_indicator: Option<(String, Style)>,
) -> Line<'static> {
    let mut leading = Vec::new();
    let mut used_width = 0;
    if let Some(short_id) = short_id {
        let id = truncate_display(short_id, width);
        let id_width = UnicodeWidthStr::width(id.as_str());
        let star = if assigned_to_current_user { "*" } else { " " };
        let star_width = width
            .saturating_sub(id_width)
            .min(UnicodeWidthStr::width(star));
        let gap_width = width.saturating_sub(id_width + star_width).min(1);
        let gap = " ".repeat(gap_width);
        used_width = id_width + star_width + gap_width;
        leading.extend([
            Span::styled(id, Style::default().fg(Color::DarkGray)),
            Span::styled(
                truncate_display(star, star_width),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(gap),
        ]);
    }
    let meta_width = push_meta_spans(&mut leading, meta, width.saturating_sub(used_width));
    let meta_gap_width = if meta_width > 0 {
        width.saturating_sub(used_width + meta_width).min(1)
    } else {
        0
    };
    let meta_gap = " ".repeat(meta_gap_width);
    let indicator_width = right_indicator
        .as_ref()
        .map(|(label, _)| UnicodeWidthStr::width(label.as_str()).min(width))
        .unwrap_or_default();
    let indicator_gap_width = usize::from(indicator_width > 0);
    let content_width = width
        .saturating_sub(used_width + meta_width + meta_gap_width)
        .saturating_sub(indicator_width + indicator_gap_width);

    if meta_width > 0 {
        leading.push(Span::raw(meta_gap));
    }

    let Some(tags) = tags.filter(|tags| !tags.is_empty()) else {
        leading.push(Span::raw(truncate_display(title, content_width)));
        push_right_indicator(&mut leading, right_indicator, width);
        return Line::from(leading);
    };

    let title_full_width = UnicodeWidthStr::width(title);
    let tag_count_width = tag_count_width(tags.len());
    let (title_budget, tag_budget) = if title_full_width + tag_count_width <= content_width {
        (
            title_full_width,
            content_width.saturating_sub(title_full_width),
        )
    } else if tag_count_width < content_width {
        (content_width - tag_count_width, tag_count_width)
    } else {
        (content_width, 0)
    };
    let tag_spans = tag_spans(tags, tag_budget);
    let tags_width = spans_width(&tag_spans);
    let title = truncate_display(title, title_budget);
    let title_width = UnicodeWidthStr::width(title.as_str());
    let padding_width = content_width.saturating_sub(title_width + tags_width);

    leading.push(Span::raw(title));
    leading.push(Span::raw(" ".repeat(padding_width)));
    leading.extend(tag_spans);
    push_right_indicator(&mut leading, right_indicator, width);
    Line::from(leading)
}

fn push_right_indicator(
    spans: &mut Vec<Span<'static>>,
    right_indicator: Option<(String, Style)>,
    width: usize,
) {
    let Some((label, style)) = right_indicator else {
        return;
    };
    let used_width = spans_width(spans);
    if used_width >= width {
        return;
    }
    let available_width = width - used_width;
    let label = truncate_display(&label, available_width);
    let label_width = UnicodeWidthStr::width(label.as_str());
    spans.push(Span::raw(
        " ".repeat(available_width.saturating_sub(label_width)),
    ));
    spans.push(Span::styled(label, style));
}

fn compact_ticket_list_line(
    short_id: &str,
    title: &str,
    meta: &[(String, Style)],
    assigned_to_current_user: bool,
    width: usize,
    right_indicator: Option<(String, Style)>,
) -> Line<'static> {
    let title_target_width = COMPACT_LIST_MIN_TITLE_WIDTH.min(width).max(1);
    let mut short_id = Some(short_id);
    let mut meta = meta.to_vec();

    while compact_title_width(short_id, &meta, width) < title_target_width {
        if !remove_first_meta_width(&mut meta, LIST_STATE_WIDTH) {
            if !remove_first_meta_width(&mut meta, LIST_AGE_WIDTH) {
                if short_id.take().is_none()
                    && !remove_first_meta_width(&mut meta, LIST_PRIORITY_WIDTH)
                {
                    break;
                }
            }
        }
    }

    ticket_list_line_from_parts(
        short_id,
        title,
        &meta,
        None,
        assigned_to_current_user,
        width,
        right_indicator,
    )
}

fn compact_title_width(short_id: Option<&str>, meta: &[(String, Style)], width: usize) -> usize {
    let id_width = short_id
        .map(|id| UnicodeWidthStr::width(id).min(width))
        .unwrap_or_default();
    let star_width = short_id
        .map(|_| width.saturating_sub(id_width).min(1))
        .unwrap_or_default();
    let id_gap_width = short_id
        .map(|_| width.saturating_sub(id_width + star_width).min(1))
        .unwrap_or_default();
    let meta_width = meta
        .iter()
        .map(|(value, _)| UnicodeWidthStr::width(value.as_str()))
        .sum::<usize>()
        .min(width.saturating_sub(id_width + star_width + id_gap_width));
    let meta_gap_width = usize::from(meta_width > 0)
        .min(width.saturating_sub(id_width + star_width + id_gap_width + meta_width));
    width.saturating_sub(id_width + star_width + id_gap_width + meta_width + meta_gap_width)
}

fn remove_first_meta_width(meta: &mut Vec<(String, Style)>, width: usize) -> bool {
    let Some(idx) = meta
        .iter()
        .position(|(value, _)| UnicodeWidthStr::width(value.as_str()) == width)
    else {
        return false;
    };
    meta.remove(idx);
    true
}

fn list_meta_display(ticket: &Ticket) -> Vec<(String, Style)> {
    vec![
        (
            fit_display(
                &relative_time(ticket.created_at, OffsetDateTime::now_utc()),
                LIST_AGE_WIDTH,
            ),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        (
            fit_display(state_abbrev(ticket.state), LIST_STATE_WIDTH),
            state_abbrev_style(ticket.state),
        ),
        (
            fit_display(
                &ticket
                    .priority
                    .map(|priority| format!("p{priority}"))
                    .unwrap_or_default(),
                LIST_PRIORITY_WIDTH,
            ),
            Style::default().fg(Color::Magenta),
        ),
    ]
}

fn state_abbrev(state: TicketState) -> &'static str {
    match state {
        TicketState::New => "nw",
        TicketState::Assigned => "as",
        TicketState::InProgress => "ip",
        TicketState::Blocked => "bl",
        TicketState::Review => "rv",
        TicketState::Resolved => "rs",
        TicketState::Wontfix => "wf",
        TicketState::Duplicate => "dp",
        TicketState::Invalid => "iv",
    }
}

fn state_abbrev_style(state: TicketState) -> Style {
    let color = match state {
        TicketState::InProgress => Color::LightYellow,
        TicketState::Blocked => Color::LightRed,
        TicketState::Review => Color::LightBlue,
        TicketState::Resolved => Color::LightGreen,
        TicketState::Wontfix | TicketState::Duplicate | TicketState::Invalid => Color::DarkGray,
        TicketState::New | TicketState::Assigned => Color::Gray,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn push_meta_spans(
    spans: &mut Vec<Span<'static>>,
    meta: &[(String, Style)],
    max_width: usize,
) -> usize {
    let mut used = 0;
    for (value, style) in meta {
        if used > 0 {
            if used >= max_width {
                break;
            }
            spans.push(Span::raw(" "));
            used += 1;
        }

        let value = truncate_display(value, max_width.saturating_sub(used));
        let value_width = UnicodeWidthStr::width(value.as_str());
        spans.push(Span::styled(value, *style));
        used += value_width;
    }
    used
}

fn fit_display(value: &str, width: usize) -> String {
    let truncated = truncate_display(value, width);
    let padding = width.saturating_sub(UnicodeWidthStr::width(truncated.as_str()));
    format!("{truncated}{}", " ".repeat(padding))
}

fn truncate_display(value: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }

    let ellipsis = if max_width > 3 { "..." } else { "." };
    let ellipsis_width = UnicodeWidthStr::width(ellipsis);
    let content_width = max_width.saturating_sub(ellipsis_width);
    let mut out = String::new();
    let mut width = 0;

    for ch in value.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + char_width > content_width {
            break;
        }
        out.push(ch);
        width += char_width;
    }
    out.push_str(ellipsis);
    out
}

fn flatten_display(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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

fn writeup_matches(writeup: &Writeup, needle: &str) -> bool {
    writeup.title.to_ascii_lowercase().contains(needle)
        || writeup
            .latest_body()
            .map(|body| body.to_ascii_lowercase().contains(needle))
            .unwrap_or(false)
        || writeup
            .tags
            .iter()
            .any(|tag| tag.to_ascii_lowercase().contains(needle))
}

fn ticket_edit_body(ticket: &Ticket) -> String {
    let mut body = ticket.title.clone();
    if let Some(description) = &ticket.description {
        body.push_str("\n\n");
        body.push_str(description);
    }
    body
}

fn writeup_edit_body(writeup: &Writeup) -> String {
    let mut body = writeup.title.clone();
    if let Some(latest_body) = writeup.latest_body() {
        body.push_str("\n\n");
        body.push_str(latest_body);
    }
    body
}

fn first_spec_line(spec: &str) -> &str {
    spec.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

fn ticket_matches_tag_filter(
    ticket: &Ticket,
    tags: &BTreeSet<String>,
    tag_filter_match_all: bool,
) -> bool {
    if tags.is_empty() {
        return true;
    }
    if tag_filter_match_all {
        tags.iter().all(|tag| ticket.tags.contains(tag))
    } else {
        tags.iter().any(|tag| ticket.tags.contains(tag))
    }
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn saved_view_tags(view: &SavedView) -> Vec<String> {
    if !view.tags.is_empty() {
        return view.tags.clone();
    }
    view.tag.iter().cloned().collect()
}

fn builtin_views(current_user: &str) -> Vec<ViewEntry> {
    vec![
        ViewEntry {
            name: "Default".to_string(),
            view: SavedView {
                status: Some("open".to_string()),
                tag_match_all: true,
                columns: default_issue_column_names(),
                ..Default::default()
            },
            kind: ViewKind::BuiltIn,
        },
        ViewEntry {
            name: "Mine".to_string(),
            view: SavedView {
                status: Some("open".to_string()),
                assigned: Some(current_user.to_string()),
                tag_match_all: true,
                columns: default_issue_column_names(),
                ..Default::default()
            },
            kind: ViewKind::BuiltIn,
        },
        ViewEntry {
            name: "Recently Closed".to_string(),
            view: SavedView {
                status: Some("closed".to_string()),
                order: Some("closed.desc".to_string()),
                tag_match_all: true,
                columns: vec![
                    "id".to_string(),
                    "closed".to_string(),
                    "state".to_string(),
                    "priority".to_string(),
                    "title".to_string(),
                ],
                ..Default::default()
            },
            kind: ViewKind::BuiltIn,
        },
        ViewEntry {
            name: "All Tickets".to_string(),
            view: SavedView {
                all: true,
                subissues: true,
                tag_match_all: true,
                columns: default_issue_column_names(),
                ..Default::default()
            },
            kind: ViewKind::BuiltIn,
        },
    ]
}

fn default_issue_columns() -> Vec<IssueColumn> {
    vec![
        IssueColumn::Id,
        IssueColumn::Date,
        IssueColumn::State,
        IssueColumn::Priority,
        IssueColumn::Title,
        IssueColumn::Tags,
    ]
}

fn default_issue_column_names() -> Vec<String> {
    default_issue_columns()
        .into_iter()
        .map(|column| column.as_str().to_string())
        .collect()
}

fn saved_issue_columns(view: &SavedView) -> Vec<IssueColumn> {
    let mut columns = view
        .columns
        .iter()
        .filter_map(|column| IssueColumn::parse(column))
        .collect::<Vec<_>>();
    if columns.is_empty() {
        columns = default_issue_columns();
    }
    if !columns.contains(&IssueColumn::Title) {
        columns.push(IssueColumn::Title);
    }
    columns
}

impl IssueColumn {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "id" | "ticid" => Some(Self::Id),
            "date" | "created" | "dt" => Some(Self::Date),
            "closed" | "closed_at" | "closed-at" => Some(Self::Closed),
            "priority" | "prio" | "p" => Some(Self::Priority),
            "state" => Some(Self::State),
            "title" => Some(Self::Title),
            "assignee" | "assigned" => Some(Self::Assignee),
            "points" | "pts" => Some(Self::Points),
            "milestone" => Some(Self::Milestone),
            "tags" => Some(Self::Tags),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Date => "date",
            Self::Closed => "closed",
            Self::Priority => "priority",
            Self::State => "state",
            Self::Title => "title",
            Self::Assignee => "assignee",
            Self::Points => "points",
            Self::Milestone => "milestone",
            Self::Tags => "tags",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Id => "Id",
            Self::Date => "Dt",
            Self::Closed => "Cls",
            Self::Priority => "P",
            Self::State => "St",
            Self::Title => "Title",
            Self::Assignee => "Assignee",
            Self::Points => "Pts",
            Self::Milestone => "Milestone",
            Self::Tags => "Tags",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Id => "ticket id and claimed marker",
            Self::Date => "created relative date",
            Self::Closed => "closed relative date",
            Self::Priority => "priority",
            Self::State => "lifecycle state",
            Self::Title => "ticket title",
            Self::Assignee => "assigned user",
            Self::Points => "estimate points",
            Self::Milestone => "milestone",
            Self::Tags => "colored tags",
        }
    }

    fn fixed_width(self) -> Option<usize> {
        match self {
            Self::Id => Some(LIST_ID_WIDTH + 1),
            Self::Date | Self::Closed => Some(LIST_AGE_WIDTH),
            Self::Priority => Some(LIST_PRIORITY_WIDTH),
            Self::State => Some(LIST_STATE_WIDTH),
            Self::Title => None,
            Self::Assignee => Some(8),
            Self::Points => Some(3),
            Self::Milestone => Some(10),
            Self::Tags => Some(20),
        }
    }
}

fn issue_column_index(column: IssueColumn) -> usize {
    ISSUE_COLUMN_CHOICES
        .iter()
        .position(|candidate| *candidate == column)
        .unwrap_or(usize::MAX)
}

fn issue_columns_label(columns: &[IssueColumn]) -> String {
    columns
        .iter()
        .map(|column| column.label())
        .collect::<Vec<_>>()
        .join(", ")
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

fn comment_summary_line(comment: &Comment, width: usize) -> Line<'static> {
    let date = relative_time(comment.at, OffsetDateTime::now_utc());
    let author = comment_author_display(&comment.author);
    let prefix_width =
        UnicodeWidthStr::width(date.as_str()) + 2 + UnicodeWidthStr::width(author.as_str()) + 2;
    let body_width = width.saturating_sub(prefix_width);
    let body = truncate_display(&flatten_display(&comment.body), body_width);

    Line::from(vec![
        Span::styled(
            date,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        Span::raw("  "),
        Span::styled(author, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::raw(body),
    ])
}

fn comment_author_display(author: &str) -> String {
    let display = author
        .split_once('@')
        .map(|(local, _)| local)
        .filter(|local| !local.is_empty())
        .unwrap_or(author);
    truncate_display(display, 15)
}

fn help_heading(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn help_section(lines: &mut Vec<Line<'static>>, label: &str) {
    if !lines.is_empty() {
        lines.push(Line::raw(""));
    }
    lines.push(help_heading(label));
}

fn help_columns(left: (&str, &str), right: Option<(&str, &str)>) -> Line<'static> {
    let mut spans = Vec::new();
    help_cell(&mut spans, left.0, left.1);
    if let Some(right) = right {
        spans.push(Span::styled("  │  ", Style::default().fg(Color::DarkGray)));
        help_cell(&mut spans, right.0, right.1);
    }
    Line::from(spans)
}

fn help_cell(spans: &mut Vec<Span<'static>>, keys: &str, description: &str) {
    spans.push(Span::styled(
        fit_display(keys, 12),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        fit_display(description, 20),
        Style::default().fg(Color::Cyan),
    ));
}

fn help_note(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(Color::DarkGray),
    ))
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

fn detail_child_issue_line(value: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(" ".repeat(13)),
        Span::styled(value.to_string(), Style::default().fg(Color::Cyan)),
    ])
}

fn writeup_detail_lines(
    writeup: &Writeup,
    tickets: &[Ticket],
    width: usize,
) -> (Vec<Line<'static>>, Vec<MarkdownHeading>) {
    let mut lines = vec![Line::from(Span::styled(
        writeup.id.to_string(),
        Style::default().fg(Color::DarkGray),
    ))];
    lines.extend(writeup_metadata_lines(writeup, width.saturating_sub(2)));
    if !writeup.tags.is_empty() {
        lines.push(tags_field_line(&writeup.tags));
    }
    if let Some(priority) = writeup.priority {
        lines.push(field_line("Priority", &priority.to_string()));
    }
    if let Some(body) = writeup.latest_body() {
        let stats = writeup_body_stats(body);
        lines.push(field_line("Stats", &writeup_stats_display(stats)));
    }
    if !writeup.tickets.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Issues",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        for (idx, ticket_id) in writeup.tickets.iter().take(9).enumerate() {
            let ticket = tickets.iter().find(|ticket| ticket.id == *ticket_id);
            let title = ticket
                .map(|ticket| ticket.title.clone())
                .unwrap_or_else(|| "missing ticket".to_string());
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}", idx + 1),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    ticket_id.to_string()[..6].to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" "),
                Span::raw(title),
            ]));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        writeup.title.clone(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::raw(""));
    let body_start = lines.len();
    if let Some(version) = writeup.versions.last() {
        lines.extend(markdown_body_lines(&version.body));
        let headings = parse_markdown_headings(&version.body)
            .into_iter()
            .map(|heading| MarkdownHeading {
                line: heading.line + body_start,
                ..heading
            })
            .collect();
        (lines, headings)
    } else {
        lines.push(Line::from(Span::styled(
            "No versions yet. Press e to add one.",
            Style::default().fg(Color::DarkGray),
        )));
        (lines, Vec::new())
    }
}

fn writeup_body_stats(body: &str) -> WriteupBodyStats {
    let words = body
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .count();
    WriteupBodyStats {
        words,
        read_minutes: words.div_ceil(200).max(usize::from(words > 0)),
        headings: parse_markdown_headings(body).len(),
    }
}

fn writeup_stats_display(stats: WriteupBodyStats) -> String {
    let word_label = if stats.words == 1 { "word" } else { "words" };
    let heading_label = if stats.headings == 1 {
        "heading"
    } else {
        "headings"
    };
    format!(
        "{} {word_label}, {} min read, {} {heading_label}",
        stats.words, stats.read_minutes, stats.headings
    )
}

fn markdown_body_lines(body: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") || trimmed_start.starts_with("~~~") {
            in_fence = !in_fence;
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }
        if in_fence {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Green),
            )));
        } else {
            lines.push(markdown_line(line));
        }
    }
    lines
}

fn markdown_line(line: &str) -> Line<'static> {
    let leading = line.len().saturating_sub(line.trim_start().len());
    let trimmed_start = line.trim_start();
    if let Some((level, title)) = markdown_heading(trimmed_start) {
        let color = markdown_heading_color(level);
        let mut spans = Vec::new();
        if leading > 0 {
            spans.push(Span::raw(" ".repeat(leading)));
        }
        spans.push(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        return Line::from(spans);
    }

    if is_markdown_rule(trimmed_start) {
        return Line::from(Span::styled(
            "─".repeat(trimmed_start.chars().count().max(3)),
            Style::default().fg(Color::DarkGray),
        ));
    }

    if let Some(rest) = trimmed_start.strip_prefix(">") {
        let mut spans = Vec::new();
        if leading > 0 {
            spans.push(Span::raw(" ".repeat(leading)));
        }
        spans.push(Span::styled(">", Style::default().fg(Color::DarkGray)));
        spans.extend(markdown_inline_spans(
            rest.trim_start(),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
        ));
        return Line::from(spans);
    }

    if let Some((marker, rest)) = markdown_list_marker(trimmed_start) {
        let mut spans = Vec::new();
        if leading > 0 {
            spans.push(Span::raw(" ".repeat(leading)));
        }
        spans.push(Span::styled(marker, Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(" "));
        spans.extend(markdown_inline_spans(rest, Style::default()));
        return Line::from(spans);
    }

    Line::from(markdown_inline_spans(line, Style::default()))
}

fn markdown_heading(line: &str) -> Option<(usize, String)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after_hashes = &line[hashes..];
    if !after_hashes.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let title = after_hashes.trim().trim_end_matches('#').trim().to_string();
    (!title.is_empty()).then_some((hashes, title))
}

fn markdown_heading_color(level: usize) -> Color {
    match level {
        1 => Color::Cyan,
        2 => Color::LightCyan,
        3 => Color::Yellow,
        4 => Color::LightYellow,
        5 => Color::Magenta,
        _ => Color::Gray,
    }
}

fn is_markdown_rule(line: &str) -> bool {
    let mut chars = line.chars();
    let Some(marker @ ('-' | '_' | '*')) = chars.next() else {
        return false;
    };
    let mut count = 1;
    for ch in chars {
        if ch.is_whitespace() {
            continue;
        }
        if ch != marker {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn markdown_list_marker(line: &str) -> Option<(String, &str)> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some((marker.trim().to_string(), rest));
        }
    }
    let marker_end = line.find(". ")?;
    if marker_end == 0 || !line[..marker_end].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((line[..=marker_end].to_string(), &line[marker_end + 2..]))
}

fn markdown_inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let next = ["**", "__", "`"]
            .iter()
            .filter_map(|marker| rest.find(marker).map(|idx| (idx, *marker)))
            .min_by_key(|(idx, _)| *idx);
        let Some((idx, marker)) = next else {
            spans.push(Span::styled(rest.to_string(), base_style));
            break;
        };
        if idx > 0 {
            spans.push(Span::styled(rest[..idx].to_string(), base_style));
        }
        let marker_len = marker.len();
        let after_marker = &rest[idx + marker_len..];
        if let Some(end) = after_marker.find(marker) {
            let content = &after_marker[..end];
            let style = if marker == "`" {
                Style::default().fg(Color::Yellow).bg(Color::DarkGray)
            } else {
                base_style.add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(content.to_string(), style));
            rest = &after_marker[end + marker_len..];
        } else {
            spans.push(Span::styled(marker.to_string(), base_style));
            rest = after_marker;
        }
    }
    spans
}

fn parse_markdown_headings(body: &str) -> Vec<MarkdownHeading> {
    let mut headings = Vec::new();
    let mut in_fence = false;
    for (line_idx, line) in body.lines().enumerate() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") || trimmed_start.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some((hashes, title)) = markdown_heading(trimmed_start) {
            headings.push(MarkdownHeading {
                level: hashes,
                title,
                line: line_idx,
            });
        }
    }
    headings
}

#[derive(Clone)]
struct MetadataField {
    key: &'static str,
    label: &'static str,
    value: String,
}

fn writeup_metadata_lines(writeup: &Writeup, width: usize) -> Vec<Line<'static>> {
    let mut fields = vec![
        MetadataField {
            key: "updated",
            label: "Updated",
            value: format!(
                "{} ago",
                relative_time(writeup_recent_at(writeup), OffsetDateTime::now_utc())
            ),
        },
        MetadataField {
            key: "status",
            label: "Status",
            value: writeup.status.as_str().to_string(),
        },
        MetadataField {
            key: "versions",
            label: "Versions",
            value: writeup.versions.len().to_string(),
        },
        MetadataField {
            key: "authors",
            label: "Authors",
            value: writeup
                .authors
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        },
    ];

    for key in ["versions", "status", "authors"] {
        if metadata_fields_width(&fields) <= width {
            break;
        }
        if let Some(idx) = fields.iter().position(|field| field.key == key) {
            fields.remove(idx);
        }
    }

    metadata_lines(&fields, width)
}

fn metadata_fields_width(fields: &[MetadataField]) -> usize {
    fields.iter().map(metadata_field_width).sum::<usize>() + fields.len().saturating_sub(1) * 2
}

fn metadata_field_width(field: &MetadataField) -> usize {
    UnicodeWidthStr::width(field.label).max(UnicodeWidthStr::width(field.value.as_str()))
}

fn metadata_lines(fields: &[MetadataField], width: usize) -> Vec<Line<'static>> {
    if fields.is_empty() || width == 0 {
        return Vec::new();
    }

    let mut widths = fields.iter().map(metadata_field_width).collect::<Vec<_>>();
    let total_width = widths.iter().sum::<usize>() + fields.len().saturating_sub(1) * 2;
    if total_width > width {
        let overflow = total_width - width;
        if let Some(last_width) = widths.last_mut() {
            *last_width = last_width.saturating_sub(overflow).max(1);
        }
    }

    let mut label_spans = Vec::new();
    let mut value_spans = Vec::new();
    for (idx, (field, column_width)) in fields.iter().zip(widths).enumerate() {
        if idx > 0 {
            label_spans.push(Span::raw("  "));
            value_spans.push(Span::raw("  "));
        }
        label_spans.push(Span::styled(
            fit_display(field.label, column_width),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        value_spans.push(Span::styled(
            fit_display(&field.value, column_width),
            Style::default().fg(Color::Cyan),
        ));
    }

    vec![Line::from(label_spans), Line::from(value_spans)]
}

fn spec_field_line(spec: &str, width: usize) -> Line<'static> {
    let label_width = 10;
    let separator_width = 3;
    let hint_key = "i";
    let hint_desc = "view/edit";
    let hint_width = UnicodeWidthStr::width(hint_key) + 1 + UnicodeWidthStr::width(hint_desc);
    let first_line = first_spec_line(spec);
    let value_budget = width.saturating_sub(label_width + separator_width + hint_width + 2);
    let value = truncate_display(first_line, value_budget);
    let value_width = UnicodeWidthStr::width(value.as_str());
    let padding_width = width
        .saturating_sub(label_width + separator_width + value_width + hint_width)
        .max(2);

    Line::from(vec![
        Span::styled(
            format!("{:<10}", "Spec"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" : ", Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(Color::Cyan)),
        Span::raw(" ".repeat(padding_width)),
        Span::styled(
            hint_key,
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(hint_desc, Style::default().fg(Color::DarkGray)),
    ])
}

fn tags_field_line(tags: &BTreeSet<String>) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!("{:<10}", "Tags"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" : ", Style::default().fg(Color::DarkGray)),
    ];
    let mut first = true;
    for tag in tags {
        if !first {
            spans.push(Span::raw(", "));
        }
        first = false;
        spans.push(Span::styled(
            tag.clone(),
            Style::default().fg(tag_color(tag)),
        ));
    }
    Line::from(spans)
}

fn tag_spans(tags: &BTreeSet<String>, max_width: usize) -> Vec<Span<'static>> {
    if max_width == 0 || tags.is_empty() {
        return Vec::new();
    }

    let tag_values = tags.iter().collect::<Vec<_>>();
    for keep in (1..=tag_values.len()).rev() {
        let hidden = tag_values.len() - keep;
        let spans = tag_list_spans(&tag_values[..keep], hidden);
        if spans_width(&spans) <= max_width {
            return spans;
        }
    }

    let spans = tag_count_spans(tags.len());
    if spans_width(&spans) <= max_width {
        spans
    } else {
        Vec::new()
    }
}

fn tag_list_spans(tags: &[&String], hidden: usize) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw("[")];
    for (idx, tag) in tags.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw(","));
        }
        spans.push(Span::styled(
            (*tag).clone(),
            Style::default().fg(tag_color(tag)),
        ));
    }
    if hidden > 0 {
        if !tags.is_empty() {
            spans.push(Span::raw(","));
        }
        spans.push(Span::styled(
            format!("+{hidden}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    spans.push(Span::raw("]"));
    spans
}

fn tag_count_spans(count: usize) -> Vec<Span<'static>> {
    vec![
        Span::raw("["),
        Span::styled(count.to_string(), Style::default().fg(Color::DarkGray)),
        Span::raw("]"),
    ]
}

fn tag_count_width(count: usize) -> usize {
    spans_width(&tag_count_spans(count))
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn tag_color(tag: &str) -> Color {
    let hash = stable_hash(tag);
    if supports_truecolor() {
        let hue = (hash % 360) as f32;
        let (r, g, b) = hsl_to_rgb(hue, 0.68, 0.62);
        Color::Rgb(r, g, b)
    } else {
        ANSI_TAG_COLORS[(hash as usize) % ANSI_TAG_COLORS.len()]
    }
}

fn supports_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("truecolor") || value.contains("24bit")
        })
        .unwrap_or(false)
        || std::env::var("TERM")
            .map(|value| value.to_ascii_lowercase().contains("direct"))
            .unwrap_or(false)
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (u8, u8, u8) {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let hue_prime = hue / 60.0;
    let x = chroma * (1.0 - (hue_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hue_prime as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

fn status_state_line(ticket: &Ticket) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:<10}", "Status"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" : ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}:{}", ticket.status.as_str(), ticket.state.as_str()),
            Style::default().fg(Color::Green),
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_issue_columns_fall_back_to_default_and_keep_title() {
        let default = saved_issue_columns(&SavedView::default());
        assert_eq!(default, default_issue_columns());

        let custom = saved_issue_columns(&SavedView {
            columns: vec!["closed".to_string(), "priority".to_string()],
            ..Default::default()
        });
        assert_eq!(
            custom,
            vec![
                IssueColumn::Closed,
                IssueColumn::Priority,
                IssueColumn::Title
            ]
        );
    }

    #[test]
    fn issue_columns_drop_optional_fields_to_fit_width() {
        let columns = vec![
            IssueColumn::Id,
            IssueColumn::Closed,
            IssueColumn::Priority,
            IssueColumn::Title,
            IssueColumn::Tags,
        ];

        assert_eq!(issue_columns_for_width(&columns, 80), columns);
        assert_eq!(
            issue_columns_for_width(&columns, 43),
            vec![
                IssueColumn::Id,
                IssueColumn::Closed,
                IssueColumn::Priority,
                IssueColumn::Title
            ]
        );
        assert_eq!(
            issue_columns_for_width(&columns, 38),
            vec![IssueColumn::Id, IssueColumn::Title]
        );
        assert_eq!(
            issue_columns_for_width(&columns, 3),
            vec![IssueColumn::Title]
        );
    }

    #[test]
    fn recently_closed_builtin_uses_closed_column_and_order() {
        let views = builtin_views("me@example.com");
        let recent = views
            .iter()
            .find(|view| view.name == "Recently Closed")
            .unwrap();

        assert_eq!(recent.view.order.as_deref(), Some("closed.desc"));
        assert!(recent.view.columns.contains(&"closed".to_string()));
        assert!(!recent.view.columns.contains(&"date".to_string()));
    }

    #[test]
    fn default_open_views_hide_subissues() {
        let views = builtin_views("me@example.com");

        for name in ["Default", "Mine", "Recently Closed"] {
            let view = views.iter().find(|view| view.name == name).unwrap();
            assert!(!view.view.subissues, "{name} should hide subissues");
        }
    }

    #[test]
    fn id_column_reserves_space_for_claimed_marker() {
        assert_eq!(IssueColumn::Id.fixed_width(), Some(4));
    }

    #[test]
    fn tag_column_uses_colored_tag_spans() {
        let mut tags = BTreeSet::new();
        tags.insert("bug".to_string());

        let mut spans = Vec::new();
        push_issue_tags_column(&mut spans, &tags, 8);

        assert_eq!(spans_width(&spans), 8);
        assert!(spans.iter().any(|span| span.content.as_ref() == "bug"));
    }

    #[test]
    fn outline_rows_indent_visible_subissues() {
        let root = uuid::Uuid::from_u128(1);
        let child = uuid::Uuid::from_u128(2);
        let grandchild = uuid::Uuid::from_u128(3);
        let sibling = uuid::Uuid::from_u128(4);
        let tickets = vec![
            test_ticket(root, None, &[child]),
            test_ticket(child, Some(root), &[grandchild]),
            test_ticket(grandchild, Some(child), &[]),
            test_ticket(sibling, None, &[]),
        ];

        let rows = build_outline_rows(&tickets, &[0, 1, 2, 3], &BTreeSet::new());

        assert_eq!(
            rows.iter()
                .map(|row| (tickets[row.ticket_idx].id, row.depth, row.has_children))
                .collect::<Vec<_>>(),
            vec![
                (root, 0, true),
                (child, 1, true),
                (grandchild, 2, false),
                (sibling, 0, false)
            ]
        );
    }

    #[test]
    fn outline_rows_hide_descendants_under_collapsed_parent() {
        let root = uuid::Uuid::from_u128(1);
        let child = uuid::Uuid::from_u128(2);
        let sibling = uuid::Uuid::from_u128(3);
        let tickets = vec![
            test_ticket(root, None, &[child]),
            test_ticket(child, Some(root), &[]),
            test_ticket(sibling, None, &[]),
        ];
        let collapsed = BTreeSet::from([root]);

        let rows = build_outline_rows(&tickets, &[0, 1, 2], &collapsed);

        assert_eq!(
            rows.iter()
                .map(|row| (tickets[row.ticket_idx].id, row.depth, row.collapsed))
                .collect::<Vec<_>>(),
            vec![(root, 0, true), (sibling, 0, false)]
        );
    }

    #[test]
    fn subissue_graph_prefix_marks_parents_and_children() {
        let root = uuid::Uuid::from_u128(1);
        let child = uuid::Uuid::from_u128(2);
        let grandchild = uuid::Uuid::from_u128(3);
        let tickets = vec![
            test_ticket(root, None, &[child]),
            test_ticket(child, Some(root), &[grandchild]),
            test_ticket(grandchild, Some(child), &[]),
        ];
        let ticket_by_id = tickets
            .iter()
            .map(|ticket| (ticket.id, ticket))
            .collect::<HashMap<_, _>>();

        assert_eq!(subissue_graph_prefix(&tickets[0], &ticket_by_id), "");
        assert_eq!(subissue_graph_prefix(&tickets[1], &ticket_by_id), " ╰┄ ");
        assert_eq!(subissue_graph_prefix(&tickets[2], &ticket_by_id), "  ╰┄ ");
    }

    #[test]
    fn issue_title_prefix_marks_parents_even_when_graph_is_hidden() {
        let root = uuid::Uuid::from_u128(1);
        let child = uuid::Uuid::from_u128(2);
        let tickets = vec![
            test_ticket(root, None, &[child]),
            test_ticket(child, Some(root), &[]),
        ];
        let ticket_by_id = tickets
            .iter()
            .map(|ticket| (ticket.id, ticket))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            issue_title_prefix(&tickets[0], &ticket_by_id, false),
            "[+] "
        );
        assert_eq!(issue_title_prefix(&tickets[1], &ticket_by_id, false), "");
        assert_eq!(issue_title_prefix(&tickets[1], &ticket_by_id, true), " ╰┄ ");
    }

    #[test]
    fn list_order_places_visible_children_after_parent() {
        let root = uuid::Uuid::from_u128(1);
        let child = uuid::Uuid::from_u128(2);
        let sibling = uuid::Uuid::from_u128(3);
        let tickets = vec![
            test_ticket(root, None, &[child]),
            test_ticket(child, Some(root), &[]),
            test_ticket(sibling, None, &[]),
        ];

        assert_eq!(
            ordered_list_indices(&tickets, &[1, 0, 2], false),
            vec![1, 0, 2]
        );
        assert_eq!(
            ordered_list_indices(&tickets, &[1, 0, 2], true),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn markdown_headings_ignore_code_fences_and_track_levels() {
        let headings = parse_markdown_headings(
            "# Intro\ntext\n```md\n# ignored\n```\n## Details ##\n#### Deep\nnot # heading",
        );

        assert_eq!(
            headings,
            vec![
                MarkdownHeading {
                    level: 1,
                    title: "Intro".to_string(),
                    line: 0,
                },
                MarkdownHeading {
                    level: 2,
                    title: "Details".to_string(),
                    line: 5,
                },
                MarkdownHeading {
                    level: 4,
                    title: "Deep".to_string(),
                    line: 6,
                },
            ]
        );
    }

    #[test]
    fn markdown_line_styles_headers() {
        let line = markdown_line("## Details ##");

        assert_eq!(line.spans[0].content.as_ref(), "Details");
        assert_eq!(line.spans[0].style.fg, Some(Color::LightCyan));
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn markdown_line_styles_bold_and_code_spans() {
        let line = markdown_line("Use **bold** and `code` here");

        assert_eq!(line.spans[1].content.as_ref(), "bold");
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[3].content.as_ref(), "code");
        assert_eq!(line.spans[3].style.fg, Some(Color::Yellow));
        assert_eq!(line.spans[3].style.bg, Some(Color::DarkGray));
    }

    #[test]
    fn markdown_body_lines_style_fenced_code_without_headings() {
        let lines = markdown_body_lines("```md\n# not heading\n```\n# Heading");

        assert_eq!(lines[1].spans[0].content.as_ref(), "# not heading");
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::Green));
        assert_eq!(lines[3].spans[0].content.as_ref(), "Heading");
        assert_eq!(lines[3].spans[0].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn writeup_body_stats_count_words_read_time_and_headings() {
        let body = "# Intro\none two three\n## Next\nfour";

        assert_eq!(
            writeup_body_stats(body),
            WriteupBodyStats {
                words: 6,
                read_minutes: 1,
                headings: 2,
            }
        );
    }

    fn test_ticket(id: uuid::Uuid, parent: Option<uuid::Uuid>, children: &[uuid::Uuid]) -> Ticket {
        Ticket {
            id,
            title: format!("Ticket {id}"),
            description: None,
            spec: None,
            status: TicketStatus::Open,
            state: TicketState::New,
            assigned: None,
            closed_by: None,
            priority: None,
            points: None,
            milestone: None,
            code: None,
            parent,
            children: children.iter().copied().collect(),
            depends_on: BTreeSet::new(),
            blocks: BTreeSet::new(),
            tags: BTreeSet::new(),
            meta: BTreeMap::new(),
            comments: Vec::new(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "test@example.com".to_string(),
        }
    }
}
