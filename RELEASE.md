# Releasing

Releases are automated by [release-plz](https://release-plz.dev). You do not run
`cargo publish`, edit versions, write changelog entries, or push tags by hand.

## How it works

1. Merge a PR to `master` using [conventional commits](https://www.conventionalcommits.org)
   (`feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, `chore:`, or a `!` / `BREAKING CHANGE:`
   footer for a major bump).
2. `.github/workflows/release.yml` opens or updates a **`chore: release vX.Y.Z`** PR. It
   contains the semver bump in `[workspace.package]`, the refreshed `Cargo.lock`, and the
   generated `CHANGELOG.md` entry. Review it like any other PR.
3. Merging that PR releases:
   - tags `vX.Y.Z` and creates the GitHub release with the changelog as its notes,
   - publishes `soulseek-rs-lib` then `soulseek-rs` to crates.io (dependency order, waiting
     for the index between them),
   - builds and attaches binaries with `.sha256` checksums for six targets:

     | Archive                                          | Notes                          |
     | ------------------------------------------------ | ------------------------------ |
     | `soulseek-rs-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz`  | static, any glibc        |
     | `soulseek-rs-vX.Y.Z-aarch64-unknown-linux-musl.tar.gz` | static, any glibc        |
     | `soulseek-rs-vX.Y.Z-x86_64-apple-darwin.tar.gz`        | Intel Mac                |
     | `soulseek-rs-vX.Y.Z-aarch64-apple-darwin.tar.gz`       | Apple Silicon            |
     | `soulseek-rs-vX.Y.Z-x86_64-pc-windows-msvc.zip`        |                          |
     | `soulseek-rs-vX.Y.Z-aarch64-pc-windows-msvc.zip`       |                          |

Both crates always share one version, one tag, and one changelog — see `release-plz.toml`.

## One-time repository setup

- **Settings → Actions → General → Workflow permissions**: enable *Allow GitHub Actions to
  create and approve pull requests*. Without it the release PR is never opened.
- **Settings → Secrets and variables → Actions**: add `CARGO_REGISTRY_TOKEN`, a
  [crates.io token](https://crates.io/settings/tokens) scoped to `publish-new` and
  `publish-update`. Without it the tag and GitHub release still happen, but the crates.io
  publish fails.

## Notes

- A merge with no releasable commits (only `docs:`/`chore:`/`test:`) opens no release PR.
  That is intended.
- The release PR does not run CI on its own, because GitHub does not trigger workflows for
  PRs opened with `GITHUB_TOKEN`. Pushing any commit to that branch yourself does start it —
  handy if you want the version bump validated before merging.
- If one target's build fails, the other five still upload (`fail-fast: false`). To retry,
  run the **Release** workflow manually and set `tag` to the existing tag (e.g. `v6.0.0`);
  it rebuilds and re-uploads every target for that tag. Re-running the failed job from the
  Actions UI replays the *old* workflow file, so it is no use when the workflow is the thing
  that needs fixing.
- To skip a release for a commit that would otherwise trigger one, use a `chore:` type.

## Installing a release

```bash
cargo install soulseek-rs
```

Or download an archive from the [releases page](https://github.com/michel/soulseek-rs/releases),
verify it against the adjacent `.sha256`, and extract the binary.

## Using the library

```toml
[dependencies]
soulseek-rs-lib = "5"
```
