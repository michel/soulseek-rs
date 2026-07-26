# Nicotine+ feature gap — shortlist and plan

Scanned Nicotine+ (nicotine-plus.org + the `pynicotine/` module map, which is
the honest feature inventory: one module per feature area). Compared against
`soulseek-rs` today.

## What soulseek-rs already has

search · download · get · browse · rooms (list/say/users/listen) · private
messages · serve/upload · user · whoami · shares (add/remove/reindex/status) ·
config · portmap · skills · TUI

## Gap list, ranked for a CLI/TUI client

| # | Nicotine+ feature | Status | CLI value |
|---|---|---|---|
| 1 | **Wishlist** — persistent queries re-searched on the server's interval | interval logged, discarded | **required by Soulseek rules** |
| 2 | **Privilege recognition** — privileged users jump the upload queue | count parsed, list discarded; no queue at all | **required by Soulseek rules** |
| 3 | **Search filters** — exclude terms, file type, min/max size | only `--min-bitrate`, `--free-slots` | high, cheap |
| 4 | Buddy list + buddy upload priority | missing | medium |
| 5 | Ban / ignore lists | missing | medium |
| 6 | Whole-folder download | per-file only | medium |
| 7 | Peer user-info exchange (description, slots, queue length) | server-side `user` only | low |
| 8 | Chat logging to disk | missing | low |
| 9 | Transfer statistics | missing | low |
| 10 | Speed limits | missing | low |
| 11 | Interests / recommendations / similar users | missing | low (social, weak in a CLI) |
| 12 | Private rooms, room wall/ticker | missing | low |
| — | Now playing, notifications, plugins, tray | missing | not CLI features |

## Scope for this pass: 1, 2, 3

Rationale: 1 and 2 are the two items Soulseek's own rules name as the price of
being a tolerated third-party client, and they are the only gaps with a real
enforcement mechanism behind them. 3 is the cheapest large usability win and
rides the same search path the wishlist needs.

## Plan (TDD — test first for every step)

### 1. Wishlist
- [x] `MessageFactory::build_wishlist_search` (server code 103) — unit test on bytes
- [x] `WishListIntervalHandler` stores the interval; `Client::wishlist_interval()` reads it
- [x] `FileConfig.wishlist: Vec<String>` persisted as a TOML array
- [x] `wish add|remove|list|run` subcommands
- [x] `serve --wishlist` re-runs wishes on the server interval
- [x] e2e: add/list/remove round-trip, duplicate add, absent remove, empty query, no config file, a comma inside a wish, `wish run` over a real code-103 search, `--only`, filters, piping into `download --stdin`, `serve --wishlist` sweeping on the announced 720s

### 2. Privilege recognition
- [x] `PrivilegedUsersHandler` parses the user list (code 69) instead of counting it
- [x] `CheckPrivileges` (code 92) → own privilege seconds; `whoami` reports it
- [x] `Client::is_privileged(user)`, `own_privilege_seconds()`, `place_in_upload_queue()`
- [x] Upload slots: cap concurrent uploads, queue the rest, privileged users first
- [x] Answer `PlaceInQueueRequest` (peer code 51) with our real queue position
- [x] e2e: slot cap holds against three raw peers; a peer asking its place gets the real one; code 92 round-trips; `whoami --json` carries it; `serve --upload-slots 1` reports a queued upload
- [x] Re-ranking on a late privileged list: covered by `client::upload_queue::context_tests` — see the note below

### 3. Search filters
- [x] `Filter` extended in `commands/transfer.rs` (where filtering already lived) rather than a new lib type
- [x] `--exclude`, `--extension`, `--min-size`, `--max-size`, shared by `search`, `get` and `wish run` through one flattened `FilterArgs`
- [x] e2e: each filter alone, both size bounds, a repeated `--extension` union, filters combined, a filter that matches nothing exits 4, and `get` honouring the same set

## Review

### Delivered

**Wishlist** — `wish add|remove|list|run`, `serve --wishlist`
- Server code 103 is built and sent; code 104's interval is stored instead of
  logged and dropped, and it drives the re-search timer. Verified live: soulfind
  announces 720s and `serve --wishlist` reports exactly that.
- Wishes live in `config.toml` as a TOML array, so a query containing a comma
  survives — which a comma-joined `config set` key would not.
- `wish run` prints `search`-shaped records, so `wish run --json | download
  --stdin` works with no reshaping. There is an e2e that does precisely that.

**Privilege recognition**
- The code-69 list is parsed into a set (it was read as a count and discarded);
  code 92 answers `whoami --json` as `privilege_seconds`.
