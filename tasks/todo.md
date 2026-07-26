# Push each release to the Homebrew tap

A release tagged and published on GitHub left `michel/homebrew-tap` stale — the
formula sat at 7.1.0 while v9.0.0 was out. The tap was only ever *pulling*, on
an hourly cron, and nothing in `release.yml` told it a release had happened.

## Plan

- [x] Bring the tap back to the current release
- [x] Add a `tap` job to `release.yml` that dispatches the tap's workflow once
      every archive and `.sha256` is uploaded
- [x] Find out why the `binaries` gate never fired on a manual dispatch
- [x] Provision `HOMEBREW_TAP_TOKEN` (Michel minted a fine-grained PAT)
- [x] Stop the tap publishing an empty `sha256` when it polls mid-upload
- [x] Verify end to end with `workflow_dispatch -f tag=v9.0.0`

## Findings

- `HOMEBREW_TAP_TOKEN` existed as a secret **name with no value**. A missing
  secret interpolates to an empty string, so `gh` answered with its generic
  "set GH_TOKEN" advice and the failure read like a workflow bug. `RELEASE_PLZ_TOKEN`
  turned out to be read-only on the tap — it can read the repo and list its
  workflows, but cannot `workflow run` or `repository_dispatch`.
- The manual "rebuild binaries for an existing tag" path had never worked. On a
  `workflow_dispatch` run `ci-gate` skips itself, and GitHub propagates that
  skip *through* `release` — which survives only via its own `always()` — to
  every descendant. `binaries` carried no status function, so it was skipped
  before its condition was ever evaluated. Both jobs now use `!cancelled()`.
- The tap's generator interpolated `$(sha …)` inside a heredoc. A failing
  command substitution there does **not** trip `set -e` (the redirection still
  succeeds), so a poll landing mid-upload would publish `sha256 ""`. Proven
  locally: the heredoc exits 0 with an empty value, the same call as an
  assignment aborts. The four hashes are now fetched into variables, validated
  as 64 hex chars, and only then written.
- Re-dispatching `binaries` for a published tag rewrites every archive with
  `--clobber`, and Rust builds are not byte-reproducible — so the checksums
  change and any already-published formula stops matching. This bit during
  verification and left `brew install` broken until the formula was regenerated.

## Decisions

- The tap's hourly cron stays. It is now the recovery path for a failed
  dispatch (expired token, tap outage) rather than the mechanism.
- `tap` needs `binaries`, not `release`: the formula reads every `.sha256` off
  the release, so it must not fire until the uploads land.
- No tag is passed to the tap. Its workflow regenerates from
  `gh release view --json tagName`, which is the release just published.
- The dispatch step checks for an empty token and names the secret, rather than
  letting `gh` emit advice that points at the wrong problem.
