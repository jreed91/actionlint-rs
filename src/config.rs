//! `.github/actionlint.yaml` configuration file.
//!
//! An actionlint-compatible config, parsed from `.github/actionlint.yaml` (or `.yml`) at the
//! repo root. Supported keys are a deliberate subset of actionlint's:
//!
//! ```yaml
//! self-hosted-runner:
//!   # Custom `runs-on` labels this repo's self-hosted runners advertise. Declaring them
//!   # here suppresses the "unknown runner label" diagnostic for those labels.
//!   labels:
//!     - linux-arm64-16core
//!     - macos-m2
//!
//! # Suppress diagnostics by message regex — like repeatable `--ignore`, but committed to the
//! # repo so the whole team shares the suppression set.
//! ignore:
//!   - 'unknown runner label "custom-.*"'
//! ```
//!
//! Unknown top-level keys are ignored (forward-compatible with actionlint configs that use
//! keys we don't implement yet, e.g. `config-variables`, `paths`). A malformed config file
//! is a hard error (exit 2) so a typo doesn't silently disable configured behavior.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::yaml;

/// Parsed configuration. All fields are optional; an absent config yields [`Config::default`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Config {
    /// Custom self-hosted runner labels declared as known (for `runs-on` label checking).
    pub self_hosted_runner_labels: Vec<String>,
    /// Message-regex ignore patterns, composed with any `--ignore` flags.
    pub ignore: Vec<String>,
    /// Whether to run the `runs-on` runner-label check. Off by default (see `runner` module):
    /// larger-runner/self-hosted labels are unbounded, so the check needs opt-in + config to
    /// avoid false positives. Set from the `--check-runner-labels` CLI flag; not read from the
    /// config file (it's a run-mode toggle, not persisted repo state).
    pub check_runner_labels: bool,
}

impl Config {
    /// Parse a config from YAML source text.
    ///
    /// Returns an error for malformed YAML or a mistyped known key (e.g. `labels` not a list),
    /// so misconfiguration surfaces loudly rather than silently doing nothing.
    pub fn parse(source: &str) -> Result<Self> {
        // An empty/whitespace-only file (or one that is just comments) is the default config.
        if source.trim().is_empty() {
            return Ok(Config::default());
        }
        let value = yaml::parse_value(source).context("parsing config file")?;
        // A file that parses to null (e.g. only comments) is also the default config.
        if value.is_null() {
            return Ok(Config::default());
        }
        let map = value
            .as_object()
            .ok_or_else(|| anyhow!("config root must be a mapping"))?;

        let mut cfg = Config::default();

        if let Some(shr) = map.get("self-hosted-runner") {
            let shr = shr
                .as_object()
                .ok_or_else(|| anyhow!("`self-hosted-runner` must be a mapping"))?;
            if let Some(labels) = shr.get("labels") {
                cfg.self_hosted_runner_labels = string_list(labels, "self-hosted-runner.labels")?;
            }
        }

        if let Some(ignore) = map.get("ignore") {
            cfg.ignore = string_list(ignore, "ignore")?;
        }

        Ok(cfg)
    }

    /// Load config from a specific file path (must exist and be readable).
    pub fn from_path(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("could not read config {}", path.display()))?;
        Config::parse(&source).with_context(|| format!("in config {}", path.display()))
    }

    /// Discover and load the config under `root`, trying `.github/actionlint.yaml` then
    /// `.yml`. Returns the default config when no file exists (config is optional).
    pub fn discover(root: &Path) -> Result<Self> {
        match Self::config_path(root) {
            Some(path) => Config::from_path(&path),
            None => Ok(Config::default()),
        }
    }

    /// The path of the config file under `root`, if one exists.
    pub fn config_path(root: &Path) -> Option<PathBuf> {
        let base = root.join(".github");
        for name in ["actionlint.yaml", "actionlint.yml"] {
            let candidate = base.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}

/// Coerce a JSON value into a `Vec<String>`, requiring a sequence of scalars. `field` names
/// the config key for error messages.
fn string_list(value: &Value, field: &str) -> Result<Vec<String>> {
    let arr = value
        .as_array()
        .ok_or_else(|| anyhow!("`{field}` must be a list"))?;
    arr.iter()
        .map(|v| match v {
            Value::String(s) => Ok(s.clone()),
            Value::Bool(b) => Ok(b.to_string()),
            Value::Number(n) => Ok(n.to_string()),
            _ => Err(anyhow!("`{field}` entries must be strings")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_is_default() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::parse("\n").unwrap(), Config::default());
    }

    #[test]
    fn parses_runner_labels() {
        let cfg = Config::parse(
            "self-hosted-runner:\n  labels:\n    - linux-arm64\n    - gpu\n",
        )
        .unwrap();
        assert_eq!(cfg.self_hosted_runner_labels, vec!["linux-arm64", "gpu"]);
    }

    #[test]
    fn parses_ignore_list() {
        let cfg = Config::parse("ignore:\n  - 'runs-on'\n  - 'must be a number'\n").unwrap();
        assert_eq!(cfg.ignore, vec!["runs-on", "must be a number"]);
    }

    #[test]
    fn unknown_top_level_keys_are_ignored() {
        // Forward-compatible with actionlint keys we don't implement.
        let cfg = Config::parse("config-variables:\n  - FOO\npaths:\n  'x': {}\n").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn wrong_type_for_labels_is_an_error() {
        let err = Config::parse("self-hosted-runner:\n  labels: not-a-list\n").unwrap_err();
        assert!(err.to_string().contains("must be a list"), "{err}");
    }

    #[test]
    fn non_mapping_root_is_an_error() {
        let err = Config::parse("- just\n- a\n- list\n").unwrap_err();
        assert!(err.to_string().contains("must be a mapping"), "{err}");
    }

    #[test]
    fn self_hosted_runner_must_be_mapping() {
        let err = Config::parse("self-hosted-runner: hello\n").unwrap_err();
        assert!(err.to_string().contains("must be a mapping"), "{err}");
    }

    #[test]
    fn discover_finds_yaml_then_yml() {
        let dir = std::env::temp_dir().join(format!("alr-cfg-{}", std::process::id()));
        let gh = dir.join(".github");
        std::fs::create_dir_all(&gh).unwrap();
        // No config yet -> default.
        assert_eq!(Config::discover(&dir).unwrap(), Config::default());
        // .yml present.
        std::fs::write(gh.join("actionlint.yml"), "ignore:\n  - abc\n").unwrap();
        assert_eq!(Config::discover(&dir).unwrap().ignore, vec!["abc"]);
        // .yaml takes precedence.
        std::fs::write(
            gh.join("actionlint.yaml"),
            "self-hosted-runner:\n  labels: [big]\n",
        )
        .unwrap();
        let cfg = Config::discover(&dir).unwrap();
        assert_eq!(cfg.self_hosted_runner_labels, vec!["big"]);
        assert!(cfg.ignore.is_empty(), ".yaml should win over .yml");
        std::fs::remove_dir_all(&dir).ok();
    }
}
