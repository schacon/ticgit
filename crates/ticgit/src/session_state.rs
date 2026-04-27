//! Per-checkout, per-user session state - currently just "which ticket
//! is currently checked out".
//!
//! This is the only piece of ticgit that lives outside the repo. It maps
//! a repository git-dir path to the currently-selected ticket UUID, so
//! `ti show` (without args) and `ti comment` can know what you mean.
//!
//! On Linux/macOS we put the cache under `$XDG_STATE_HOME/ticgit/` (or
//! `~/.local/state/ticgit/`); on macOS Application Support; on Windows
//! the standard cache dir.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// Map of canonicalised git-dir path → currently checked-out ticket UUID.
    pub current: HashMap<String, Uuid>,
}

fn state_file() -> Result<PathBuf> {
    if let Ok(override_path) = std::env::var("TICGIT_STATE_FILE") {
        return Ok(PathBuf::from(override_path));
    }
    let base = dirs::state_dir()
        .or_else(dirs::cache_dir)
        .or_else(dirs::home_dir)
        .context("could not determine a state directory")?;
    Ok(base.join("ticgit").join("state.json"))
}

impl State {
    pub fn load() -> Result<Self> {
        let path = state_file()?;
        if !path.exists() {
            return Ok(State::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(State::default());
        }
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    pub fn save(&self) -> Result<()> {
        let path = state_file()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn current_for(&self, git_dir: &Path) -> Option<Uuid> {
        let key = key_for(git_dir);
        self.current.get(&key).copied()
    }

    pub fn set_current(&mut self, git_dir: &Path, id: Uuid) {
        self.current.insert(key_for(git_dir), id);
    }

    pub fn clear_current(&mut self, git_dir: &Path) {
        self.current.remove(&key_for(git_dir));
    }
}

fn key_for(git_dir: &Path) -> String {
    git_dir
        .canonicalize()
        .unwrap_or_else(|_| git_dir.to_path_buf())
        .to_string_lossy()
        .to_string()
}
