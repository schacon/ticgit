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
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// Map of canonicalised git-dir path → currently checked-out ticket UUID.
    pub current: HashMap<String, Uuid>,
    /// Map of canonicalised git-dir path → last-used list filters.
    #[serde(default)]
    pub last_filters: HashMap<String, SavedView>,
    /// Map of canonicalised git-dir path → named saved views.
    #[serde(default)]
    pub views: HashMap<String, HashMap<String, SavedView>>,
    /// Map of canonicalised git-dir path → per-user project UI settings.
    #[serde(default)]
    pub project_settings: HashMap<String, ProjectSettings>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_width_percent: Option<u16>,
}

/// A saved set of list filter parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedView {
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub tag_match_all: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub only_tagged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub all: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub subissues: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub limit: usize,
}

fn is_false(v: &bool) -> bool {
    !v
}

fn is_true(v: &bool) -> bool {
    *v
}

fn default_true() -> bool {
    true
}

fn is_zero(v: &usize) -> bool {
    *v == 0
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

    pub fn set_last_filters(&mut self, git_dir: &Path, view: SavedView) {
        self.last_filters.insert(key_for(git_dir), view);
    }

    pub fn last_filters_for(&self, git_dir: &Path) -> Option<&SavedView> {
        self.last_filters.get(&key_for(git_dir))
    }

    pub fn save_view(&mut self, git_dir: &Path, name: &str, mut view: SavedView) {
        if view.created_at.is_none() {
            view.created_at = Some(OffsetDateTime::now_utc());
        }
        self.views
            .entry(key_for(git_dir))
            .or_default()
            .insert(name.to_string(), view);
    }

    pub fn load_view(&self, git_dir: &Path, name: &str) -> Option<&SavedView> {
        self.views.get(&key_for(git_dir))?.get(name)
    }

    pub fn delete_view(&mut self, git_dir: &Path, name: &str) -> bool {
        if let Some(repo_views) = self.views.get_mut(&key_for(git_dir)) {
            return repo_views.remove(name).is_some();
        }
        false
    }

    pub fn list_views(&self, git_dir: &Path) -> Vec<(String, SavedView)> {
        self.views
            .get(&key_for(git_dir))
            .map(|m| {
                let mut v: Vec<_> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                v.sort_by(|a, b| {
                    b.1.created_at
                        .cmp(&a.1.created_at)
                        .then_with(|| a.0.cmp(&b.0))
                });
                v
            })
            .unwrap_or_default()
    }

    pub fn project_settings_for(&self, git_dir: &Path) -> ProjectSettings {
        self.project_settings
            .get(&key_for(git_dir))
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_project_settings(&mut self, git_dir: &Path, settings: ProjectSettings) {
        self.project_settings.insert(key_for(git_dir), settings);
    }
}

fn key_for(git_dir: &Path) -> String {
    git_dir
        .canonicalize()
        .unwrap_or_else(|_| git_dir.to_path_buf())
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_views_orders_newest_first() {
        let mut state = State::default();
        let git_dir = Path::new("/tmp/ticgit-session-state-test");
        let older = OffsetDateTime::from_unix_timestamp(1).unwrap();
        let newer = OffsetDateTime::from_unix_timestamp(2).unwrap();

        state.save_view(
            git_dir,
            "older",
            SavedView {
                created_at: Some(older),
                ..Default::default()
            },
        );
        state.save_view(
            git_dir,
            "newer",
            SavedView {
                created_at: Some(newer),
                ..Default::default()
            },
        );

        let names = state
            .list_views(git_dir)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["newer", "older"]);
    }
}
