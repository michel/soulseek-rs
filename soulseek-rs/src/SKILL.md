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

## Commands

Each entry lists the keys of the objects it prints.

**`search <query>`** — one object per matching file:
`user`, `path`, `size`, `bitrate`, `duration`, `free_slot`, `slots`, `speed`.
Flags: `--min-bitrate <kbps>`, `--free-slots`, `-n/--limit` (0 means all),
`--sort best|size|bitrate|speed`. Collects results for the whole
`--search-timeout` window before printing anything.

**`download <user> <path> [--size N]`** or **`download --stdin`** — one object
per finished transfer: `user`, `path`, `size`, `file` (the local path written).
`--stdin` consumes `search --json` output as-is. A transfer that fails leaves a
`<file>.part` behind; re-running the same command resumes from it rather than
starting over, so retrying a failed download is cheap.

**`get <query>`** — search, pick, and download in one step. `--pick
best|first|all`, `-n/--limit`, `--min-bitrate`, `--free-slots`. Prefer this
over `search` + `download` when the user just wants the file.

**`browse <user>`** — one object per file the peer shares: `user`, `directory`,
`path`, `size`.

**`user <name>`** — one object: `user`, `status` (`online`/`away`/`offline`),
`privileged`, `average_speed`, `shared_files`, `shared_folders`. Any field may
be `null` when the server has not answered that half — `null` means unknown,
not offline.

**`whoami`** — one object: `user`, `server`, `listening`, `listen_port`,
`shared_folders`, `shared_files`, `download_dir`. The cheapest way to prove the
credentials and connection work.

**`room list`** — `room`, `users`.
**`room users <room>`** — `room`, `user`.
**`room listen <room>`** — streams `type`, `room`, `user`, `message`.
**`room say <room> <message>`** — prints nothing; exit 0 is the confirmation.

**`message read`** — streams `timestamp`, `user`, `message`.
**`message send <user> <message>`** — prints nothing; exit 0 is the confirmation.

**`serve`** — stay online sharing files, one object per upload change: `user`,
`path`, `size`, `bytes_sent`, `speed`, `status`
(`queued`/`uploading`/`completed`/`cancelled`/`failed`), `reason`.

**`shares list`** — one object per configured folder: `directory`, `usable`.
**`shares add <directory>`** and **`shares remove <directory>`** — change the
config file; they print nothing.
**`shares status`** and **`shares reindex`** — `folders`, `files`,
`directories`, counted by logging in and scanning.

**`config path`**, **`config list`**, **`config get <key>`**, and
**`config set <key> <value>`** — `key`, `value`.

**`portmap`** — `ok`, `backend`, `external`, `port`. Exit 4 when no router
answered, which is a negative answer rather than an error.

**`skills install`**, **`skills uninstall`**, and **`skills list`** — write
this skill file into the local agent directories, or remove it: `agent`,
`path`, `action`.

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
- **Never put a password in argv** — `ps` shows it to every user on the box.
  Use `--password-stdin`, the OS keychain, or `SOULSEEK_PASSWORD_CMD`.
- **Downloading pulls a file off a stranger's machine.** Confirm with the user
  before `get --pick all` or any fetch whose size is not known up front.
