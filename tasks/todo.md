# Concurrency stress benchmark, and the ceilings it found

The client had never been driven at scale. The goal was a repeatable local
benchmark against soulfind, then to fix whatever it exposed until the client
behaves like a modern Rust P2P client on a fast machine.

## Targets

| Dimension                       | Target                       |
|---------------------------------|------------------------------|
| Concurrent peer connections     | 500+                         |
| Simultaneous transfers          | 50+                          |
| Aggregate throughput            | ≥100 MiB/s                   |
| Search fan-in                   | hundreds/sec, none dropped   |

## Plan

- [x] Build a load generator that mixes searches, downloads, uploads and browses
- [x] Make it convenient to run and give it a stored, ratcheting score
- [x] Find the first ceiling and fix it at the root
- [x] Keep going until the load stops finding failures
- [x] Verify no regression in the existing suite

## Findings

Every ceiling below was invisible under light load and only appeared in a
**mixed** run. Three of the four are silent-data-loss bugs, not slowness.

### 1. Peers capped at one per core

`ActorSystem::spawn` ran each actor's whole lifetime as one job on a fixed-size
`ThreadPool`. Actor loops block until stopped, so the pool acted as a semaphore:
with 16 cores, peer 16 onward never started, never connected and never errored.
Uploads simply stopped working at that point.

The pool had exactly one consumer, and every job it ever received was
non-terminating — it was pure downside. Deleted; actors get their own thread
(256 KiB stack, so a thousand peers costs address space, not memory).

### 2. TransferResponse raced its own bookkeeping

`handle_transfer_request` sent `UpdateDownloadTokens` to the client loop *and*
immediately replied `TransferResponse` to the peer. The response is what invites
the peer's file connection, and that connection is matched by token — so under
load the client loop lost the race, the file connection found no download, and
the transfer was dropped with the download stuck on `Queued` forever.

The response now goes out from the handler that records the token, after it.
This one alone took the mixed profile from 71/96 downloads at 7.9 MiB/s to
96/96 at 185 MiB/s.

### 3. Download tokens collided by construction

`token = md5(filename)[0..5]` — 20 bits, derived from the filename alone. The
download store removes *every* entry with a given token, so two downloads
sharing one destroy each other. At 512 concurrent downloads a birthday collision
showed up most runs; in normal use, fetching the same filename from two peers
collides *every* time. Tokens now come from a counter.

### 4. Port 0 treated as an address

When the server does not know where a user listens it answers `GetPeerAddress`
with port 0. The client cached that and dialled it, which fails with "Can't
assign requested address", and the upload waiting on it was dropped.

First attempt at this one skipped the rest of the handler and broke three
browse tests — the failing dial is exactly what makes an unreachable peer fall
back to the server-brokered path. The fix is narrower: do not cache port 0 and
do not serve against it, but leave the connect attempt alone.

### 5. Conservative defaults throttled a modern connection

Rebasing master in brought an upload slot queue defaulting to 2 slots, and the
CLI already capped downloads at 5. Both are the numbers you pick when bandwidth
is the scarce resource — but on Soulseek a transfer is paced by the *other*
peer, so concurrency is what fills a modern link, not per-transfer speed.
Measured with 64 waiting peers: 2 slots took 24.2s, 8 took 6.2s, 32 took 3.1s.

Upload slots 2 → 10, max concurrent downloads 5 → 20. The queue itself stays:
privileged users jumping it is a condition of being a tolerated third-party
client.

### 6. Every upload slept half a second after sending

`serve_file` lingered 500ms so the downloader could drain before the socket
closed. With a slot cap that is time no other peer can use — 64 uploads over 10
slots paid it about six times over. A half-close (`shutdown(Write)`) is the
correct mechanism: the FIN says "that is the whole file", and TCP still delivers
everything written before it. 64 uploads at 10 slots: 5.4s → 3.05s.

## Results

Default profile: 1024 peers, 128 searches, 512 downloads, 256 uploads,
128 browses, 4 MiB files.

| | Before | After |
|---|---|---|
| Concurrent peers | ~15 | 1024 |
| Downloads completed | stalls above ~64 peers | 512/512 |
| Uploads completed | 0 once peers ≥ cores | 256/256 |
| Aggregate throughput | 7.9 MiB/s | 400–850 MiB/s |
| Wall time | 60 s timeout | ~7 s |

Score **97.78/100**, stable across repeated runs (97.2–98.9). Every
functional dimension is at 100%; the remainder is throughput headroom against a
deliberately generous 500 MiB/s target.

All 569 tests pass after rebasing master in, including the soulfind-backed e2e
suites (verified against an unmodified worktree at HEAD after an earlier
regression).

## Not done

- **Brokered fallback for uploads.** A peer whose port the server does not know
  still cannot be served; downloads already fall back to server brokering,
  uploads do not. Out of scope here, and it needs its own e2e coverage.
- **Threads scale with peers**, not with cores — 1024 peers is 1024 actor
  threads. It holds up fine at this scale, but a reactor over the peer sockets
  is the answer if the target ever moves to five figures.
- **soulfind is now the limiting factor**, not the client: a thousand
  simultaneous logins starve it badly enough that the client's own login times
  out. The harness staggers logins to work around it. Pushing much past 1024
  peers needs the mock server written in Rust.
