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
and `soulseek-rs daemon status … || soulseek-rs daemon &` does **not** do this.
`&` backgrounds the whole list, so it returns instantly and the next command
races the login it is supposed to be using. A command that arrives too early
finds no daemon and quietly opens a session of its own, which is the outcome
this whole section exists to avoid.

Then use every other command exactly as documented below. They find the daemon
by themselves. It listens on a Unix socket only your user can open, so there is
nothing to configure, no address to pass, and no token to handle.

**Why this matters, and why it is not optional for you.** Soulseek is a real
network of other people's computers, and you arrive on it under the user's one
account. That name is their standing: peers see what it shares, add it to their
lists, and queue it again later. The server allows **one session per account**,
so without a daemon every invocation is its own login and two at once silently
cut each other off (exit 7). Never work around that by inventing usernames — an
account per command is churn on infrastructure other people maintain, and it
throws away the standing the user built.

A daemon is one login for as long as you work, however many commands you run
against it. It also keeps the user's shared folders indexed and reachable
instead of re-scanning them on every invocation, and downloads it starts
outlive the command that asked for them, so a transfer survives the process
that queued it.

So: **one daemon, then as many commands as you like.** Reach for `--no-daemon`
only when you deliberately want an isolated session, and expect to justify it.

Two things change while a daemon is running: files land in the *daemon's*
download directory (`daemon status` reports which), and `shares add` or
`config set download_dir` update it straight away rather than at its next
start.

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
| 7 | The session ended mid-command (usually another login took the username) | Nothing this run saw is reliable; start a daemon so concurrent runs share the one login, then retry — never invent a second username |

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
best|first|all`, `-n/--limit`, plus every filter `search` accepts. Use it when
any copy will do. It ranks on free slot, bitrate and peer speed, so it cannot
tell a remix from its original: for a named track or version, search and choose
yourself.

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
**`room users <room>`** — `room`, `user`, `status`, `average_speed`,
`shared_files`, `shared_folders`, `slots_full`, `country`. Everything but
`room` and `user` is `null` when the server sent no statistics for that
member — `null` means unknown, not zero. `slots_full` is true when the peer's
upload slots are all occupied, so a download from them will queue.
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
automatically, with no flags, no credentials, and no second login.
`--bind ADDR` also accepts connections from other machines, which need the
token.
State lives in the daemon, so every client reads the same thing: `search` sees
searches any client ran, the transfer queue is shared, and `message read`
starts from the conversation the daemon already collected rather than only
what arrives while it runs.

**`daemon status`** — one object: `user`, `server`, `version`, `protocol`,
`listening`, `listen_port`, `shared_folders`, `shared_files`, `download_dir`,
`session_loss`, `clients`, `uptime_secs`. Exit 3 when nothing is running.
**`daemon stop`** — ask it to shut down; prints nothing.
**`daemon token`** — the secret a client on another machine needs. Local
clients never need it.

The daemon can also be on a *different* machine, such as a home server or NAS
doing the downloading. Point at it once with `config set daemon host:5030` and
`config set daemon_token <token>`, and every command afterwards drives that
machine; files land there, not here. The interface is an open JSON-RPC schema
(`rpc.discover` serves it), so a purpose-built client is a reasonable thing to
write.

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
# One shot, when any copy will do: find it, pick the best, fetch it.
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

## Searching: widen by rungs, then match back

Soulseek matches your query against the filenames peers happen to have on
disk. Nobody agrees on how to write a track, so the name the user gives you is
usually *more* specific than any filename: it carries a mix suffix that is not
in the file, a diacritic the uploader typed as ASCII, an ampersand where the
file has a space. Search that verbatim and you get exit 4 for a track dozens of
people are sharing.

So climb down a ladder, one rung at a time, and **stop at the first rung that
returns anything usable**:

1. **As written.** `bicep glue original mix`
2. **Without the version suffix**, meaning anything in trailing brackets or
   after a dash: `(Original Mix)`, `[Radio Edit]`, `- Remastered 2011`, `(feat. X)`.
   `bicep glue`
3. **Without featured artists and punctuation**, and with diacritics folded to
   ASCII: `sigur rós hoppípolla` becomes `sigur ros hoppipolla`, `Tyler, The
   Creator - EARFQUAKE` becomes `tyler the creator earfquake`.
4. **The two or three rarest words**, usually a surname and one distinctive
   title word: `hoppipolla`.

Each rung is a *different, wider* query, which is why this does not break the
rule against re-running a search: never repeat a rung, and do not go past the
fourth. Four searches is the most any one track should ever cost, and usually
it is fewer, because two shortcuts apply:

- **A rung that produces the same string as the one before it is not a rung.**
  A title with no diacritics and no punctuation makes rung 3 identical to rung
  2. Skip it rather than spending a search on it.
- **No results at all is different from results that do not match.** An empty
  answer means the tokens are wrong, not merely too many, so go straight to the
  rarest-word rung instead of stepping. Results that came back but matched
  nothing mean you are in the right area: look further down them before
  widening.

**Then match back.** Widening finds candidates; it does not choose. Compare each
result against what the user actually asked for, folded the same way (lowercase,
diacritics stripped, punctuation to spaces), and require the distinctive words
to appear in the path. A search for `hoppipolla` will also return live versions,
covers, and a DJ set that mentions it.

Where the user named a specific version, prefer a filename that carries it and
treat one that does not as second best rather than a match:

```bash
# "Bicep - Glue (Original Mix)"  ->  rung 2 finds it
soulseek-rs search 'bicep glue' --json --min-bitrate 320
# then, among the results, prefer paths containing "glue" and not "live",
# "remix" or "edit" unless that is what was asked for.
```

A track that survives all four rungs with nothing plausible is genuinely not
there *today*. Say so and move on rather than inventing more spellings, and
offer to put it on the wishlist: `wish add <query>` keeps looking on the
server's own schedule, and `serve --wishlist` or `wish run` reports what turns
up later. That is the right home for the one white label in a set that nobody
is sharing this afternoon.

## Downloading a list of tracks

Soulseek is people's home computers, and they can see what you are doing. The
fast way and the safe way are mostly the same way, but where they differ, take
the safe one: a peer who blocks you is gone for good.

The other cost is your own. A list of thirty tracks can be done for a handful
of commands or for millions of tokens, and the difference is how much of each
result set you read. Narrow first, then read what is left.

### Before you start: are you sharing anything?

```bash
soulseek-rs shares status --json
```

If `files` is 0, **stop and tell the user before downloading anything**. Sharing
nothing is the single most common reason to get banned on Soulseek: plenty of
users block non-sharers on sight, and some clients do it automatically. One
line is enough ("you're not sharing anything, which gets you blocked; point
`shares add` at a folder first"), then let them decide.

### Search, narrow, then judge

For a named tracklist, always look at the candidates. **Do not use `get`.** It
ranks on free slot, bitrate and peer speed, which are facts about the transfer
and say nothing about whether the file is the track that was asked for. On a
setlist full of remixes, edits and white labels it does not miss, it *succeeds
with the wrong file*: exit 0, a file on disk, and nobody finds out until they
listen. A miss you can retry; a confident wrong answer you cannot.

`get` is for when the user has not named anything in particular ("find me some
deep house"). A tracklist is the opposite of that.

The cost of looking is not the looking, it is reading three hundred results to
choose one. So narrow before you read:

```bash
# 1. search, and keep the raw records in a file
soulseek-rs search 'kalabrese rumpelzirkus' --json --search-timeout 25 \
  >> raw.ndjson

