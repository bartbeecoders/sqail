//! Local persistence in the config directory: query history (JSON lines),
//! saved snippets, and the open-tabs workspace; plus the file helpers every
//! config file uses (atomic writes, setting a damaged file aside, the
//! single-instance lock).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::config_dir;

/// Most history entries kept; the file is compacted past 120% of this.
pub const HISTORY_LIMIT: usize = 5000;

// ------------------------------------------------------------ file helpers --

/// Replace `path` with `bytes` so that a crash or a concurrent writer never
/// leaves a torn file: write a temporary file with a unique name in the same
/// directory, flush it to disk, then rename it over `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_mode(path, bytes, false)
}

/// [`write_atomic`] for secrets: on Unix the file is created owner-only
/// (0600) from the start, so the content is never readable by others.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_mode(path, bytes, true)
}

fn write_atomic_mode(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private; // the per-user profile directory is already private
    let written = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written?;
    // Make the rename itself durable (Unix; Windows has no directory handles
    // to sync this way, and NTFS journals the rename).
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Move a file that could not be read aside, to `<name>.bad` (or
/// `<name>.<n>.bad` when that exists, so an earlier one is never
/// overwritten). Returns the new path.
pub fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    for n in 0..1000 {
        let bad = if n == 0 {
            path.with_file_name(format!("{name}.bad"))
        } else {
            path.with_file_name(format!("{name}.{n}.bad"))
        };
        if !bad.exists() {
            std::fs::rename(path, &bad)?;
            return Ok(bad);
        }
    }
    Err(io::Error::other("too many .bad files"))
}

/// Read and parse a config file. `Ok(None)` when it does not exist. One
/// that cannot be read or parsed is moved aside with [`set_aside`] (so the
/// next save does not destroy it) and reported as `Err(message)`.
pub fn load_file<T>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<T, String>,
) -> Result<Option<T>, String> {
    let error = match std::fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(v) => return Ok(Some(v)),
            Err(e) => e,
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => e.to_string(),
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    tracing::warn!(file = %path.display(), error = %error, "could not read a config file");
    Err(match set_aside(path) {
        Ok(bad) => format!(
            "{name} could not be read ({error}); it was kept as {}",
            bad.display()
        ),
        Err(e) => format!("{name} could not be read ({error}) or moved aside ({e})"),
    })
}

/// One running sqail per config directory: its settings and workspace are
/// whole-file state, and two instances would overwrite each other's. Held
/// for the life of the process; the OS releases it if the process dies.
pub struct InstanceLock {
    _file: File,
}

/// Why [`InstanceLock::acquire`] failed.
pub enum LockError {
    /// Another sqail holds it.
    Held,
    /// Locking is not possible here (file system without locks, no access).
    Unavailable(io::Error),
}

impl InstanceLock {
    pub fn acquire(dir: &Path) -> Result<Self, LockError> {
        std::fs::create_dir_all(dir).map_err(LockError::Unavailable)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("sqail.lock"))
            .map_err(LockError::Unavailable)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(LockError::Held),
            Err(std::fs::TryLockError::Error(e)) => Err(LockError::Unavailable(e)),
        }
    }
}

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
    /// Changes whenever `entries` does (views cache on it).
    revision: u64,
}

impl History {
    pub fn load() -> Self {
        Self::load_from(config_dir().join("history.jsonl"))
    }

