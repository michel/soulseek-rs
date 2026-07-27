# The daemon control protocol

`soulseek-rs daemon` stays logged in and answers JSON-RPC over a socket, so
several clients can share one Soulseek session. The machine-readable contract
is [`openrpc.json`](openrpc.json) — an [OpenRPC](https://open-rpc.org)
document, the JSON-RPC counterpart to OpenAPI. Point a client generator at it,
or at a running daemon via `rpc.discover`, and you have a client in your
language.

This page covers what a schema cannot: framing, the handshake, and how pushed
events behave.

## Framing

Newline-delimited JSON-RPC 2.0. One JSON value per line, in both directions,
UTF-8, no length prefix. A line that does not parse gets a `-32700` reply and
the connection stays open.

## Transports and authentication

| Transport | Address | Authentication |
|---|---|---|
| Unix socket | `<state-dir>/daemon.sock`, mode 0600 | The socket's permissions. `token` may be omitted. |
| TCP | opt-in via `daemon --bind ADDR` | The token in `<state-dir>/daemon.token`, mode 0600. |

The local socket is the Docker and ssh-agent bargain: if the operating system
let you open it, you are the user who started the daemon, and there is nothing
further to prove. This is what lets a local client — a script or an agent —
work with no configuration at all.

TCP always requires the token, and a token that *is* offered is always
checked, on either transport, so a wrong one never passes. Failed attempts
sleep one second. The token is 256 bits from the OS, generated on first start
and reused across restarts so existing clients keep working;
`soulseek-rs daemon token` prints it.

There is no transport encryption. Binding non-loopback TCP means putting SSH
or a TLS-terminating proxy in front of it.

`<state-dir>` is `$SOULSEEK_STATE_DIR`, else the platform data directory
(`~/.local/share/soulseek-rs/state` on Linux and macOS).

### The handshake

`auth` must be the first request on a connection. Anything else is answered
`-32600` and the connection is dropped. Until it succeeds the connection is
told nothing and receives no events.

```json
--> {"jsonrpc":"2.0","id":1,"method":"auth","params":{"protocol":1}}
<-- {"jsonrpc":"2.0","id":1,"result":{"protocol":1,"daemon_version":"13.0.0","username":"you"}}
```

Compare `protocol` with your own and fail loudly on a mismatch rather than
guessing; it is bumped whenever a change would break a client written against
the old shape.

## Requests

Parameters are by name — a JSON object, never an array. Every method and its
parameter and result schemas are in `openrpc.json`; `rpc.discover` returns
that same document from the running daemon.

Requests may be pipelined and are answered out of order, so match replies by
`id`. A request without an `id` is a notification and is answered with
silence, including when it fails.

## Errors

```json
{"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"no results","data":{"exit":4}}}
```

Standard codes (`-32700` parse, `-32600` invalid request, `-32601` unknown
method, `-32602` bad parameters) mean what they always mean. Application
failures use `-32000` and carry `data.exit`: the CLI exit status the same
operation would have produced locally, so a wrapper can branch on it exactly
as it branches on the exit code.

| `exit` | Meaning |
|---|---|
| 2 | Usage or configuration |
| 3 | Could not reach the server, or the login was rejected |
| 4 | Succeeded but produced nothing |
| 5 | Timed out |
| 6 | A transfer started but did not complete |
| 7 | The server session ended mid-operation |

## Events

The daemon pushes notifications — no `id`, so never answer them:

```json
{"jsonrpc":"2.0","method":"event.room","params":{"event":"message","room":"lobby","username":"bob","message":"hi"}}
```

`event.room`, `event.message`, `event.upload`, `event.download_status`,
`event.browse`, `event.session_loss`. Payload schemas are in `openrpc.json`,
where they appear as entries marked `x-notification` — OpenRPC has no
first-class notion of a server-pushed message, so that flag is how they are
distinguished from methods you may call.

**Events are the only way to observe these things, and they are not
replayable.** The underlying library hands over its event buffer and empties
it, so the daemon drains once and copies to every attached connection. A
client that is not connected when something happens does not learn about it
later, with one exception: private messages are also kept, and
`message.history` returns the conversation including what arrived while
nobody was attached.

Every connection gets its own queue, bounded at 1024 events. **A client that
stops reading its socket is disconnected** once that queue fills — the daemon
will not let one stalled consumer cost the others their events. Read
continuously; do not treat the socket as request/response only.

A `download.start` you issued reports progress through `event.download_status`
matched on `username` + `filename`.

## Behaviour worth knowing

**Files land on the daemon's filesystem.** `download_directory` is the
daemon's configuration; a remote client cannot see that filesystem and does
not choose. `daemon.status` reports the directory in use. There is no method
to fetch the bytes over the control socket.

**Several clients, no locking.** Last write wins. Two clients cancelling the
same transfer is two hands on one keyboard, not an error.

**The daemon owns durable state.** Transfers, joined rooms, and the
conversation survive a restart: unfinished transfers are re-queued and rooms
rejoined on startup.

**Searching is two steps.** `search.start` puts the query on the wire and
returns immediately; peers answer over their own connections for as long as
you care to wait. Decide the window yourself, then read `search.results`. This
is deliberate — a client's search window must not be time the daemon spends
not answering everybody else.

## A session by hand

```console
$ nc -U ~/.local/share/soulseek-rs/state/daemon.sock
{"jsonrpc":"2.0","id":1,"method":"auth","params":{"protocol":1}}
{"jsonrpc":"2.0","id":1,"result":{"protocol":1,"daemon_version":"13.0.0","username":"you"}}
{"jsonrpc":"2.0","id":2,"method":"search.start","params":{"query":"aphex twin"}}
{"jsonrpc":"2.0","id":2,"result":{"ok":true}}
{"jsonrpc":"2.0","id":3,"method":"search.results","params":{"query":"aphex twin"}}
{"jsonrpc":"2.0","id":3,"result":{"results":[…]}}
```

That the whole protocol is reachable from `nc` and a text editor is the reason
it is shaped this way.
