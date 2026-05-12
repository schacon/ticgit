//! Clap definitions and top-level dispatch.

use clap::{ArgAction, Parser, Subcommand};

use crate::commands;

#[derive(Debug, Parser)]
#[command(
    name = "ti",
    version,
    disable_version_flag = true,
    about = "Tickets in your Git repo, stored as git-meta metadata",
    subcommand_help_heading = "",
    hide_possible_values = true,
    help_template = "\
{about}

\x1b[1;36mUsage:\x1b[0m {usage}

\x1b[1;36mCreate & Browse:\x1b[0m
  new        Create a new ticket
  list, ls   List tickets, with optional filters
  show       Show one ticket and its comments
  recent     Show the most recently touched tickets
  mine       List tickets assigned to you
  history    Show change history for a ticket
  tui        Browse open tickets in an interactive terminal UI

\x1b[1;36mWork on Tickets:\x1b[0m
  checkout, co  Select a ticket as \"current\"
  next          Pick the next best ticket and check it out
  edit          Edit a ticket's title and description
  comment       Add a comment to a ticket
  state         Change a ticket's lifecycle status/state
  close         Close a ticket (shorthand for state resolved)

\x1b[1;36mTicket Fields:\x1b[0m
  tag        Add or remove a tag
  assign     Set or clear assigned user
  priority   Set or clear priority (lower = more important)
  points     Set or clear points (estimate)
  milestone  Set or clear milestone
  subissue   Make a ticket a sub-issue of another
  code       Set or clear a code URI (repo + branch)
  depends    Add or remove a dependency between tickets
  meta       Set a custom metadata field

\x1b[1;36mViews & Import:\x1b[0m
  views      Manage saved views (save, delete, list)
  writeup    Capture rough notes and promote them to tickets
  stats      Show a ticket stats dashboard
  import     Import tickets from external systems (e.g. GitHub)

\x1b[1;36mTeam:\x1b[0m
  users      Manage user nick/email mappings (shared mailmap)
  mine       List tickets assigned to you

\x1b[1;36mSync & Setup:\x1b[0m
  sync       Sync ticket metadata with a Git remote
  pull       Pull tickets from a fork or remote URL
  init       Initialise ticgit on the current repo
  setup      Configure git-meta remote from .git-meta
  update     Update ti to the latest release

\x1b[1;36mAgents:\x1b[0m
  agent      Markdown guide for AI agents
  list --markdown          Markdown ticket list
  show <id> --markdown     Ticket detail with next-step suggestions

\x1b[1;36mExamples:\x1b[0m
  ti new --title \"fix the parser\" --tags bug
  ti list --tag bug
  ti views save bugs
  ti list bugs
  ti show a3f
  ti checkout a3f && ti comment \"on it\"
  ti state resolved --ticket a3f
  ti sync

\x1b[1;36mAgent Examples:\x1b[0m
  ti agent
  ti list --markdown
  ti show a3f --markdown
  ti new -F /tmp/ticket.md --markdown
  ti comment --ticket a3f \"done\"

\x1b[1;36mOptions:\x1b[0m
{options}"
)]
pub struct Cli {
    /// Print version.
    #[arg(short = 'v', long = "version", action = ArgAction::Version)]
    pub version: Option<bool>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // -- Create & browse --------------------------------------------------
    /// Create a new ticket.
    #[command(next_help_heading = "Create & Browse")]
    New(commands::new::Args),

    /// List tickets, with optional filters.
    #[command(visible_alias = "ls")]
    List(commands::list::Args),

    /// Show one ticket and its comments.
    Show(commands::show::Args),

    /// Show the most recently touched tickets.
    Recent(commands::recent::Args),

    /// List tickets assigned to you (from git config user.email).
    Mine(commands::list::Args),

    /// Show change history for a ticket.
    History(commands::history::Args),

    /// Browse open tickets in an interactive terminal UI.
    Tui(commands::tui::Args),

    /// Print a Markdown guide for AI agents.
    Agent,

    // -- Work on tickets --------------------------------------------------
    /// Select a ticket as "current" for subsequent commands.
    #[command(visible_alias = "co", next_help_heading = "Work on Tickets")]
    Checkout(commands::checkout::Args),

    /// Pick the next best ticket to work on and check it out.
    Next(commands::next::Args),

    /// Assign a ticket to you and mark it assigned.
    Claim(commands::claim::Args),

    /// Edit a ticket's title and description in your editor.
    Edit(commands::edit::Args),

    /// Add a comment to a ticket.
    Comment(commands::comment::Args),

    /// Change a ticket's lifecycle status/state.
    State(commands::state::Args),

    /// Alias for `state`.
    Status(commands::state::Args),

    /// Close a ticket by marking it resolved.
    Close(commands::close::Args),

    // -- Ticket fields ----------------------------------------------------
    /// Add or remove a tag on a ticket.
    #[command(next_help_heading = "Ticket Fields")]
    Tag(commands::tag::Args),

    /// Set or clear a ticket's assigned user.
    Assign(commands::assign::Args),

    /// Set or clear a ticket's priority (lower = more important).
    Priority(commands::priority::Args),

    /// Set or clear a ticket's points (estimate).
    Points(commands::points::Args),

    /// Set or clear a ticket's milestone.
    Milestone(commands::milestone::Args),

    /// Make a ticket a sub-issue of another ticket.
    Subissue(commands::subissue::Args),

    /// Set or clear a code URI (https://host/path:branch) for associated code.
    Code(commands::code::Args),

    /// Add or remove a dependency between tickets.
    #[command(visible_alias = "dep")]
    Depends(commands::depends::Args),

    /// Set or clear a ticket's implementation spec.
    Spec(commands::spec::Args),

    /// Set a string metadata field on a ticket.
    Meta(commands::meta::Args),

    // -- Views & import ---------------------------------------------------
    /// Manage saved views (save, delete, list).
    #[command(next_help_heading = "Views & Import")]
    Views(commands::view::Args),

    /// Capture rough notes and promote them to tickets.
    Writeup(commands::writeup::Args),

    /// Show a ticket stats dashboard.
    Stats(commands::stats::Args),

    /// Import tickets from external systems.
    Import(commands::import::Args),

    // -- Team ---------------------------------------------------------------
    /// Manage user nick → email mappings (shared mailmap).
    #[command(next_help_heading = "Team")]
    Users(commands::users::Args),

    // -- Sync & setup -----------------------------------------------------
    /// Sync ticket metadata with a Git remote (pull then push).
    #[command(next_help_heading = "Sync & Setup")]
    Sync(commands::sync::Args),

    /// Pull tickets from a fork or remote URL.
    Pull(commands::pull::Args),

    /// Initialise ticgit metadata on the current repo (idempotent).
    Init,

    /// Configure git-meta remote from `.git-meta` file (idempotent).
    Setup,

    /// Update ti to the latest release.
    Update(commands::update::Args),
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        None => commands::list::run(commands::list::Args::default()),
        Some(Command::Init) => commands::init::run(),
        Some(Command::Setup) => commands::setup::run(),
        Some(Command::New(args)) => commands::new::run(args),
        Some(Command::List(args)) => commands::list::run(args),
        Some(Command::Show(args)) => commands::show::run(args),
        Some(Command::Checkout(args)) => commands::checkout::run(args),
        Some(Command::Next(args)) => commands::next::run(args),
        Some(Command::Claim(args)) => commands::claim::run(args),
        Some(Command::Close(args)) => commands::close::run(args),
        Some(Command::Edit(args)) => commands::edit::run(args),
        Some(Command::Stats(args)) => commands::stats::run(args),
        Some(Command::Import(args)) => commands::import::run(args),
        Some(Command::Recent(args)) => commands::recent::run(args),
        Some(Command::Mine(mut args)) => {
            if args.assigned.is_none() {
                let output = std::process::Command::new("git")
                    .args(["config", "user.email"])
                    .output();
                if let Ok(out) = output {
                    let email = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !email.is_empty() {
                        args.assigned = Some(email);
                    }
                }
            }
            commands::list::run(args)
        }
        Some(Command::History(args)) => commands::history::run(args),
        Some(Command::Tui(args)) => commands::tui::run(args),
        Some(Command::Agent) => {
            crate::agent_help::print();
            Ok(())
        }
        Some(Command::Tag(args)) => commands::tag::run(args),
        Some(Command::State(args)) => commands::state::run(args),
        Some(Command::Status(args)) => commands::state::run(args),
        Some(Command::Assign(args)) => commands::assign::run(args),
        Some(Command::Priority(args)) => commands::priority::run(args),
        Some(Command::Points(args)) => commands::points::run(args),
        Some(Command::Milestone(args)) => commands::milestone::run(args),
        Some(Command::Subissue(args)) => commands::subissue::run(args),
        Some(Command::Code(args)) => commands::code::run(args),
        Some(Command::Depends(args)) => commands::depends::run(args),
        Some(Command::Spec(args)) => commands::spec::run(args),
        Some(Command::Meta(args)) => commands::meta::run(args),
        Some(Command::Comment(args)) => commands::comment::run(args),
        Some(Command::Views(args)) => commands::view::run(args),
        Some(Command::Writeup(args)) => commands::writeup::run(args),
        Some(Command::Users(args)) => commands::users::run(args),
        Some(Command::Sync(args)) => commands::sync::run_sync(args),
        Some(Command::Pull(args)) => commands::pull::run(args),
        Some(Command::Update(args)) => commands::update::run(args),
    }
}
