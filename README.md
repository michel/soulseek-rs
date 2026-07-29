<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="website/public/wordmark-on-dark.svg">
    <img src="website/public/wordmark-on-light.svg" alt="soulseek-rs" width="340">
  </picture>
</div>

**A soulseek client for the terminal. Built for agents and people who live
there.**

Search the network, share your files, browse someone's collection, join a room.
It runs over ssh on the machine where your music already lives.

Soulseek is a closed-source P2P network from the 2000s, still used by music
enthusiasts to share niche music. This repository is that client plus
[`soulseek-rs-lib`](./soulseek-rs-lib), the protocol library under it.

**[re-invention.nl/soulseek-rs](https://re-invention.nl/soulseek-rs/)**: what it
does, how to install it, and every `config.toml` setting.

## Demo

[![soulseek-rs TUI demo](fixtures/demo.svg)](https://re-invention.nl/soulseek-rs/)

## Features

- **Search & download**: queue from the TUI or fetch in one command with `get`;
  filter by bitrate, size, file type, free slots, or terms to exclude
- **Wishlist**: `wish add` what nobody has today; `wish run` and
  `serve --wishlist` keep looking on the interval the server sets
- **Sharing**: `shares add` a directory and your files show up in searches,
  with `serve` online so peers can browse and download them
- **Upload queue**: a slot cap, server-listed privileged users first, and a
  truthful answer when a peer asks their place in line
- **Browse**: list any user's shared files and download straight from the tree
- **Resumable downloads**: a dead transfer leaves a `.part`, and re-running it
  asks the peer for only what is missing
- **Chat rooms**: list, join, and talk in public rooms, several open at once
- **Private messages**: send and receive, with an inbox in the TUI
- **Firewalled peers**: browsing and downloads fall back to server-brokered
  connections when a peer is unreachable directly
- **Automatic port mapping**: UPnP-IGD and NAT-PMP, with `portmap` to test your
  router
- **TUI and CLI**: every feature reachable without a terminal, records on
  stdout, the exit code saying what happened
- **Daemon mode**: one login several clients share, locally or over the network,
  so a download outlives the command that queued it

## About

A learning exercise in Rust. I've been on Soulseek since the early 2000s, and
its closed-source protocol is a good way to learn asynchronous network
programming and reverse engineering.

A Cargo workspace with two crates: **soulseek-rs-lib**, the protocol
implementation for anyone building their own client, and **soulseek-rs**, the
client built on it. The library stays lean on dependencies and has none today;
the client takes them freely.

## Installation

Homebrew ships a prebuilt binary, so no Rust toolchain:

```bash
brew install michel/tap/soulseek-rs   # macOS and Linux
cargo install soulseek-rs             # or from crates.io
```

From source, `cargo build --release` leaves the binary at
`target/release/soulseek-rs`. To build your own client on the protocol library,
add `soulseek-rs-lib = "8"` to your `Cargo.toml`.

## Usage

Run it with no subcommand for the interactive TUI:

```bash
soulseek-rs
```

Everything else is a one-shot command that runs headless.

### Daemon mode

A one-shot command logs in, does its job, and exits. Run a daemon instead and
it holds the session while everything else borrows it:

```bash
soulseek-rs daemon                 # stays in the foreground; leave it running
soulseek-rs search 'aphex twin'    # no login, no flags; uses the daemon
soulseek-rs                        # the TUI attaches to the same session
```

**Why you want this.** The server allows one session per account, so without a
daemon two commands at once cut each other off. A daemon is one login for as
long as you work, your shares indexed once instead of rescanned every
invocation, downloads that outlive the command that queued them, and chat
collected while nothing is attached. Driving this from a script or an agent,
start one.

Commands find it themselves. It listens on `daemon.sock` in the state directory
(`~/.local/share/soulseek-rs/state`, or wherever `SOULSEEK_STATE_DIR` points),
openable only by your user, with the token remote clients need beside it in
`daemon.token`. Without one running everything behaves as it did before, and
`--no-daemon` forces that for a single run.

Windows has no Unix socket. A daemon there needs `--bind ADDR`, and every
client needs `--daemon ADDR` and the token even on the same machine.

From a script, start it and *wait* for it. The socket appears before the login
finishes, and a command arriving early waits on the handshake for up to 30
seconds. Poll `daemon status` first.

```bash
soulseek-rs daemon status --json >/dev/null 2>&1 || {
  soulseek-rs daemon >/dev/null 2>&1 &
  for _ in $(seq 30); do
    soulseek-rs daemon status --json >/dev/null 2>&1 && break
    sleep 1
  done
}
```

```bash
soulseek-rs daemon status          # who it is, what it shares, who is attached
soulseek-rs daemon stop
```

Every window is a view of the same session. A second TUI shows the searches and
transfers already running, queueing in either shows up in both, and the status
bar names the daemon it attached to. Closing a window leaves its downloads
running. The settings popup edits the daemon's folders, validated there and
created nowhere on this machine, and writes the same paths into this machine's
`config.toml` for the next run without a daemon.

Everything touching the network goes through it. `config`, `skills`,
`completions` and `portmap` stay local. `shares add`/`remove`,
`config set download_dir` and `config set shared_dirs` do both: they write the
config file *and* update a running daemon, so a folder is served or downloaded
into straight away rather than at the next start. Files land on the *daemon's*
filesystem, and `daemon status` reports where.

It does not sweep the wishlist. Standing searches re-run inside
`serve --wishlist` or a one-off `wish run`, both of which borrow the daemon's
session, so schedule either one to work the list through. With a remote daemon
the list lives in the client's `config.toml`, so `wish run` from the laptop
drives the daemon against the laptop's wishes.

`daemon --upload-slots N` sizes the upload queue for the session it holds, 1 to
1000, where `serve` stops at 64.

#### A download box you drive from your laptop

Put soulseek-rs on the machine that should be downloading: a home server, a
NAS, a Raspberry Pi. **On that machine**, let the daemon take connections from
your network:

```bash
soulseek-rs daemon --bind 0.0.0.0:5030
```

It prints the token on stderr at startup, with the two `config set` lines to
paste on the other machine. `daemon token` prints it again, always *this*
machine's, so run it on the box. It needs no terminal and nobody logged in;
hand it to systemd or launchd. Run the unit as the user who types the commands,
or export the same `SOULSEEK_STATE_DIR` in both, because a client looks for the
socket under its own state directory.

**On your laptop**, say once where it lives:

```bash
soulseek-rs config set daemon nas.local:5030
soulseek-rs config set daemon_token <the token you copied>
```

Every command after that is the normal command, and the laptop needs no
Soulseek account of its own:

```bash
soulseek-rs get 'selected ambient works'
soulseek-rs daemon status          # what it's doing right now
soulseek-rs daemon stop            # shuts down the remote daemon
```

Downloads belong to the daemon, so closing the lid does not stop them. They
still honour the *command's* deadline: a `download` or `get` reaching
`--timeout` (300s) tells the daemon to drop the transfer before it exits 5.
Raise it for a big file, or queue from the attached TUI and quit.

`shares add` resolves paths against the machine you type it on, so
`shares add /srv/music` from the laptop exits 2 unless the laptop has that path
too. Add the box's folders on the box. Pushing a share list or download folder
to a remote daemon changes its live session while writing the *laptop's* config
file, so put the change in the daemon's own `config.toml` as well, or push it
again after a restart.

That TCP port has no encryption. Fine on a home network; do not expose it to
the internet. Reach it over SSH instead, with a tunnel
(`ssh -L 5030:localhost:5030 nas`) or by running the commands on the box.

#### Build your own remote

The daemon speaks newline-delimited JSON-RPC 2.0, described by an
[OpenRPC](https://open-rpc.org) document,
[`docs/openrpc.json`](docs/openrpc.json), which it also serves over
`rpc.discover`. Point a client generator at it for typed bindings in your
language. Searches, transfers, rooms, private messages and shares are all on
the interface this CLI uses, with no private back channel, and live updates are
pushed rather than polled.

Keep both ends on the same release; the handshake refuses mismatched protocol
versions. The destination is session-wide, so `download.set_dir` moves it for
every transfer and `download.start` ignores any directory the caller names.
[`docs/daemon-protocol.md`](docs/daemon-protocol.md) covers the framing, the
handshake, and how pushed events behave.

### Scripting

The one-shot commands follow three rules, so they compose with other tools:

- **stdout is data.** One record per line, and nothing else. Progress,
  warnings, and errors go to stderr; `--quiet` silences the progress.
- **`--json` emits newline-delimited JSON**, one object per line. Without it,
  records are tab-separated fields with no header and no decoration.
- **The exit code is the verdict.** No output plus exit 0 never means failure.

| Code | Meaning                                                           |
| ---- | ----------------------------------------------------------------- |
| 0    | success                                                           |
| 1    | unexpected error                                                  |
| 2    | bad arguments, missing credentials, unusable configuration        |
| 3    | could not reach the server or a daemon, or the login was rejected |
| 4    | the command worked but found nothing                              |
| 5    | timed out waiting for a response or a transfer                    |
| 6    | a transfer started but did not finish                             |
| 7    | the session ended mid-command; nothing it saw is reliable         |

#### Commands

```bash
soulseek-rs search <QUERY>                # print matching files
soulseek-rs download <USER> <PATH>        # fetch one known file
soulseek-rs download --stdin              # fetch files listed on stdin
soulseek-rs get <QUERY>                   # search, pick, and download
soulseek-rs wish add|remove|list <QUERY>  # keep looking for something
soulseek-rs wish run                      # search every stored wish once
soulseek-rs browse <USER>                 # list a user's shared files
soulseek-rs room list                     # public rooms and user counts
soulseek-rs room say <ROOM> <MESSAGE>     # post to a room
soulseek-rs room listen <ROOM>            # stream room messages and joins
soulseek-rs message send <USER> <TEXT>    # send a private message
soulseek-rs message read                  # stream incoming private messages
soulseek-rs room users <ROOM>             # who is in a room
soulseek-rs serve [--follow]              # stay online sharing, stream uploads
soulseek-rs daemon [--bind ADDR]          # run as a service others share
soulseek-rs daemon token|status|stop      # control a running one
soulseek-rs whoami                        # confirm credentials and connection
soulseek-rs user <NAME>                   # a peer's status and share counts
soulseek-rs shares list|add|remove|status|reindex
soulseek-rs config path|list|get|set
soulseek-rs portmap                       # test automatic port mapping
soulseek-rs skills install|uninstall|list # teach a coding agent this CLI
soulseek-rs completions install|uninstall # tab completion for bash/zsh/fish
```

A script usually starts with `whoami`, which answers "are these credentials
good and what am I offering" in one call.

With a daemon attached every command reports the daemon's world: `--username`
and `--password` go unread, and `whoami` names the daemon's account, server and
download folder. `--no-daemon` tests this machine's own login instead.
`shares list` reads the config file either way, while `shares status` and
`shares reindex` ask the session and report what peers can see.

Nineteen need no credentials: `config path|list|get|set`,
`shares list|add|remove`, `wish add|remove|list`, `portmap`,
`skills install|uninstall|list`, `completions install|uninstall` and
`daemon token|status|stop`. Most never touch the network; `daemon status` and
`daemon stop` talk to a daemon that is already logged in, so a machine with no
account of its own can still ask whether one is running. `shares status` and
`shares reindex` are not among them, because reporting what the network sees
means logging in and scanning the folders.

Six commands perform an action and print nothing, answering with their exit
code alone: `room say`, `message send`, `shares add`, `shares remove`,
`wish remove` and `daemon stop`. `room listen`, `message read` and `serve`
stream records until `--duration` seconds pass or, with `--follow`, until
interrupted. The rest print records and exit.

#### Record shapes

Text mode puts the identifying fields first and the remote path last, because
a path may contain anything except a tab (tabs and newlines in peer-supplied
text are replaced with spaces so a record is always exactly one line):

```
search           user   size    bitrate         path
browse           user   size    path
download / get   local-path
room list        users  room
room users       user
room listen      type   room    user            message
message read     timestamp      user            message
serve            status user    sent/size       path
daemon status    user   server  clients         uptime-secs
daemon token     token
user             user   status  average-speed   shared-files
whoami           user   server  shared-folders  shared-files
shares list      ok|missing     directory
shares status    folders        files
config get|set|list|path        key             value
portmap          ok|failed      backend         external
```

`room listen`'s `type` is `message`, `join` or `leave`; a join or leave leaves
the message field empty. `serve`'s `status` is `queued`, `uploading`,
`completed`, `cancelled` or `failed`. `user`'s `status` is `online`, `away` or
`offline`, and any field the server did not answer prints as `-`.

`--json` carries more than text does:

| Command            | Extra fields                                                                                                                    |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| `search`           | `duration`, `slots`, `speed`, `free_slot`                                                                                       |
| `browse`           | `directory`                                                                                                                     |
| `download` / `get` | `user`, remote `path`, `size`, beside the local `file`                                                                          |
| `whoami`           | `listening`, `listen_port`, `download_dir`, `privilege_seconds`                                                                 |
| `user`             | `privileged`, `shared_folders`                                                                                                  |
| `shares status`    | the `directories` array                                                                                                         |
| `serve`            | `bytes_sent`, `size`, `speed`, `reason` (queue place or failure)                                                                |
| `daemon status`    | `version`, `protocol`, `listening`, `listen_port`, `shared_folders`, `shared_files`, `download_dir`, `session_loss`              |
| `portmap`          | `port`                                                                                                                          |

`session_loss` is `displaced`, `disconnected`, or `null` while the session
holds. Fields the server never answered are `null` rather than absent, so a
missing reply cannot be misread as a zero.

`download --stdin` reads back either shape it emits: a JSON object per line,
or tab-separated text whose first field is the user and last field is the
path. That is why `search` and `browse` pipe straight into it.

#### Recipes

```bash
# Everything above 320kbps from peers with a free slot, best first
soulseek-rs search "aphex twin xtal" --min-bitrate 320 --free-slots

# One command, start to finish: search, pick the best, download it
soulseek-rs get "aphex twin xtal"

# Filter with jq, then fetch what survived
soulseek-rs search "rjd2" --json \
  | jq -c 'select(.size > 5000000 and .free_slot)' \
  | soulseek-rs download --stdin

# Grab a peer's whole FLAC folder
soulseek-rs browse someuser | grep -i '\.flac$' | soulseek-rs download --stdin

# Branch on the outcome
if soulseek-rs get "some rare track" --quiet; then
  echo "got it"
elif [ $? -eq 4 ]; then
  echo "nobody has it"
fi

# Nobody has it today: keep looking, and fetch it the day someone does
soulseek-rs wish add "some rare 12 inch"
soulseek-rs wish run --json | soulseek-rs download --stdin

# Only lossless, nothing from a bootleg folder, album-sized files only
soulseek-rs search "boards of canada" \
  --extension flac --exclude bootleg --min-size 20000000

# Log room chat as JSON until interrupted
soulseek-rs room listen lobby --follow --json >> lobby.ndjson

# Be a peer: share a folder and log every upload served
soulseek-rs shares add ~/Music
soulseek-rs serve --follow --json | tee -a uploads.ndjson

# Decide whether a peer is worth queueing to before committing
soulseek-rs user someuser --json | jq -e '.status != "offline"'
```

### Agent skills

[![an agent loading the soulseek-rs skill and downloading tracks](website/public/agent-demo.svg)](https://re-invention.nl/soulseek-rs/#agents)

`--help` lists flags but cannot say which JSON keys a record carries or what
exit 4 means. That lives in a skill file shipped inside the binary:

```bash
soulseek-rs skills install
```

It writes `SKILL.md` into every agent it finds in your home directory, today
Claude Code (`~/.claude/skills`) and opencode (`~/.config/opencode/skill`), and
prints one record per target:

```
installed   claude     /home/you/.claude/skills/soulseek-rs/SKILL.md
unchanged   opencode   /home/you/.config/opencode/skill/soulseek-rs/SKILL.md
```

Rerun it after upgrading the binary; an identical copy reports `unchanged`.
`skills list` shows what is there, and `skills uninstall` removes only the file
this binary wrote. For any other agent, name the directory yourself, or commit
the skill next to a project so everyone working on it gets the same brief:

```bash
soulseek-rs skills install --dir ~/.config/some-agent/skills
soulseek-rs skills install --dir .claude/skills   # commit it with the repo
```

### Shell completions

Tab completion comes from the same `clap` definition the binary parses with, so
it never drifts from `--help`:

```bash
soulseek-rs completions install
```

It covers whichever of bash, zsh and fish this machine uses, writing each script
where that shell already looks. `~/.config/fish/completions` is scanned
unprompted; bash and zsh also get one marked `source` line appended to
`~/.bashrc` or `~/.zshrc`, since nothing else puts a per-user directory on their
search path. Open a new shell and `soulseek-rs sh<TAB>` completes.

```
installed   zsh    /home/you/.local/share/zsh/site-functions/_soulseek-rs
installed   fish   /home/you/.config/fish/completions/soulseek-rs.fish
```

`completions uninstall` removes the scripts and takes back exactly the lines it
added, leaving the rest of your rc file alone. Pass `--shell bash` (repeatable)
to act on one shell rather than the ones detected.

### Sharing

`serve` is the mode that makes this client a source: it stays logged in, keeps
the listener and share index alive, answers searches and browse requests, and
prints one record per upload as it changes state (`queued`, `uploading`,
`completed`, `cancelled`, `failed`). It ends after `--duration` seconds (an
hour by default), or runs until interrupted with `--follow`.

```bash
soulseek-rs shares add ~/Music     # remembered in config.toml
soulseek-rs shares status          # what the network will see
soulseek-rs shares reindex         # after adding files on disk
soulseek-rs serve --follow         # add --upload-slots N to widen the queue
```

Uploads only appear inside a running `serve`, since every other command is
short-lived with nothing to serve, so there is no `uploads list`. Pausing,
resuming, and a queue that outlives one invocation are
[daemon mode](#daemon-mode).

With a daemon running `serve` starts no second server. It attaches and streams
the daemon's uploads, `--upload-slots` resizes the daemon's queue, and the local
"nothing to share" and "needs the listener" checks are skipped because the
daemon's shares and listener are the ones peers reach.

Downloads run under `--timeout` (300s) and `--max-concurrent-downloads` at a
time, so an unattended run always ends. An interrupted one resumes: bytes stream
into `<file>.part` and it is renamed only once whole, so re-running the same
`download` asks the peer to start at the offset already on disk. Partial files
are never offered to peers, and deleting the `.part` starts over.

### Configuration

Every setting can come from a flag, an environment variable, or the config
file, in that order of precedence. Flags work before or after the subcommand.

| Flag                           | Environment variable                | `config.toml` key            | Default                   |
| ------------------------------ | ----------------------------------- | ---------------------------- | ------------------------- |
| `--username`                   | `SOULSEEK_USERNAME`                 | `username`                   | required                  |
| `--password`                   | `SOULSEEK_PASSWORD`                 | never stored in the file     | required                  |
| `--password-cmd`               | `SOULSEEK_PASSWORD_CMD`             | `password_cmd`               | unset                     |
| `--server`                     | `SOULSEEK_SERVER`                   | `server`                     | `server.slsknet.org:2416` |
| `--download-dir`               | `SOULSEEK_DOWNLOAD_DIR`             | `download_dir`               | `<Downloads>/Soulseek`    |
| `--shared-dir` (repeatable)    | `SOULSEEK_SHARED_DIR`               | `shared_dir` / `shared_dirs` | the download dir          |
| `--listener-port`              | `SOULSEEK_LISTENER_PORT`            | `listener_port`              | `2234`                    |
| `--no-listener` / `--listener` | `SOULSEEK_NO_LISTENER`              | `disable_listener`           | listener on               |
| `--max-concurrent-downloads`   | `SOULSEEK_MAX_CONCURRENT_DOWNLOADS` | `max_concurrent_downloads`   | `20`                      |
| `--search-timeout`             | `SOULSEEK_SEARCH_TIMEOUT`           | `search_timeout`             | `10`                      |
| `--daemon ADDR`                | `SOULSEEK_DAEMON`                   | `daemon`                     | local socket if one is up |
| `--daemon-token`               | `SOULSEEK_DAEMON_TOKEN`             | `daemon_token`               | unset (TCP only)          |
| `--no-daemon`                  | —                                   | —                            | attach if one is running  |
| `--log-file`                   | `SOULSEEK_LOG_FILE`                 | not a config key             | stderr                    |

Boolean environment variables accept `1`/`0`/`true`/`false`/`yes`/`no`.
`--config <FILE>` (or `SOULSEEK_CONFIG`) reads a different file, `--no-config`
ignores it entirely, and `SOULSEEK_CONFIG_DIR` and `SOULSEEK_STATE_DIR` relocate
the config and state directories wholesale. The file lives at
`~/.config/soulseek-rs/config.toml` on macOS and Linux. Every variable above is
also read from a `.env` in the working directory, usually the tidiest way to
hand a container its credentials.

`config get` and `config set` cover the eleven settings the file holds
(`username`, `server`, `listener_port`, `disable_listener`, `download_dir`,
`shared_dirs`, `max_concurrent_downloads`, `search_timeout`, `password_cmd`,
`daemon`, `daemon_token`), and list them back at you when you name something
else. An empty string clears a key, and `shared_dirs` takes a comma-separated
list. Waits expressed in seconds (`--search-timeout`, `--timeout`,
`--duration`) are bounded to one day, so a mistyped flag is rejected rather
than hanging.

#### Credentials for unattended runs

Four ways to supply a password, in the order they are consulted:

1. `--password-stdin`: read the first line of stdin, like `docker login`.
2. `--password` / `SOULSEEK_PASSWORD`: convenient, but visible in `ps`.
3. The **OS keychain** (macOS Keychain, Windows Credential Manager, Linux
   Secret Service), under service `soulseek-rs` and your username. The TUI
   stores your password here after a successful login.
4. `password_cmd` / `--password-cmd`: a shell command whose stdout is the
   password, e.g. `pass show soulseek`. Best for headless boxes with no
   keychain.

```bash
# Nothing sensitive on the command line or in the environment
pass show soulseek | soulseek-rs --username alice --password-stdin get "some track"
```

Logging stays at errors only unless you ask for more: `-v` through `-vvvv`
raise it, and `LOG_LEVEL`/`RUST_LOG` are honoured when no `-v` is given.
`NO_COLOR` and a non-terminal stderr both disable colour.

### Chat and messages

```bash
soulseek-rs message send <username> "hello there"
soulseek-rs message read --duration 60    # or --follow until killed
soulseek-rs room list                     # public rooms with user counts
soulseek-rs room listen <room>            # stream messages, joins, leaves
soulseek-rs room say <room> "hello room"  # post one message and exit
```

In the TUI, `m` composes a private message and `i` opens the inbox with an
unread counter. `c` opens the chat-rooms popup: a `/`-filterable room list
busiest first, `Enter` to join, several rooms open at once as tabs
(`Tab`/`Shift-Tab` to switch, `x` to leave, `l` back to the list), and `↑`/`↓`
through the member list with `b` to browse someone or `m` to message them.
Unread counts appear on the tabs and on the `c chat (n)` shortcut.

### Connectivity

Browsing and downloading are peer-to-peer, so one side has to accept an incoming
connection. With the listener on (the default) the client opens its listen port
via **UPnP-IGD** and **NAT-PMP**, renews it while running, and removes it on
exit. Best-effort: a router with both disabled makes it a no-op, and a log line
says to forward the port yourself.

- The port is `--listener-port` (`SOULSEEK_LISTENER_PORT`, default `2234`).
  Forward that TCP port when auto-mapping cannot get it.
- `--no-listener` turns the listener and port mapping off; `--listener` forces
  it on when the config file disables it.
- When another process already holds the port, the usual case with several
  one-shot commands at once, the client takes one the OS picks, says so on
  stderr, and advertises that one. `whoami` reports what it really holds.

```bash
soulseek-rs portmap            # exits 0 when mapping works, 4 when it does not
soulseek-rs portmap --json     # {"ok":true,"backend":"upnp","external":"…"}
```

`portmap` opens the port, reports whether the router allowed it and your
external address, then removes the test mapping. The verdict is the exit code,
so `soulseek-rs portmap || notify-me` works as a health check.

Two peers both behind routers with no forwarded port cannot browse each other.
That is a Soulseek limitation, not a bug.

## Development

```bash
RUST_LOG=trace cargo run   # run with debug and trace output
cargo test
cargo clippy
cargo fmt
```

### End-to-end tests

Two suites run against [soulfind](https://github.com/soulfind-dev/soulfind), a
local Soulseek server: `soulseek-rs-lib/tests/e2e.rs` covers the protocol
library, and `soulseek-rs/tests/cli_e2e.rs` drives the binary the way a script
would. Both are **server-optional**, running when a server is available and
skipping otherwise, so `cargo test` stays green everywhere.

They locate a server in this order:

1. `SOULSEEK_TEST_SERVER=host:port`: connect to an already-running server, or
2. `SOULFIND_BIN=/path/to/soulfind` (or a `soulfind/bin/soulfind` checkout in a
   parent directory), which spawns soulfind on an ephemeral port with a
   throwaway database.

```bash
# Build soulfind once (see its BUILDING.md), then:
SOULFIND_BIN=/path/to/soulfind/bin/soulfind \
  cargo test --workspace -- --nocapture
```

> On macOS, soulfind's `sqlite3_config` call is rejected by the system SQLite
> and the server dies at startup with `SQLite error 7 (out of memory)`, which
> the suite reports as a skip. Run the tests with
> `DYLD_LIBRARY_PATH=/opt/homebrew/opt/sqlite/lib` so soulfind picks up
> Homebrew's SQLite instead; no patching needed.

Set `SOULSEEK_E2E_REQUIRED=1` to turn a missing server into a hard failure
instead of a skip. CI sets it, so the e2e suite runs against a freshly built
soulfind instead of skipping.

### Continuous integration

`.github/workflows/ci.yml` runs on every push and pull request:

- **Format & Clippy**: `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`.
- **Test**: `cargo test --verbose` on Linux, macOS, and Windows.
- **End-to-end**: builds soulfind from source (LDC + `dub build :server`),
  points `SOULFIND_BIN` at it, and runs both the library and CLI e2e suites
  with `SOULSEEK_E2E_REQUIRED=1`, so a missing server fails instead of skipping.

## License

MIT. See [LICENSE](./LICENSE).
