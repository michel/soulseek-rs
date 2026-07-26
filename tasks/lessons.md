# Lessons

## Adding slow tests to a parallel suite breaks tests you never touched

**2026-07-26, library e2e on CI.** CI failed two *download* tests with
`login: Timeout` — tests the change under review never went near, and which had
passed on the previous run of the same branch. The cause was load: unlike
`cli_e2e`, the library suite had no `SERVER_GATE`, so every test spawned its own
soulfind concurrently. Survivable at 25 tests; the four queue tests added here
are long-running and hold peer sockets open on purpose, which starved the
servers enough to leave a login unanswered.

**Why it matters:** the failure pointed at innocent code, so the tempting reads
were "flake, re-run it" and "the download path regressed". Both are wrong, and
the first one ships a suite that keeps failing on unrelated PRs.

**How to apply:** when CI fails a test your diff does not touch, check what your
diff did to the *shared* resources the suite competes for — wall-clock, ports,
server processes, open sockets — before concluding flake or regression. And when
a sibling suite already carries a gate, read its comment first:
`cli_e2e::SERVER_GATE`'s comment described this exact failure mode, so the fix
was to apply the same gate, not to invent one.

## A polling assertion that is true before the event is not an assertion

**2026-07-26, upload-queue e2e.** Three tests waited for "the blocker took the
slot" with `wait_for(|| place_of(...).is_none())`. That is already true before
the peer has queued anything, so `wait_for` returned on its first poll. One was
worse: `(A && B) || A`, which `&&`-binding reduces to plain `A`, leaving the
`uploads()` half dead. All three would have passed against a completely broken
pump. A review agent caught it; the test suite never would have.

**Why it matters:** a green test that cannot fail is worse than no test, because
it buys false confidence in exactly the code you were least sure about.

**How to apply:** for any `wait_for`/poll assertion, ask "what does this return
on the very first call, before the thing I am waiting for has happened?" If the
answer is `true`, the assertion is inverted or vacuous. Prefer waiting on a
*positive* observation — here, the peer receiving the offer (peer code 40) —
over the absence of something, since absence is also the initial state.

## Adding a cap without adding an expiry invents a wedge

**2026-07-26, upload slots.** Adding `upload_slots` (default 2) gave privileged
users something to jump, but nothing ever removed an un-accepted offer from
`ctx.uploads`. A peer that queued a file and vanished held its slot for the life
of the process, so two of them shut uploads down permanently. Before the cap,
a stale entry was harmless — the cap is what turned it into a failure mode.

**How to apply:** when introducing a bound on a resource, find the release path
in the same change. Ask "what happens if the thing holding this never comes
back?" Here the mechanism already existed and was already wired
(`ClientOperation::PeerDisconnected`) — the fix was ten lines in an arm I was
already editing.

## Don't wait per item when one shared window covers them all

**2026-07-26, wishlist sweep.** `sweep` called a blocking
`wishlist_search(wish, timeout)` per wish. Ten wishes at the default 10s timeout
meant a 100-second sweep, during which `serve` reported no uploads and stopped
checking its own `--duration`. Peers answer a search whenever they get round to
it, so the window was never per-query to begin with.

**How to apply:** when a "wait for responses" API is called in a loop, check
whether the wait is inherent to each call or just how the wrapper was written.
Splitting send from wait (`start_wishlist_search` + `collect_for`) turned N
windows into one. Watch for this whenever a blocking helper is convenient for
the single-item case and quietly quadratic for the batch.

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

## Confirm a bottleneck hypothesis before doing surgery on it

**2026-07-26, concurrency stress work.** Downloads stalled under mixed load. I
reasoned my way to a confident cause — `wait_while_paused` taking a global read
lock every 8 KiB, starving the single writer thread on a reader-preferring
`RwLock` — and it was wrong. Throttling that check changed nothing. The real
cause was a race two layers away: `TransferResponse` was sent before the client
recorded the peer's transfer token, so the incoming file connection matched
nothing.

**Why it matters:** the wrong hypothesis was *plausible*, mechanically specific,
and would have justified a real refactor of the download hot path. Shipping it
would have added churn to code that was never the problem, and left the actual
bug in place.

**How to apply:** before restructuring anything, run the cheapest experiment
that would falsify the hypothesis — here, two lines throttling the lock, which
took one run to disprove. When that fails, stop theorising and get the trace: a
debug log filtered to one stuck peer showed the file connection arriving 2 ms
after the token update was queued, which named the bug outright. Grep a failing
identifier through the log before guessing at the mechanism.

## Report the load generator's own failures separately

**Same session.** The mock swarm's peers sometimes failed to come online, and
the upload metric counted only peers that made it — so `188/188` looked perfect
while four peers had silently vanished from the denominator. A harness that
drops its own failures reports a flattering score exactly when it is least
earned.

**How to apply:** count attempts, not survivors, and give harness-side failures
their own line in the report. `mocks never online 2` next to `uploads 190/192`
is what let me tell "the client dropped two uploads" apart from "the test rig
never made the request" — a distinction I had been getting wrong for several
runs, in the direction of blaming the client.

## A defensive guard can delete a fallback path

**Same session.** The server answers `GetPeerAddress` with port 0 when it does
not know where a user listens. The client cached and dialled it, losing uploads,
so I skipped the handler on port 0. That broke three browse tests: the *failing
dial* is precisely what triggers the server-brokered fallback for firewalled
peers. The bogus-looking behaviour was load-bearing.

**How to apply:** when adding an early return for an invalid value, check what
currently consumes that value further down. Scope the guard to the specific
misuse — here, do not cache and do not serve against port 0 — rather than
short-circuiting the whole path. And diff against an unmodified worktree at HEAD
before concluding a test failure is pre-existing flakiness; I nearly did, and the
baseline was green 25/25.
