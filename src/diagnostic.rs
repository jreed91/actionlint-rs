//! Diagnostics: what we report to the user, and how we render them.
//!
//! v1 output is human-readable `file:line:col: message` only. SARIF/JSON is deferred
//! (see docs/ROADMAP.md).

use std::fmt;
use std::path::{Path, PathBuf};

/// A 1-based source position. Column is in characters, matching editor conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

impl Position {
    pub fn new(line: usize, col: usize) -> Self {
        Position { line, col }
    }
}

/// A single lint finding, anchored to a source position.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub pos: Position,
    /// The JSON Pointer into the document that the schema error referenced
    /// (e.g. `/jobs/build/runs-on`). Kept for debugging and future SARIF output.
    pub pointer: String,
    pub message: String,
}

impl Diagnostic {
    pub fn new(
        file: impl Into<PathBuf>,
        pos: Position,
        pointer: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Diagnostic {
            file: file.into(),
            pos,
            pointer: pointer.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Diagnostic {
    /// Renders as `file:line:col: message`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            display_path(&self.file),
            self.pos.line,
            self.pos.col,
            self.message
        )
    }
}

fn display_path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_new_sets_fields() {
        let p = Position::new(3, 7);
        assert_eq!(p.line, 3);
        assert_eq!(p.col, 7);
    }

    #[test]
    fn renders_as_file_line_col_message() {
        let d = Diagnostic::new(
            "wf.yml",
            Position::new(4, 4),
            "/jobs/build/runs-on",
            "runs-on is required",
        );
        assert_eq!(d.to_string(), "wf.yml:4:4: runs-on is required");
        // The pointer is retained for future SARIF output.
        assert_eq!(d.pointer, "/jobs/build/runs-on");
    }

    #[test]
    fn display_path_handles_nested_paths() {
        let d = Diagnostic::new(
            PathBuf::from(".github/workflows/ci.yml"),
            Position::new(1, 1),
            "",
            "root problem",
        );
        assert_eq!(d.to_string(), ".github/workflows/ci.yml:1:1: root problem");
    }
}
