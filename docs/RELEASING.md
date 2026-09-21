# Releasing

Releases are **fully automated** by [semantic-release](https://semantic-release.gitbook.io/):
there are no manual version bumps or tags. Merging the right commit to `main` cuts a release,
builds cross-platform binaries, and attaches them — all from CI.

## How a release happens

```
merge to main
   │
   ▼
release job (.github/workflows/release.yml)
   • semantic-release reads the commits since the last release
   • computes the next version from Conventional Commits
   • runs scripts/set-version.sh <version>  → bumps Cargo.toml AND Cargo.lock (kept in sync so
     the build job's `cargo build --locked` doesn't fail)
   • commits the bump: "chore(release): X.Y.Z [skip ci]"
   • creates the git tag vX.Y.Z + a GitHub Release with generated notes
   • exposes `published` + `version` as job outputs
   │
   ▼  needs: release   (NOT the `release: published` event — see note below)
build job (matrix)
   • cross-compiles the CLI for each target
   • packages actionlint-rs-X.Y.Z-<target>.(tar.gz|zip) + .sha256
   • uploads them as assets on the release
```

> **Why the build job uses `needs: release`, not `on: release: published`.** A release created
> with the built-in `GITHUB_TOKEN` does **not** trigger further workflow runs — GitHub's
> recursion guard suppresses the `release: published` event
> ([docs](https://docs.github.com/en/actions/concepts/security/github_token)). So the build job
> can't listen for that event; instead it depends on the `release` job and reads the version
> from its outputs. (An alternative is to run semantic-release with a PAT / GitHub App token,
> whose events *do* trigger workflows — but that needs an extra secret, which this setup avoids.)

## What triggers a version bump

The version is derived from the commits merged since the last release, using
[Conventional Commits](https://www.conventionalcommits.org/):

| Commit type | Example | Release |
|---|---|---|
| `fix:` | `fix: cron validation rejected valid step ranges` | **patch** (x.y.**Z**) |
| `feat:` | `feat: add --format json` | **minor** (x.**Y**.0) |
| `feat!:` or a `BREAKING CHANGE:` footer | `feat!: drop --schema alias` | **major** (**X**.0.0) |
| `docs:`, `chore:`, `ci:`, `test:`, `refactor:`, … | `docs: expand the checks matrix` | **none** |

If nothing since the last release warrants a bump, no release is cut. Write commit subjects in
this style so the tool can version correctly.

## Release targets

| Target | Runner | Built with | Asset |
|---|---|---|---|
| `x86_64-unknown-linux-musl` | ubuntu-latest | `cross` (static musl) | `.tar.gz` |
| `aarch64-unknown-linux-musl` | ubuntu-latest | `cross` (static musl) | `.tar.gz` |
| `aarch64-apple-darwin` | macos-latest | native | `.tar.gz` |
| `x86_64-pc-windows-msvc` | windows-latest | native | `.zip` |

Each asset has a matching `.sha256`. The binary self-reports its version via
`actionlint-rs --version` (read from `Cargo.toml`, so it matches the tag).

## Consuming a release

**As a GitHub Action** — `action.yml` is a composite action. When referenced at a version tag
it downloads the matching prebuilt binary for the runner's OS/arch; on any other ref (a branch,
or a target with no published asset) it builds from source as a fallback:

```yaml
- uses: jreed91/actionlint-rs@v1
  with:
    files: .github/workflows/ci.yml   # optional; omit to discover all workflows
```

**As a binary** — download the asset for your platform from the
[releases page](https://github.com/jreed91/actionlint-rs/releases), verify the checksum, and
put `actionlint-rs` on your `PATH`.

**As a Docker action** — the `Dockerfile` still builds a static-musl image for anyone who
prefers `runs: using: docker`.

## Required setup (one-time, repo settings)

- The `release` job uses the built-in `GITHUB_TOKEN` (no extra secrets). It needs
  **Settings → Actions → General → Workflow permissions → Read and write**, and
  **Allow GitHub Actions to create and approve pull requests** is *not* required.
- Branch protection on `main` must allow the release bot's `chore(release):` commit (or use a
  dedicated token). The commit carries `[skip ci]` so it doesn't loop.

## Verifying locally

You can't cross-compile every target without the toolchains, but you can sanity-check the
pieces the release relies on:

```sh
scripts/set-version.sh 9.9.9   # then `git diff Cargo.toml` — only the package version changes
cargo build --release && ./target/release/actionlint-rs --version
```

CI proves the full multi-target build on the first release run.
