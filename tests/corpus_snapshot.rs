//! Snapshot gate for the golden corpus.
//!
//! The `scripts/run-corpus.sh` gate only asserts a coarse invariant (good files → zero
//! diagnostics; bad files → ≥1). This test pins the **exact** linter output for every
//! `corpus/good` and `corpus/bad` file as an `insta` snapshot, so any change to *what* is
//! reported — a new finding, a reworded message, a moved anchor — surfaces as a reviewable
//! snapshot diff rather than passing silently. (This is the corpus README's "snapshot-based
//! expected output per file" item.)
//!
//! Update snapshots intentionally with `cargo insta review` (or `INSTA_UPDATE=always`).

use std::fs;
use std::path::{Path, PathBuf};

use actionlint_rs::{lint, run_lint::RunLinters, schema};

/// Lint one file the way the corpus gate does (default schema, no external tools) and render
/// its diagnostics as a stable, path-independent string for snapshotting.
fn render(path: &Path) -> String {
    let validator = schema::build_validator().expect("validator builds");
    // `lint_file_with` reads from disk so path-relative resolution (reusable callees) works.
    let diags = lint::lint_file_with(&validator, path, &RunLinters::none())
        .unwrap_or_else(|e| panic!("lint {} failed: {e:#}", path.display()));
    if diags.is_empty() {
        return "(no diagnostics)".to_string();
    }
    // Diagnostics are already sorted by the linter; render without the absolute file path so
    // snapshots are stable across checkouts.
    diags
        .iter()
        .map(|d| format!("{}:{} [{}] {}", d.pos.line, d.pos.col, d.rule_id, d.message))
        .collect::<Vec<_>>()
        .join("\n")
}

fn files_in(subdir: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus").join(subdir);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("yml") | Some("yaml")
            )
        })
        .collect();
    files.sort();
    files
}

/// Snapshot every file in a corpus subdirectory, named `<subdir>__<file-stem>`.
fn snapshot_dir(subdir: &str) {
    let files = files_in(subdir);
    assert!(!files.is_empty(), "corpus/{subdir} should contain workflow files");
    for path in files {
        let stem = path.file_stem().unwrap().to_string_lossy().replace('.', "_");
        let name = format!("{subdir}__{stem}");
        insta::assert_snapshot!(name, render(&path));
    }
}

#[test]
fn good_corpus_snapshots() {
    snapshot_dir("good");
}

#[test]
fn bad_corpus_snapshots() {
    snapshot_dir("bad");
}