# 2. cut it to a handful without reading it
jq -c 'select(.path | ascii_downcase | test("kalabrese"))' raw.ndjson \
  | jq -c 'select(.size > 20000000)' | head -8
```

The search flags do the same job before the records are ever written:
`--extension flac`, `--min-bitrate 320`, `--exclude live`, `--exclude snippet`,
`--free-slots`, `--min-size`. Use them freely; a filter is free and a result
set is not.

Then read the handful that survives and choose. Eight lines is a judgement you
can make; three hundred is a bill.

### Fuzzy matching: what "the right one" means

Fold both sides before comparing: lowercase, diacritics to ASCII, punctuation
and underscores to spaces, runs of spaces collapsed. Then:

- **Every distinctive word of the artist and title must appear** in the folded
  path. Ignore word order; peers name files every possible way round.
- **The version is what you match on, not what you dropped.** The ladder drops
  `(Kalabrese Remix)` in order to *find* anything at all; the match must then
  require `kalabrese`, or you will take the original and think you succeeded.
  Same for `dub`, `extended`, `instrumental`, a year, a label catalogue number.
- **Reject what you were not asked for**: `live`, `radio edit`, `snippet`,
  `preview`, `continuous`, `dj mix`, `mixed by`. A DJ set's tracklist will
  otherwise match the recording of the set itself.
- **Prefer lossless** for anything a DJ will play: `flac`, `aiff`, `wav`. Fall
  back to 320 kbps only when there is no lossless copy.
- **Prefer a free slot** among candidates that are equally right, never over
  one that is more right.

When two candidates are both plausible and differ in ways you cannot resolve
from the filename, say so and pick the larger lossless one. Do not download
both.

### When an artist repeats, browse one peer

If any artist appears more than once in the list, search for them once and
`browse` whoever turns up with the most:

```bash
soulseek-rs search 'artist' --json
soulseek-rs browse someuser --json      # everything they share, as paths
```

One search and one browse replaces a search per track, and the listing shows
which of them that peer does not have. Four tracks by one artist is enough to
be worth it; a whole album certainly is. Taking several files from one person
is also what their queue is for, as long as you keep to three at a time.

### Never retype a remote path

Remote paths belong to the peer. They are backslash-separated and may contain
anything, and they will not survive being read out of one process, put through
a second, and typed into a third. **So do not move the path. Move a line
number.**

```bash
soulseek-rs search 'bicep glue' --json --search-timeout 25 >> raw.ndjson
sed -n '7p' raw.ndjson > pick.ndjson     # choose by line, never by retyping
soulseek-rs download --stdin --json < pick.ndjson
```

`download --stdin` reads exactly the records `search --json` writes, so the
bytes go from the peer to the downloader without any hand touching them. Append
every rung to the same file and the line numbers stay stable.

This is also how a subagent hands a pick back: it reports *which line of which
file*, plus its reasoning. It must never quote the path itself.

### A subagent per track

Give each track its own subagent, **at most twenty at a time**. A subagent
works the search ladder, appends every rung to its own file, narrows, and
judges what is left. This is where the judgement belongs: a subagent reading
eight candidates for one track costs little, and it is the only thing that
tells a remix from its original.

Fan them out properly. Twenty subagents run one after another is slower than
doing it yourself; use whatever your host gives you for running agents in
parallel and collecting their results.

Each subagent reports its pick as a file and a line number and **waits for one
word back**. Clear each pick the moment it arrives; do not wait for the rest. A
batch that holds every transfer until the last search finishes moves no bytes
for the first minute, and one track nobody is sharing holds up the ones people
are. Answering a pick needs two running counts, not the whole set:

- **Transfers that peer already has.** Three is the cap, and it is the one that
  gets you banned. At three, hold the pick or tell the subagent to take its
  second choice elsewhere.
- **Transfers running overall.** Twenty is the cap.

Drop a pick naming a file already cleared. **Report as you go**: each subagent
says when its search found candidates and again when its download finished, so
a slow batch is distinguishable from a stuck one.

### How much to have in flight

| | At once |
|---|---|
| Subagents | 20 |
| Searches on the wire | 10 |
| Transfers running | 20 |
| Transfers from any one peer | 3 |
| Search rungs spent on one track | 4 |

Twenty transfers is the client's own default, and the reasoning is in the code:
a Soulseek transfer is paced by the peer sending it, usually a few hundred
KiB/s, so a fast link is filled by many transfers at once rather than by faster
ones.

The two numbers that are *not* just "as many as the machine can take":

- **Three from one peer.** This is the one that gets you banned. A client
  serves ten upload slots by default, so three is a visible minority of one
  person's capacity rather than a monopoly on it. Twenty transfers is fine;
  twenty transfers from two people is not.
- **Ten searches.** The server rate-limits searches, and it is the daemon's
  single connection making all of them.

### Handing a whole batch to one command

Where the picks need no arguing, put them in one file and let the tool manage
the queue:

```bash
soulseek-rs download --stdin --json -c 20 < picks.ndjson
```

It takes the list in order, so **order the lines to alternate between peers**.
Consecutive lines naming the same user are what put several simultaneous
transfers on one person.

### What not to do

- **Do not read a full result set.** Narrow it with search flags or `jq`
  first: thirty unfiltered result sets is millions of tokens, and you needed
  eight lines from each.
- **Do not let `get` choose a version for you.** It ranks on transfer facts,
  not on whether the file is the right track, so on a tracklist it returns the
  wrong one with exit 0.
- **Do not search the same query twice.** The server rate-limits searches, and
  a re-run returns what you already have. Widening it is a different query;
  repeating it is not.
- **Do not spend a rung that changes nothing.** If folding diacritics and
  punctuation leaves the same string, that rung is not a rung. Skip it.
- **Do not retry a failed transfer against the same peer.** Exit 6 or 5 means
  they went offline or are not serving you. Take the next candidate from the
  results you already have.
- **Do not fetch what you were not asked for.** Confirm before `get --pick all`
  or any fetch whose size you do not know.

Twenty at once works only because the daemon holds the session: twenty commands
sharing one login. Without a daemon they displace each other (exit 7). Do not
reach for `--username` to get around it: that registers an account per track on
someone else's server. Start the daemon first.

Keep the listener on: a peer that cannot connect back to you cannot send you
the file.

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
- **Exit 4 is not a failure.** A search that finds nothing usually means the
  query was more specific than anyone's filename. Take the next rung of the
  search ladder; use `whoami` to rule out the connection.
- **Exit 7 is not an answer.** The session ended mid-command, so nothing it saw
  counts. Something else logged in as you, almost always a second run of your
  own. Start a daemon and route through it rather than hunting for
  an unused name.
- **Never put a password in argv** — `ps` shows it to every user on the box.
  Use `--password-stdin`, the OS keychain, or `SOULSEEK_PASSWORD_CMD`.
- **Downloading pulls a file off a stranger's machine.** Confirm with the user
  before `get --pick all` or any fetch whose size is not known up front.
- **Sharing nothing gets the user banned.** Check `shares status` before a
  batch; if `files` is 0, say so before downloading. Users block leechers, and
  some clients do it without being asked.
- **At most three files at a time from any one peer.** Their queue is per-user
  and they can see it. A pile of simultaneous transfers from one person reads
  as abuse and is answered with a ban. Twenty transfers is fine; twenty
  transfers from two people is not.
- **Exit 6 means pick someone else.** The transfer died because that peer went
  away or is not serving you. Take the next candidate from the results you
  already have; retrying the same peer hammers their queue.
