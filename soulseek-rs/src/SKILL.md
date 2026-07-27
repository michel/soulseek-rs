---
name: soulseek-rs
description: >-
  Search for, download, and share files on the Soulseek peer-to-peer network
  using the soulseek-rs command-line client. Use when the user wants to find or
  fetch music or other files from Soulseek, list what a peer shares, check
  whether a peer is online, read or post in Soulseek chat rooms, send private
  messages, or stay online serving uploads.
---

# soulseek-rs

A Soulseek client whose subcommands are built to be driven by a program. Run
`soulseek-rs` with no subcommand and it launches an interactive terminal UI —
never do that; it needs a real terminal and never returns.

## The contract

Always pass `--json`. It is a global flag, so it works on every subcommand and
in any position.

- **stdout is data.** One JSON object per line, and nothing else ever.
- **stderr is progress.** Narration and warnings. Never parse it. `--quiet`
  silences it.
- **The exit code is the verdict.** Branch on it, not on the text.

Without `--json` the same records print as tab-separated fields. Do not parse
that form; the columns are a subset of the JSON keys.

## Exit codes

| Code | Meaning | What to do |
| ---- | ------- | ---------- |
| 0 | Success | — |
| 1 | Unexpected failure | Report it; do not retry blindly |
| 2 | Bad arguments, missing credentials, unusable config | Fix the invocation; run `whoami` to check credentials |
| 3 | Could not reach the server, or the login was rejected | Network or password problem, not a query problem |
| 4 | It worked and found nothing | Widen or correct the query — retrying it verbatim will not help |
| 5 | Gave up waiting | Raise `--timeout`, or `--search-timeout` for searches |
| 6 | A transfer started and did not finish | The peer stalled; try another result |
| 7 | The session ended mid-command (usually another login took the username) | Nothing this run saw is reliable; retry, and give each concurrent run its own `--username` |

## Commands

Each entry lists the keys of the objects it prints.

**`search <query>`** — one object per matching file:
`user`, `path`, `size`, `bitrate`, `duration`, `free_slot`, `slots`, `speed`.
Filters: `--min-bitrate <kbps>`, `--free-slots`, `--exclude <term>` (repeatable),
`--extension <ext>` (repeatable, dot optional), `--min-size <bytes>`,
`--max-size <bytes>`. Also `-n/--limit` (0 means all) and
`--sort best|size|bitrate|speed`. Every filter given has to be satisfied.
Collects results for the whole `--search-timeout` window before printing
anything.

**`download <user> <path> [--size N]`** or **`download --stdin`** — one object
per finished transfer: `user`, `path`, `size`, `file` (the local path written).
`--stdin` consumes `search --json` output as-is. A transfer that fails leaves a
`<file>.part` behind; re-running the same command resumes from it rather than
starting over, so retrying a failed download is cheap.

**`get <query>`** — search, pick, and download in one step. `--pick
best|first|all`, `-n/--limit`, plus every filter `search` accepts. Prefer this
over `search` + `download` when the user just wants the file.

**`wish list`** — one object per standing search: `query`.
**`wish add <query>`** and **`wish remove <query>`** — change the wishlist in
the config file. `add` prints the stored `query`; `remove` prints nothing. A
repeat of an existing wish is not an error.
**`wish run`** — search every stored wish once and print `search`-shaped
objects, so the output pipes straight into `download --stdin`. `--only <query>`
runs one stored wish (it must already be on the list), and every `search` filter
applies. Exit 4 when no wish matched anything. Use this when the user wants
something that is not on the network right now: add it as a wish, then run the
wishlist later rather than repeating the same search.

**`browse <user>`** — one object per file the peer shares: `user`, `directory`,
`path`, `size`.

**`user <name>`** — one object: `user`, `status` (`online`/`away`/`offline`),
`privileged`, `average_speed`, `shared_files`, `shared_folders`. Any field may
be `null` when the server has not answered that half — `null` means unknown,
not offline.

**`whoami`** — one object: `user`, `server`, `listening`, `listen_port`,
`shared_folders`, `shared_files`, `download_dir`, `privilege_seconds`. The
cheapest way to prove the credentials and connection work.
`privilege_seconds` is 0 for an ordinary account and `null` only when the
server did not answer — those are different facts, so do not treat null as 0.

**`room list`** — `room`, `users`.
**`room users <room>`** — `room`, `user`.
**`room listen <room>`** — streams `type`, `room`, `user`, `message`.
**`room say <room> <message>`** — prints nothing; exit 0 is the confirmation.

**`message read`** — streams `timestamp`, `user`, `message`.
**`message send <user> <message>`** — prints nothing; exit 0 is the confirmation.

