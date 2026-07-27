---
name: soulseek-rs
description: >-
  Search for, download, and share files on the Soulseek peer-to-peer network
  using the soulseek-rs command-line client. Use when the user wants to find or
  fetch music or other files from Soulseek, list what a peer shares, check
  whether a peer is online, read or post in Soulseek chat rooms, send private
  messages, stay online serving uploads, or run soulseek-rs as a background
  service that several commands share.
---

# soulseek-rs

A Soulseek client whose subcommands are built to be driven by a program. Run
`soulseek-rs` with no subcommand and it launches an interactive terminal UI —
never do that; it needs a real terminal and never returns.

## Start a daemon first

Before anything else, make sure one is running:

```bash
soulseek-rs daemon status --json >/dev/null 2>&1 || {
  soulseek-rs daemon >/dev/null 2>&1 &
  for _ in $(seq 30); do
    soulseek-rs daemon status --json >/dev/null 2>&1 && break
    sleep 1
  done
}
```

Wait for it, do not just launch it. A daemon takes a second or two to log in,
and `soulseek-rs daemon status … || soulseek-rs daemon &` does **not** do this
— `&` backgrounds the whole list, so it returns instantly and the next command
races the login it is supposed to be using. A command that arrives too early
finds no daemon and quietly opens a session of its own, which is the outcome
this whole section exists to avoid.

Then use every other command exactly as documented below. They find the daemon
by themselves — it listens on a Unix socket only your user can open, so there
is nothing to configure, no address to pass, and no token to handle.

**Why this matters, and why it is not optional for you.** Soulseek is a real
network of other people's computers, and the server allows **one session per
account**. Without a daemon, every invocation is its own login, and running two
at once leaves you two bad choices: share a username and they silently cut each
other off (exit 7), or give each run a throwaway name and *register a new
account on a public server for every search you make*. The second is what a
naive script does, and it is the antisocial one — it is account churn on
infrastructure other people maintain and share.

A daemon is one login for as long as you work, however many commands you run
against it. It also indexes your shared folders once instead of re-scanning
them on every invocation, and downloads it starts outlive the command that
asked for them, so a transfer survives the process that queued it.

So: **one daemon, then as many commands as you like.** Reach for `--no-daemon`
only when you deliberately want an isolated session, and expect to justify it.

Two things change while a daemon is running: files land in the *daemon's*
download directory (`daemon status` reports which), and `shares add` updates it
straight away rather than at its next start.

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

With a daemon running this needs no thought — they all share its one session:

```bash
for q in 'gary beck fold' 'marcal steady'; do
  soulseek-rs search "$q" --json &
done
wait
```

Without one, the same loop is a bug: the runs share a username, so each login
displaces the last and they kill each other's session (exit 7). Giving each run
its own `--username` avoids the collision by registering a fresh account on the
server for every search, which is worse — do not do it. Start the daemon.

The listener port needs no care either way: a run that finds the configured
port taken binds a free one and tells the server about that one instead.

## Rules

- **Remote paths belong to the peer.** They are backslash-separated and may
  contain anything. Pass them back byte-identical — never normalise, unescape,
  or let a shell expand them.
- **`--follow` never returns.** For `room listen`, `message read`, and `serve`,
  always pass `--duration <seconds>` instead.
- **`daemon` never returns either.** It is a service: background it
  (`soulseek-rs daemon &`) and check it with `daemon status`. Running it in the
  foreground will hang you until the timeout.
- **Every wait is bounded.** `--timeout` per command, `--search-timeout` for
  searches. There is no unbounded form.
- **Exit 4 is not a failure.** A search that finds nothing means the query was
  too narrow or misspelled. Widen it; use `whoami` to rule out the connection.
- **Exit 7 is not an answer.** The session ended mid-command, so nothing it saw
  counts. It means something else logged in as you — almost always a second
  run of your own. Start a daemon and route through it rather than hunting for
  an unused name.
- **Never put a password in argv** — `ps` shows it to every user on the box.
  Use `--password-stdin`, the OS keychain, or `SOULSEEK_PASSWORD_CMD`.
- **Downloading pulls a file off a stranger's machine.** Confirm with the user
  before `get --pick all` or any fetch whose size is not known up front.