- Uploads now go through a real queue with a slot cap (`--upload-slots`, default
  2). Privileged users sort ahead, then first-come. A queue only means something
  if there is a cap, so those are one change.
- `PlaceInQueueRequest` (peer code 51) is answered from that queue — previously
  the message had no handler at all, so peers got silence.
- Queued uploads now appear in `Client::uploads()` as `Queued(place)`, which
  makes `serve`'s long-documented `queued` status real and shows the queue in
  the TUI.

**Search filters**
- `--exclude` (repeatable), `--extension` (repeatable, dot optional),
  `--min-size`, `--max-size`, on top of `--min-bitrate` and `--free-slots`, in
  one `FilterArgs` shared by `search`, `get` and `wish run`.

### What the review gates changed

`/simplify` ran four review agents over the diff. Three findings were real
defects in the work above, not style:

1. **The `serve --wishlist` sweep was one search window per wish.** Ten wishes
   at the default 10s timeout meant a 100-second sweep during which `serve`
   reported no uploads and stopped checking its own `--duration`. Fixed by
   splitting the library's search into a send half and a wait half
   (`Client::start_wishlist_search` + `Client::collect_for`), so all the wishes
   go out and one shared window covers them. The remaining one-window block is
   marked with a `ponytail:` note.
2. **The upload queue had admission but no eviction.** A peer that queued a file
   and then went away held its slot for the life of the process — so two of them
   would wedge uploads shut permanently. This was a failure mode the slot cap
   introduced. Fixed by releasing a peer's queue entries and un-accepted offers
   on `PeerDisconnected`, then re-pumping.
3. **Three e2e assertions were vacuous.** `wait_for(|| place_of(...).is_none())`
   is already true before the peer has queued anything — `(A && B) || A` in one
   of them, reducing to `A` — so "the blocker took the slot" would have passed
   against a completely broken pump. They now wait for the peer to actually
   receive the offer (peer code 40), which is the only positive proof.

Also applied: `Filter` now stores its terms pre-normalised so matching a result
allocates nothing (a search tests thousands of files); the queue methods moved
next to the ordering code in `client/upload_queue.rs`; `Client::place_in_upload_queue`
was dropped rather than kept as permanent public API for test convenience; and
the wishlist default interval moved to the library as `DEFAULT_WISHLIST_INTERVAL`
instead of being restated in the CLI.

Skipped, with reasons: merging `Filter` into `cli::FilterArgs` (they now have
different invariants — `Filter` holds normalised data, `FilterArgs` holds raw
user input); a shared `basename` helper for the four path-splitting sites (three
are pre-existing TUI code outside this diff); and extracting a `UploadQueue`
newtype (the reviewer's own call was "don't block on it").

### Verification

`cargo test --workspace` with `SOULFIND_BIN` set and `SOULSEEK_E2E_REQUIRED=1`:
536 tests, 0 failures. The e2e suites really ran (318s for `cli_e2e`, 11s for
the library suite) rather than skipping.

`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo fmt --all --check` are clean. `aislop scan`: `soulseek-rs-lib` 100/100,
`soulseek-rs` 99/100 — the one remaining warning is the known `parse_request`
false positive (a `'{'` char literal breaks aislop's brace counter).

One `cli_e2e` test (`a_filter_that_matches_nothing_is_the_no_results_code`)
failed once on a soulfind login inside the shared test harness, then passed
3/3 in isolation and 70/70 on a full re-run. Pre-existing harness contention,
not this change.

### Where the coverage is not live, and why

The privileged-user *ordering* e2e cannot run against the soulfind build used
here: granting privileges in its database has no effect on the code-69 list it
sends, verified by probing the server directly outside the test. The wiring is
live-verified (the client logs the list it received); the ordering is covered by
`client::upload_queue::context_tests`, which drives a real `ClientContext`
through request → late privileged list → re-rank → freed slot. The e2e is kept
and skips with that reason printed, so it starts passing on a server that does
declare privileged users.

### Deliberately not done

- **Server code 91 (`AddToPrivileged`)** — the protocol documents it as
  "OBSOLETE, no longer sent by the server". Parsing it would be dead code.
- **`upload_slots` as a config key** — `serve --upload-slots` covers the
  headless case, which is the only place it matters.
- **Gap items 4–12.** Buddy list and ban/ignore are the next most valuable pair
  and share the same "persisted per-user flags" machinery, so they belong in one
  follow-up rather than half of each here.
