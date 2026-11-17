//! SigmaOS-apt-mark — package mark-state database.
//!
//! Inspired by Debian/Ubuntu `apt-mark`: tracks per-package install marks
//! (`auto`, `manual`, `hold`) so the package manager can distinguish
//! user-requested packages from automatically-pulled dependencies, and can
//! hold packages against upgrade/removal.
//!
//! The database is a plain-text file (one `state<TAB>package` per line) so it
//! is human-readable, diff-friendly, and trivially portable — the same
//! philosophy as `/var/lib/apt/extended_states`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Errors raised by this component.
#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    /// The requested operation is not supported by this component yet.
    #[error("unsupported operation: {0}")]
    Unsupported(&'static str),
    /// Underlying I/O failure while reading or writing the mark database.
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    /// A line in the database file could not be parsed.
    #[error("malformed database line: {0}")]
    Malformed(String),
}

/// Result alias for component operations.
pub type ComponentResult<T> = Result<T, ComponentError>;

/// Install mark applied to a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    /// Installed automatically as a dependency of another package.
    Auto,
    /// Explicitly requested by the user.
    Manual,
    /// Pinned: excluded from automatic upgrade/removal.
    Hold,
}

impl Mark {
    fn as_str(self) -> &'static str {
        match self {
            Mark::Auto => "auto",
            Mark::Manual => "manual",
            Mark::Hold => "hold",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Mark::Auto),
            "manual" => Some(Mark::Manual),
            "hold" => Some(Mark::Hold),
            _ => None,
        }
    }
}

impl fmt::Display for Mark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// In-memory + on-disk package mark database.
#[derive(Debug, Default)]
pub struct Marks {
    inner: BTreeMap<String, Mark>,
    path: Option<PathBuf>,
}

impl Marks {
    /// Create an in-memory database not backed by a file.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open (or create) a database persisted at `path`.
    pub fn open(path: impl AsRef<Path>) -> ComponentResult<Self> {
        let path = path.as_ref().to_path_buf();
        let mut db = Self {
            inner: BTreeMap::new(),
            path: Some(path.clone()),
        };
        if path.exists() {
            db.load()?;
        }
        Ok(db)
    }

    /// Apply `mark` to `package`, then persist if file-backed.
    pub fn set(&mut self, package: &str, mark: Mark) -> ComponentResult<()> {
        self.inner.insert(package.to_string(), mark);
        self.save()
    }

    /// Convenience: mark a package `manual` (user-requested).
    pub fn mark_manual(&mut self, package: &str) -> ComponentResult<()> {
        self.set(package, Mark::Manual)
    }

    /// Convenience: mark a package `auto` (dependency).
    pub fn mark_auto(&mut self, package: &str) -> ComponentResult<()> {
        self.set(package, Mark::Auto)
    }

    /// Convenience: mark a package `hold`.
    pub fn mark_hold(&mut self, package: &str) -> ComponentResult<()> {
        self.set(package, Mark::Hold)
    }

    /// Remove the mark for `package` (e.g. `apt-mark unhold`).
    pub fn clear(&mut self, package: &str) -> ComponentResult<()> {
        self.inner.remove(package);
        self.save()
    }

    /// The mark for `package`, or `None` if unmarked.
    pub fn get(&self, package: &str) -> Option<Mark> {
        self.inner.get(package).copied()
    }

    /// Whether `package` is held.
    pub fn is_held(&self, package: &str) -> bool {
        self.get(package) == Some(Mark::Hold)
    }

    /// Packages carrying `mark`, sorted by name.
    pub fn list(&self, mark: Mark) -> Vec<String> {
        self.inner
            .iter()
            .filter(|(_, m)| **m == mark)
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// All marked packages as `(name, mark)` pairs.
    pub fn all(&self) -> Vec<(String, Mark)> {
        self.inner
            .iter()
            .map(|(p, m)| (p.clone(), *m))
            .collect()
    }

    fn load(&mut self) -> ComponentResult<()> {
        let text = fs::read_to_string(self.path.as_ref().unwrap())?;
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(2, '\t');
            let state = parts.next().unwrap_or("");
            let pkg = parts.next().ok_or_else(|| {
                ComponentError::Malformed(format!("line {}: missing package", n + 1))
            })?;
            let mark = Mark::parse(state).ok_or_else(|| {
                ComponentError::Malformed(format!("line {}: bad state {:?}", n + 1, state))
            })?;
            self.inner.insert(pkg.to_string(), mark);
        }
        Ok(())
    }

    fn save(&self) -> ComponentResult<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let mut out = String::new();
        for (pkg, mark) in &self.inner {
            out.push_str(mark.as_str());
            out.push('\t');
            out.push_str(pkg);
            out.push('\n');
        }
        fs::write(path, out)?;
        Ok(())
    }
}

/// A handle to an initialised component instance.
#[derive(Debug, Default)]
pub struct Component {
    marks: Marks,
}

impl Component {
    /// Create a new, uninitialised component handle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialise with an in-memory mark database.
    pub fn init(&mut self) -> ComponentResult<()> {
        self.marks = Marks::new();
        Ok(())
    }

    /// Initialise backed by `path`.
    pub fn init_at(&mut self, path: impl AsRef<Path>) -> ComponentResult<()> {
        self.marks = Marks::open(path)?;
        Ok(())
    }

    /// Returns `true` once initialised.
    pub fn is_ready(&self) -> bool {
        true
    }

    /// Access the underlying mark database.
    pub fn marks(&self) -> &Marks {
        &self.marks
    }

    /// Mutable access to the underlying mark database.
    pub fn marks_mut(&mut self) -> &mut Marks {
        &mut self.marks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_marks() {
        let mut db = Marks::new();
        db.mark_manual("bash").unwrap();
        db.mark_auto("libc6").unwrap();
        db.mark_hold("linux-image").unwrap();
        assert_eq!(db.get("bash"), Some(Mark::Manual));
        assert_eq!(db.get("libc6"), Some(Mark::Auto));
        assert!(db.is_held("linux-image"));
        assert_eq!(db.get("nonexistent"), None);
    }

    #[test]
    fn list_filters_by_mark() {
        let mut db = Marks::new();
        db.mark_auto("a").unwrap();
        db.mark_auto("b").unwrap();
        db.mark_manual("c").unwrap();
        assert_eq!(db.list(Mark::Auto), vec!["a", "b"]);
        assert_eq!(db.list(Mark::Manual), vec!["c"]);
        assert!(db.list(Mark::Hold).is_empty());
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = std::env::temp_dir().join(format!("aptmark-{}", std::process::id()));
        let path = dir.join("extended_states");
        {
            let mut db = Marks::open(&path).unwrap();
            db.mark_manual("vim").unwrap();
            db.mark_hold("kernel").unwrap();
        }
        let db = Marks::open(&path).unwrap();
        assert_eq!(db.get("vim"), Some(Mark::Manual));
        assert_eq!(db.get("kernel"), Some(Mark::Hold));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_line_errors() {
        let dir = std::env::temp_dir().join(format!("aptmark-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db");
        fs::write(&path, "bogus-state\tpkg\n").unwrap();
        assert!(Marks::open(&path).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_removes_mark() {
        let mut db = Marks::new();
        db.mark_hold("x").unwrap();
        assert!(db.is_held("x"));
        db.clear("x").unwrap();
        assert_eq!(db.get("x"), None);
    }
}
