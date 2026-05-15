//! Editor integration — delegates to `soe` for editor resolution and capture.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

const MARKDOWN_COMMENT_PREFIX: &str = "TICGIT:";

/// Open the best available editor with comment-prompt instructions.
/// Returns `Some(content)` on save, `None` if cancelled or empty.
pub fn capture(prompt: &str) -> Result<Option<String>> {
    soe::capture(prompt)
}

/// Like [`capture`], but pre-fills the buffer with `initial` content.
pub fn capture_with_initial(prompt: &str, initial: &str) -> Result<Option<String>> {
    soe::capture_with_initial(prompt, initial)
}

/// Open an editor for Markdown content without treating Markdown headings as
/// comments.
pub fn capture_markdown(prompt: &str) -> Result<Option<String>> {
    capture_markdown_with_initial(prompt, "")
}

/// Like [`capture_markdown`], but pre-fills the buffer with `initial` content.
pub fn capture_markdown_with_initial(prompt: &str, initial: &str) -> Result<Option<String>> {
    match resolve_editor() {
        Some(editor) => capture_markdown_external(&editor, prompt, initial),
        None => capture_markdown_builtin(prompt, initial),
    }
}

/// Open the editor to capture a ticket comment with ticket context in the
/// commented prompt block.
pub fn capture_comment(title: &str) -> Result<Option<String>> {
    capture(&comment_prompt(title))
}

fn comment_prompt(title: &str) -> String {
    format!("Ticket comment\nTicket: {title}\nLines starting with # are ignored.")
}

/// Read a ticket title/description body from disk. The first line is the
/// title; remaining lines become the optional description.
pub fn read_ticket_edit_file(path: &Path) -> Result<(String, Option<String>)> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading ticket content from `{}`", path.display()))?;
    parse_ticket_edit(&raw)
}

pub fn parse_ticket_edit(raw: &str) -> Result<(String, Option<String>)> {
    let mut lines = raw.lines();
    let title = lines.next().unwrap_or_default().trim().to_string();
    if title.is_empty() {
        anyhow::bail!("ticket title cannot be empty");
    }

    let description = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    let description = if description.is_empty() {
        None
    } else {
        Some(description)
    };

    Ok((title, description))
}

fn capture_markdown_external(editor: &str, prompt: &str, initial: &str) -> Result<Option<String>> {
    let mut tf = tempfile::Builder::new()
        .prefix("ti-writeup-")
        .suffix(".md")
        .tempfile()
        .context("creating editor tempfile")?;

    write_markdown_capture_buffer(&mut tf, prompt, initial)?;
    tf.flush().context("flushing editor tempfile")?;

    let path = tf.path().to_path_buf();
    let status =
        spawn_editor(editor, &path).with_context(|| format!("spawning editor `{editor}`"))?;
    if !status.success() {
        anyhow::bail!("editor `{editor}` exited with status {status}");
    }

    let mut contents = String::new();
    std::fs::File::open(&path)
        .context("re-opening editor tempfile")?
        .read_to_string(&mut contents)
        .context("reading editor tempfile")?;

    Ok(strip_prefixed_comments(&contents, MARKDOWN_COMMENT_PREFIX))
}

fn capture_markdown_builtin(prompt: &str, initial: &str) -> Result<Option<String>> {
    let mut content = Vec::new();
    write_markdown_capture_buffer(&mut content, prompt, initial)?;
    let content = String::from_utf8(content).context("building editor buffer")?;

    let result = soe::edit("ti-writeup.md", &content, soe::EditorMode::PlainText)?;

    Ok(result.and_then(|text| strip_prefixed_comments(&text, MARKDOWN_COMMENT_PREFIX)))
}

fn write_markdown_capture_buffer(out: &mut impl Write, prompt: &str, initial: &str) -> Result<()> {
    if !initial.is_empty() {
        write!(out, "{initial}").context("writing initial content to editor buffer")?;
        if !initial.ends_with('\n') {
            writeln!(out).context("terminating initial content")?;
        }
        writeln!(out).ok();
    }

    for line in prompt.lines() {
        writeln!(out, "{MARKDOWN_COMMENT_PREFIX} {line}")
            .context("writing prompt to editor buffer")?;
    }
    writeln!(
        out,
        "{MARKDOWN_COMMENT_PREFIX} Lines starting with {MARKDOWN_COMMENT_PREFIX} are ignored."
    )
    .context("writing prompt to editor buffer")?;
    writeln!(out).ok();
    Ok(())
}

fn strip_prefixed_comments(text: &str, prefix: &str) -> Option<String> {
    let cleaned = text
        .lines()
        .filter(|line| !line.trim_start().starts_with(prefix))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn spawn_editor(editor: &str, path: &Path) -> Result<std::process::ExitStatus> {
    let path_str = path.display().to_string();
    if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", &format!("{editor} \"{path_str}\"")])
            .status()
            .map_err(Into::into)
    } else {
        Command::new("sh")
            .args(["-c", &format!("{editor} \"$@\""), "--", &path_str])
            .status()
            .map_err(Into::into)
    }
}

fn resolve_editor() -> Option<String> {
    fn non_empty_var(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }

    non_empty_var("GIT_EDITOR")
        .or_else(git_config_editor)
        .or_else(|| non_empty_var("VISUAL"))
        .or_else(|| non_empty_var("EDITOR"))
}

fn git_config_editor() -> Option<String> {
    let output = Command::new("git")
        .args(["config", "core.editor"])
        .output()
        .ok()?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_prompt_includes_ticket_title() {
        let prompt = comment_prompt("Fix the thing");

        assert!(prompt.contains("Ticket: Fix the thing"));
        assert!(prompt.contains("Lines starting with # are ignored."));
    }

    #[test]
    fn parse_ticket_edit_preserves_markdown_headings() {
        let (title, description) = parse_ticket_edit("Updated title\n\n# Heading\n\nBody").unwrap();

        assert_eq!(title, "Updated title");
        assert_eq!(description.as_deref(), Some("# Heading\n\nBody"));
    }

    #[test]
    fn markdown_capture_strips_only_ticgit_prompt_lines() {
        let text = "Title\n\n# Heading\nBody\nTICGIT: prompt\n  TICGIT: prompt";

        assert_eq!(
            strip_prefixed_comments(text, MARKDOWN_COMMENT_PREFIX).as_deref(),
            Some("Title\n\n# Heading\nBody")
        );
    }
}
