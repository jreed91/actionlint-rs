//! CLI entry point — the primitive interface (the GitHub Action is a thin wrapper).
//!
//! Usage:
//!   actionlint-rs [FILES...]     lint the given workflow files
//!   actionlint-rs -             lint stdin
//!   actionlint-rs               lint all .github/workflows/*.{yml,yaml} under the cwd
//!
//! Exit codes: 0 = no problems, 1 = lint problems found, 2 = usage/IO error.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

use actionlint_rs::{diagnostic::Diagnostic, lint, sarif, schema};
use clap::ValueEnum;

#[derive(Parser, Debug)]
#[command(
    name = "actionlint-rs",
    about = "Schema-driven GitHub Actions workflow linter (structural checks, v1)"
)]
struct Cli {
    /// Workflow files to lint. Use `-` for stdin. If omitted, discovers
    /// `.github/workflows/*.{yml,yaml}` under the current directory.
    files: Vec<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Human)]
    format: Format,

    /// Structural schema source: the community SchemaStore schema (default), or GitHub's
    /// first-party schema transpiled to JSON Schema (the option-B path).
    #[arg(long, value_enum, default_value_t = SchemaArg::Schemastore)]
    schema: SchemaArg,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum SchemaArg {
    /// Community SchemaStore schema.
    Schemastore,
    /// GitHub's first-party workflow-v1.0 DSL, transpiled to JSON Schema.
    FirstParty,
}

impl From<SchemaArg> for schema::SchemaSource {
    fn from(a: SchemaArg) -> Self {
        match a {
            SchemaArg::Schemastore => schema::SchemaSource::SchemaStore,
            SchemaArg::FirstParty => schema::SchemaSource::FirstParty,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Human-readable `file:line:col: message`.
    Human,
    /// SARIF 2.1.0 JSON, for GitHub code-scanning inline annotations.
    Sarif,
}

fn main() -> ExitCode {
    match run() {
        Ok(problems_found) => {
            if problems_found {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("actionlint-rs: {e:#}");
            ExitCode::from(2)
        }
    }
}

/// Returns `Ok(true)` if any lint problems were reported.
fn run() -> Result<bool> {
    // Drop empty-string arguments before parsing. The Docker action passes
    // `${{ inputs.files }}`, which becomes a single empty arg `""` when the (optional)
    // `files` input is unset — clap rejects an empty positional value, so filtering it here
    // makes the default action invocation fall through to workflow discovery.
    let args = std::env::args_os().filter(|a| !a.is_empty());
    let cli = Cli::parse_from(args);
    let validator = schema::build_validator_for(cli.schema.into())?;

    let mut all: Vec<Diagnostic> = Vec::new();

    if cli.files.len() == 1 && cli.files[0].as_os_str() == "-" {
        let mut source = String::new();
        std::io::stdin()
            .read_to_string(&mut source)
            .context("reading stdin")?;
        all.extend(lint::lint_stdin(&validator, &source)?);
    } else {
        let targets = if cli.files.is_empty() {
            discover_workflows(Path::new("."))?
        } else {
            cli.files.clone()
        };

        if targets.is_empty() {
            eprintln!("actionlint-rs: no workflow files found under .github/workflows/");
            // In SARIF mode still emit a valid (empty) document so an upload step has
            // something well-formed to consume.
            if cli.format == Format::Sarif {
                println!("{}", sarif::to_sarif(&[]));
            }
            return Ok(false);
        }

        for file in &targets {
            all.extend(lint::lint_file(&validator, file)?);
        }
    }

    match cli.format {
        Format::Human => {
            for d in &all {
                println!("{d}");
            }
        }
        Format::Sarif => {
            println!("{}", sarif::to_sarif(&all));
        }
    }
    Ok(!all.is_empty())
}

/// Find `.github/workflows/*.{yml,yaml}` under `root` (non-recursive within that dir).
fn discover_workflows(root: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join(".github").join("workflows");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
    {
        let path = entry?.path();
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml") | Some("yaml")
        ) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}