    fn load_from(path: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&path)
            .map(|text| {
                text.lines()
                    .filter_map(|l| serde_json::from_str::<HistoryEntry>(l).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut h = Self {
            path,
            entries,
            revision: 0,
        };
        h.trim();
        h
    }

    fn trim(&mut self) {
        if self.entries.len() > HISTORY_LIMIT {
            let drop = self.entries.len() - HISTORY_LIMIT;
            self.entries.drain(..drop);
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn push(&mut self, e: HistoryEntry) {
        self.revision += 1;
        let line = serde_json::to_string(&e).unwrap_or_default();
        self.entries.push(e);
        let res = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(config_dir())?;
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            writeln!(f, "{line}")?;
            drop(f);
            if self.entries.len() > HISTORY_LIMIT * 6 / 5 {
                // Compact from the file, not from memory, so entries another
                // sqail appended meanwhile are kept.
                let mut fresh = Self::load_from(self.path.clone());
                fresh.trim();
                let body: String = fresh
                    .entries
                    .iter()
                    .filter_map(|e| serde_json::to_string(e).ok())
                    .map(|l| l + "\n")
                    .collect();
                write_atomic(&self.path, body.as_bytes())?;
                self.entries = fresh.entries;
            }
            Ok(())
        })();
        if let Err(e) = res {
            tracing::warn!(error = %e, "could not write history");
        }
    }

    pub fn clear(&mut self) {
        self.revision += 1;
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

    /// The snippets, and a message when `snippets.json` was damaged (it is
    /// then kept as `snippets.json.bad`).
    pub fn load() -> (Self, Option<String>) {
        match load_file(&Self::path(), |t| {
            serde_json::from_str(t).map_err(|e| e.to_string())
        }) {
            Ok(items) => (
                Self {
                    items: items.unwrap_or_default(),
                },
                None,
            ),
            Err(e) => (Self { items: Vec::new() }, Some(e)),
        }
    }

    pub fn save(&self) {
        let body = serde_json::to_string_pretty(&self.items).unwrap_or_default();
        if let Err(e) = write_atomic(&Self::path(), body.as_bytes()) {
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
}

impl Workspace {
    pub fn path() -> PathBuf {
        config_dir().join("workspace.json")
    }

    /// The saved tabs (`Ok(None)` on first start). A damaged
    /// `workspace.json` is kept as `workspace.json.bad` and reported.
    pub fn load() -> Result<Option<Self>, String> {
        load_file(&Self::path(), |t| {
            serde_json::from_str(t).map_err(|e| e.to_string())
        })
    }

    pub fn save(&self) {
        let body = serde_json::to_string(self).unwrap_or_default();
        if let Err(e) = write_atomic(&Self::path(), body.as_bytes()) {
            tracing::warn!(error = %e, "could not save workspace");
        }
    }
}

/// The open tabs as autosave writes them: the same JSON as [`Workspace`],
/// but the texts are shared with the previous snapshot, so a tab that did
/// not change is neither copied nor compared beyond a pointer check.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WorkspaceSnapshot {
    pub tabs: Vec<TabSnapshot>,
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TabSnapshot {
    pub title: String,
    pub path: Option<PathBuf>,
    #[serde(serialize_with = "serialize_text")]
    pub text: Arc<str>,
    pub connection: Option<Uuid>,
    pub cursor: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
}

fn serialize_text<S: serde::Serializer>(text: &Arc<str>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(text)
}

/// Writes workspace snapshots in order: snapshot `seq` is written only if
/// no later one was written first (writes run on worker threads).
#[derive(Clone, Default)]
pub struct WorkspaceWriter {
    written: Arc<Mutex<u64>>,
}

impl WorkspaceWriter {
    /// Serialise and write `ws` atomically to `path` (blocking; call it off
    /// the UI thread except when quitting).
    pub fn write(&self, path: &Path, ws: &WorkspaceSnapshot, seq: u64) {
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if *written >= seq {
            return; // a newer snapshot is on disk already
        }
        let body = serde_json::to_vec(ws).unwrap_or_default();
        match write_atomic(path, &body) {
            Ok(()) => *written = seq,
            Err(e) => tracing::warn!(error = %e, "could not save workspace"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_snapshots_read_back_as_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let snap = |text: &str| WorkspaceSnapshot {
            tabs: vec![TabSnapshot {
                title: "q".into(),
                path: None,
                text: Arc::from(text),
                connection: None,
                cursor: 3,
                pinned: true,
            }],
            active: 0,
        };
        let w = WorkspaceWriter::default();
        w.write(&path, &snap("SELECT 2"), 2);
        // An older snapshot that arrives late does not overwrite it.
        w.write(&path, &snap("SELECT 1"), 1);
        let ws: Workspace = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(ws.tabs[0].text, "SELECT 2");
        assert_eq!(ws.tabs[0].cursor, 3);
        assert!(ws.tabs[0].pinned);
    }

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

    fn files(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn atomic_writes_replace_the_file_and_leave_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.toml");
        write_atomic(&path, b"one").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(files(&dir.path().join("sub")), ["settings.toml"]);
        // A failed write leaves the old content and no temp file behind.
        let target_is_dir = dir.path().join("sub").join("adir");
        std::fs::create_dir(&target_is_dir).unwrap();
        std::fs::write(target_is_dir.join("x"), "x").unwrap();
        assert!(write_atomic(&target_is_dir, b"nope").is_err());
        assert_eq!(files(&dir.path().join("sub")), ["adir", "settings.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only_from_the_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.toml");
        // Even replacing a file that others could read.
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, b"secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret");
    }

    #[test]
    fn damaged_files_are_set_aside_without_overwriting_earlier_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let parse = |t: &str| serde_json::from_str::<Workspace>(t).map_err(|e| e.to_string());

        assert_eq!(load_file(&path, parse), Ok(None), "missing is not an error");

        std::fs::write(&path, r#"{"tabs":[],"active":0}"#).unwrap();
        assert_eq!(load_file(&path, parse), Ok(Some(Workspace::default())));

        std::fs::write(&path, r#"{"tabs":[{"title":"x""#).unwrap();
        let err = load_file(&path, parse).unwrap_err();
        assert!(err.contains("workspace.json could not be read"), "{err}");
        assert!(err.contains("workspace.json.bad"), "{err}");
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("workspace.json.bad")).unwrap(),
            r#"{"tabs":[{"title":"x""#
        );

        std::fs::write(&path, "second").unwrap();
        let err = load_file(&path, parse).unwrap_err();
        assert!(err.contains("workspace.json.1.bad"), "{err}");
        assert_eq!(
            files(dir.path()),
            ["workspace.json.1.bad", "workspace.json.bad"]
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("workspace.json.1.bad")).unwrap(),
            "second"
        );
    }

    #[test]
    fn a_second_instance_cannot_take_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let Ok(first) = InstanceLock::acquire(dir.path()) else {
            panic!("first lock")
        };
        assert!(matches!(
            InstanceLock::acquire(dir.path()),
            Err(LockError::Held)
        ));
        // Another config directory is another instance.
        let other = tempfile::tempdir().unwrap();
        assert!(InstanceLock::acquire(other.path()).is_ok());
        drop(first);
        assert!(InstanceLock::acquire(dir.path()).is_ok());
    }
}
