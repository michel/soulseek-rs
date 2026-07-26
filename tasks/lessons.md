# Lessons

## Don't act on a credential choice the moment it is selected

**2026-07-26, Homebrew tap automation.** Offered three ways to give the release
write access to the tap, one of which was reusing Michel's local `gh` token —
an account-wide, non-expiring credential. He picked it, and I wrote it into the
repo's Actions secrets in the next tool call. He immediately reversed: "i will
make a dedicated token."

**Why it matters:** storing a broad credential in CI is outward-facing and only
half-reversible. Deleting the secret is easy; the token having been written to
GitHub's secret store is not something you can take back with certainty.

**How to apply:** when an option carries a security cost I have just finished
describing, treat the selection as intent, not as authorisation to execute.
Restate the consequence in one line and let the next message confirm. The cost
of one extra beat is trivial next to provisioning the wrong credential.

## A skipped job poisons everything downstream of it

`ci-gate` skipping itself on `workflow_dispatch` cascaded *through* `release`
— which survived on its own `always()` — and skipped `binaries` before its
`if` was ever evaluated. I changed the condition twice, chasing the `inputs`
context, before comparing runs and noticing `binaries` only ever ran when
`ci-gate` *ran*.

**How to apply:** when a job is skipped and its condition looks correct, check
the whole `needs` chain for a skipped ancestor before touching the expression.
An intermediate `always()` rescues that job alone, not its descendants. Compare
a working run against a failing one early — the diff pointed straight at it and
would have saved two dispatch cycles.
