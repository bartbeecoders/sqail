//! Local persistence in the config directory: query history (JSON lines),
//! saved snippets, and the open-tabs workspace.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::config_dir;

/// Most history entries kept; the file is compacted past 120% of this.
pub const HISTORY_LIMIT: usize = 5000;

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix seconds.
    pub at: u64,
    pub connection: Option<Uuid>,
    pub connection_name: String,
    pub sql: String,
    pub duration_ms: u64,
    pub rows: u64,
    pub outcome: Outcome,
}

pub struct History {
    path: PathBuf,
    /// Oldest first.
    pub entries: Vec<HistoryEntry>,
}

impl History {
    pub fn load() -> Self {
        let path = config_dir().join("history.jsonl");
        let entries = std::fs::read_to_string(&path)
            .map(|text| {
                text.lines()
                    .filter_map(|l| serde_json::from_str::<HistoryEntry>(l).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut h = Self { path, entries };
        h.trim();
        h
    }

    fn trim(&mut self) {
        if self.entries.len() > HISTORY_LIMIT {
            let drop = self.entries.len() - HISTORY_LIMIT;
            self.entries.drain(..drop);
        }
    }

    pub fn push(&mut self, e: HistoryEntry) {
        let line = serde_json::to_string(&e).unwrap_or_default();
        self.entries.push(e);
        let res = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(config_dir())?;
            if self.entries.len() > HISTORY_LIMIT * 6 / 5 {
                self.trim();
                let body: String = self
                    .entries
                    .iter()
                    .filter_map(|e| serde_json::to_string(e).ok())
                    .map(|l| l + "\n")
                    .collect();
                std::fs::write(&self.path, body)
            } else {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?;
                writeln!(f, "{line}")
            }
        })();
        if let Err(e) = res {
            tracing::warn!(error = %e, "could not write history");
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: Uuid,
    pub name: String,
    pub sql: String,
}

pub struct Snippets {
    pub items: Vec<Snippet>,
}

impl Snippets {
    fn path() -> PathBuf {
        config_dir().join("snippets.json")
    }

    pub fn load() -> Self {
        let items = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { items }
    }

    pub fn save(&self) {
        let res = std::fs::create_dir_all(config_dir()).and_then(|_| {
            std::fs::write(
                Self::path(),
                serde_json::to_string_pretty(&self.items).unwrap_or_default(),
            )
        });
        if let Err(e) = res {
            tracing::warn!(error = %e, "could not save snippets");
        }
    }

    pub fn add(&mut self, name: String, sql: String) {
        self.items.push(Snippet {
            id: Uuid::new_v4(),
            name,
            sql,
        });
        self.items.sort_by_key(|s| s.name.to_lowercase());
        self.save();
    }

    pub fn remove(&mut self, id: Uuid) {
        self.items.retain(|s| s.id != id);
        self.save();
    }
}

/// Open tabs, restored on start.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub title: String,
    pub path: Option<PathBuf>,
    pub text: String,
    pub connection: Option<Uuid>,
    /// Character offset of the cursor.
    pub cursor: usize,
}

impl Workspace {
    fn path() -> PathBuf {
        config_dir().join("workspace.json")
    }

    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) {
        let res = std::fs::create_dir_all(config_dir()).and_then(|_| {
            // Write-then-rename so a crash never leaves a torn file.
            let tmp = Self::path().with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_string(self).unwrap_or_default())?;
            std::fs::rename(&tmp, Self::path())
        });
        if let Err(e) = res {
            tracing::warn!(error = %e, "could not save workspace");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_entries_round_trip_as_json_lines() {
        let e = HistoryEntry {
            at: 1,
            connection: None,
            connection_name: "pg".into(),
            sql: "SELECT 1\nFROM t".into(),
            duration_ms: 5,
            rows: 1,
            outcome: Outcome::Failed,
        };
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains('\n'), "one entry per line");
        assert_eq!(serde_json::from_str::<HistoryEntry>(&line).unwrap(), e);
    }
}
