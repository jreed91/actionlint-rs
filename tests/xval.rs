//! Cross-validation gate: track where the first-party (default) and SchemaStore schemas
//! disagree on real workflows.
//!
//! For every `corpus/xval/*.yml`, validate the parsed workflow under BOTH schemas. A
//! "disagreement" is a file that one schema accepts and the other rejects. The gate compares
//! the observed disagreement set against a checked-in baseline (`corpus/xval/BASELINE.txt`)
//! and fails on any drift — a NEW disagreement (schemas diverged further) or a RESOLVED one
//! (baseline is now stale). This turns schema drift into a reviewable signal without blocking
//! on the legitimate, already-understood differences between the two schemas.
//!
//! Update the baseline (with a documented reason) when a change is intended.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use actionlint_rs::{schema, yaml};

fn xval_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/xval")
}

fn workflow_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(xval_dir())
        .expect("corpus/xval must exist")
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

/// Read the baseline set of accepted-disagreement filenames (ignoring comments/blanks).
fn baseline() -> BTreeSet<String> {
    let text = fs::read_to_string(xval_dir().join("BASELINE.txt")).expect("BASELINE.txt exists");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn cross_validation_disagreements_match_baseline() {
    let first_party = schema::build_first_party_validator().expect("first-party validator");
    let schemastore = schema::build_schemastore_validator().expect("schemastore validator");

    let files = workflow_files();
    assert!(
        files.len() >= 10,
        "cross-validation corpus is too small ({}); scrape more real workflows",
        files.len()
    );

    let mut observed: BTreeSet<String> = BTreeSet::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = fs::read_to_string(path).unwrap();
        // A YAML parse failure is itself a disagreement-independent problem; surface it.
        let parsed = yaml::parse(&src)
            .unwrap_or_else(|e| panic!("xval corpus file {name} failed to parse: {e}"));
        let fp_ok = first_party.is_valid(&parsed.json);
        let ss_ok = schemastore.is_valid(&parsed.json);
        if fp_ok != ss_ok {
            observed.insert(name);
        }
    }

    let baseline = baseline();

    let new_disagreements: Vec<&String> = observed.difference(&baseline).collect();
    let resolved: Vec<&String> = baseline.difference(&observed).collect();

    assert!(
        new_disagreements.is_empty() && resolved.is_empty(),
        "cross-validation drift detected — update corpus/xval/BASELINE.txt (with a reason) \
         if this is intended.\n  NEW disagreements (schemas diverged): {new_disagreements:?}\n  \
         RESOLVED (baseline now stale): {resolved:?}\n  observed={observed:?}\n  baseline={baseline:?}"
    );
}

#[test]
fn every_baseline_entry_names_a_real_file() {
    // A baseline entry for a file that no longer exists is stale bookkeeping.
    let existing: BTreeSet<String> = workflow_files()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    for entry in baseline() {
        assert!(
            existing.contains(&entry),
            "BASELINE.txt names `{entry}`, which is not in corpus/xval/"
        );
    }
}
