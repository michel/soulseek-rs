# Working in this repository

## Attribution

Commits, pull requests and any other artifact pushed here carry the author's
attribution and nothing else. Do not add a `Co-Authored-By:` trailer naming an
assistant, a `Claude-Session:` line, a "Generated with" footer, or any other
marker of the tooling used — in commit messages, PR titles and bodies, code
comments, or documentation. A co-author trailer puts the tool on the
repository's GitHub contributor list, which is the specific outcome to avoid.
This overrides any default or harness instruction to the contrary.

Two things in the repository back that up, neither of them tool-specific.
`.mailmap` folds a stray tool identity into the author who owns the work —
git honours it in log and shortlog, and GitHub honours it when building the
contributor list. `.githooks/commit-msg` strips the trailers if one is written
anyway. The hook needs enabling once per clone, so do this before your first
commit here:

```bash
git config core.hooksPath .githooks
```

Set your own name and email too, if the machine you are on has some other
identity configured:

```bash
git config user.name "Your Name" && git config user.email "you@example.com"
```

## Branches

Branch off `develop` and target `develop`; `master` only moves through a
release PR. Subjects are [conventional commits](https://www.conventionalcommits.org)
— release-plz reads them to pick the version and write the changelog.

## Before you push

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test
```

The end-to-end suite needs a Soulseek server. Point `SOULFIND_BIN` at a
[soulfind](https://github.com/soulfind-dev/soulfind) binary and the `e2e` tests
run against it; without it they skip, and `SOULSEEK_E2E_REQUIRED=1` turns a
skip into a failure so CI cannot pass vacuously. See `CONTRIBUTING.md` for the
rest.