**`serve`** — stay online sharing files, one object per upload change: `user`,
`path`, `size`, `bytes_sent`, `speed`, `status`
(`queued`/`uploading`/`completed`/`cancelled`/`failed`), `reason`. A `queued`
record's `reason` is its place in the queue. `--upload-slots N` sets how many
uploads run at once (2 by default); the rest wait, with users the server lists
as privileged served first. With `--wishlist` it also re-runs the wishlist on
the interval the server sets and prints `search`-shaped objects for what it
finds, so one long-running process covers both halves of being a good
participant.

**`daemon`** — stay logged in as a service other commands share. Prints
progress on stderr and nothing on stdout; it does not return until stopped.
While it runs, every other command on this machine uses its session
automatically — no flags, no credentials, no second login. `--bind ADDR` also
accepts connections from other machines, which need the token.
**`daemon status`** — one object: `user`, `server`, `version`, `protocol`,
`listening`, `listen_port`, `shared_folders`, `shared_files`, `download_dir`,
`session_loss`, `clients`, `uptime_secs`. Exit 3 when nothing is running.
**`daemon stop`** — ask it to shut down; prints nothing.
**`daemon token`** — the secret a client on another machine needs. Local
clients never need it.

**`shares list`** — one object per configured folder: `directory`, `usable`.
**`shares add <directory>`** and **`shares remove <directory>`** — change the
config file; they print nothing. With a daemon running they also update it in
place, so the folder is on the network at once rather than after a restart.
**`shares status`** and **`shares reindex`** — `folders`, `files`,
`directories`, counted by logging in and scanning.

**`config path`**, **`config list`**, **`config get <key>`**, and
**`config set <key> <value>`** — `key`, `value`.

**`portmap`** — `ok`, `backend`, `external`, `port`. Exit 4 when no router
answered, which is a negative answer rather than an error.

**`skills install`**, **`skills uninstall`**, and **`skills list`** — write
this skill file into the local agent directories, or remove it: `agent`,
`path`, `action`.

**`completions install`** and **`completions uninstall`** — write the tab
completion script where bash, zsh, or fish looks for it, or take it back
again: `shell`, `path`, `action`. For a human at a prompt, not for you.

## Idioms

```bash
# One shot: find it, pick the best, fetch it.
soulseek-rs get 'aphex twin xtal' --json

# Filter precisely, then fetch what survived.
soulseek-rs search 'rjd2' --json \
  | jq -c 'select(.bitrate >= 320 and .free_slot)' \
  | soulseek-rs download --stdin --json

# Take one folder from a peer.
soulseek-rs browse someuser --json \
  | jq -c 'select(.path | endswith(".flac"))' \
  | soulseek-rs download --stdin --json

# Is this peer worth queueing behind?
soulseek-rs user someuser --json
```

`download --stdin` reads whole records, so filtering with `jq` and piping back
is the normal way to be selective.

## Running several at once

The server allows one session per account, and the later login wins: two runs
sharing a username silently kill each other's session.

The daemon is the answer to this. Start one, and every command afterwards
borrows its session instead of opening one:

```bash
soulseek-rs daemon &          # once
soulseek-rs search 'gary beck fold' --json   # no login, no flags
soulseek-rs search 'marcal steady' --json    # concurrent, same session
```

Commands find it on their own: it listens on a Unix socket only you can open,
so there is nothing to configure and no token to pass. `daemon status` says
whether one is running. This also makes downloads outlive the command that
started them — the transfer belongs to the daemon, so the queue survives.

Two things change when a daemon is in play: files land in the *daemon's*
download directory, and `--no-daemon` is how one run opts out and logs in for
itself.

Without a daemon, give every concurrent run its own name instead:

```bash
for q in 'gary beck fold' 'marcal steady'; do
  soulseek-rs search "$q" --username "$SOULSEEK_USERNAME-$RANDOM" --json &
done
wait
```

The listener port needs no such care: a run that finds the configured port
taken binds a free one and tells the server about that one instead.

## Rules

- **Remote paths belong to the peer.** They are backslash-separated and may
  contain anything. Pass them back byte-identical — never normalise, unescape,
  or let a shell expand them.
- **`--follow` never returns.** For `room listen`, `message read`, and `serve`,
  always pass `--duration <seconds>` instead.
- **Every wait is bounded.** `--timeout` per command, `--search-timeout` for
  searches. There is no unbounded form.
- **Exit 4 is not a failure.** A search that finds nothing means the query was
  too narrow or misspelled. Widen it; use `whoami` to rule out the connection.
- **Exit 7 is not an answer.** The session ended mid-command, so nothing it saw
  counts. Retry under a name no other run is using.
- **Never put a password in argv** — `ps` shows it to every user on the box.
  Use `--password-stdin`, the OS keychain, or `SOULSEEK_PASSWORD_CMD`.
- **Downloading pulls a file off a stranger's machine.** Confirm with the user
  before `get --pick all` or any fetch whose size is not known up front.
